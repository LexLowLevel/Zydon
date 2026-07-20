// Task control block (TCB).
//
// Hot fields occupy the first cache line for the poll loop.
// WaitNode and CpuContext are inline to avoid heap allocations.

use core::future::Future;
use core::pin::Pin;
use core::sync::atomic::{AtomicI32, AtomicU32, AtomicU64, Ordering};
use core::task::{Context, Waker};
use core::cell::UnsafeCell;

use crate::task::task_id::TaskId;
use crate::task::task_state::{AtomicTaskState, TaskState};
use crate::primitives::wait_queue::WaitNode;

/// Kernel stack size per task (bytes).
pub const KERNEL_STACK_SIZE: usize = 8192;

/// Saved CPU context (arch-specific, see arch/contextswi.s).
#[repr(C, align(16))]
pub struct CpuContext {
    regs: [u8; 512], // opaque register save area
}

impl CpuContext {
    pub const fn new() -> Self {
        Self { regs: [0; 512] }
    }
}

/// Core affinity mask (bit N = may run on core N).
pub type AffinityMask = u64;

/// Task control block. Hot fields first for cache locality.
#[repr(C)]
pub struct TaskControlBlock {
    // ── hot cache line ──
    pub id: TaskId,
    pub state: AtomicTaskState,
    pub effective_priority: AtomicI32, // base + PI boost
    pub base_priority: AtomicI32,
    pub quantum_remaining: AtomicI32,
    pub affinity: AtomicU64,

    // ── cold fields ──
    /// Async body. SAFETY: executor has exclusive access during poll.
    future: UnsafeCell<Pin<Box<dyn Future<Output = ()> + Send>>>,
    waker: UnsafeCell<Option<Waker>>,
    pub wait_node: UnsafeCell<WaitNode>,
    pub saved_context: CpuContext,
    pub kernel_stack: *mut u8,
    pub current_core: AtomicU32, // u32::MAX = not running
    pub decay_counter: AtomicU32,
    pub user_tid: u64,
    /// Intrusive run queue link. Access only under owning LevelQueue's lock.
    pub run_queue_next: UnsafeCell<*const TaskControlBlock>,
    /// Waker refcount. Ensures TCB outlives any Waker pointing to it.
    waker_refcount: AtomicU32,
}

// SAFETY: future is Send; mutable access via UnsafeCell enforced by
// state machine; WaitNode accessed only by owner or under WaitQueue lock.
unsafe impl Send for TaskControlBlock {}
unsafe impl Sync for TaskControlBlock {}

impl TaskControlBlock {
    /// New TCB. Created state, all-core affinity.
    pub fn new<F>(id: TaskId, future: F) -> Self
    where
        F: Future<Output = ()> + Send + 'static,
    {
        Self {
            id,
            state: AtomicTaskState::new(TaskState::Created),
            effective_priority: AtomicI32::new(0),
            base_priority: AtomicI32::new(0),
            quantum_remaining: AtomicI32::new(0),
            affinity: AtomicU64::new(u64::MAX), // all cores
            future: UnsafeCell::new(Box::pin(future)),
            waker: UnsafeCell::new(None),
            wait_node: UnsafeCell::new(WaitNode::new()),
            saved_context: CpuContext::new(),
            kernel_stack: core::ptr::null_mut(),
            current_core: AtomicU32::new(u32::MAX),
            decay_counter: AtomicU32::new(0),
            user_tid: 0,
            run_queue_next: UnsafeCell::new(core::ptr::null()),
            waker_refcount: AtomicU32::new(0),
        }
    }

    /// Mutable ref to future for polling.
    /// SAFETY: caller must be owning-core executor, task must be RUNNING.
    pub unsafe fn future_mut(&self) -> Pin<&mut (dyn Future<Output = ()> + Send)> {
        (*self.future.get()).as_mut()
    }

    /// Set waker. SAFETY: only from executor before poll.
    pub unsafe fn set_waker(&self, waker: Waker) {
        *self.waker.get() = Some(waker);
    }

    /// Mutable ref to embedded wait node.
    /// SAFETY: caller must ensure node isn't concurrently linked.
    pub unsafe fn wait_node_mut(&self) -> &mut WaitNode {
        &mut *self.wait_node.get()
    }

    /// Transition to READY and enqueue (called by waker).
    /// No-op if already READY or RUNNING.
    pub fn wake(&self) {
        loop {
            let current = self.state.get();
            match current {
                TaskState::Blocked => {
                    if self.state.transition(TaskState::Blocked, TaskState::Ready).is_ok() {
                        // Executor picks this up on next poll.
                        return;
                    }
                    // CAS failed; retry.
                }
                TaskState::Created => {
                    // Created→Ready (never scheduled).
                    if self.state.transition(TaskState::Created, TaskState::Ready).is_ok() {
                        return;
                    }
                }
                _ => {
                    // Already Ready/Running/Dying/Dead. No-op.
                    return;
                }
            }
        }
    }

    /// Set base priority (updates effective if no PI boost active).
    pub fn set_priority(&self, priority: i32) {
        self.base_priority.store(priority, Ordering::Relaxed);
        let effective = self.effective_priority.load(Ordering::Relaxed);
        if effective <= priority || effective == self.base_priority.load(Ordering::Relaxed) {
            self.effective_priority.store(priority, Ordering::Relaxed);
        }
    }

    /// Apply priority inheritance boost.
    pub fn boost_priority(&self, boost_to: i32) {
        loop {
            let current = self.effective_priority.load(Ordering::Relaxed);
            if current >= boost_to {
                break;
            }
            match self.effective_priority.compare_exchange_weak(
                current,
                boost_to,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(_) => continue,
            }
        }
    }

    /// Restore effective priority to base (undo PI boost).
    pub fn unboost_priority(&self) {
        let base = self.base_priority.load(Ordering::Relaxed);
        self.effective_priority.store(base, Ordering::Relaxed);
    }

    /// Reset quantum for a new scheduling period.
    pub fn reset_quantum(&self, quantum: i32) {
        self.quantum_remaining.store(quantum, Ordering::Relaxed);
    }

    /// Consume one tick. Returns remaining ticks.
    #[inline]
    pub fn consume_tick(&self) -> i32 {
        let prev = self.quantum_remaining.fetch_sub(1, Ordering::Relaxed);
        prev - 1
    }

    /// Eligible to run on `core_id`?
    #[inline]
    pub fn can_run_on(&self, core_id: u32) -> bool {
        let mask = self.affinity.load(Ordering::Relaxed);
        (mask & (1u64 << core_id)) != 0
    }
}