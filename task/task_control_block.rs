// Task control block (TCB).
//
// Hot fields are packed into the first cache line for the poll loop.
// WaitNode and CpuContext are embedded to avoid heap allocations on
// park/context-switch.

use core::future::Future;
use core::pin::Pin;
use core::sync::atomic::{AtomicI32, AtomicU32, AtomicU64, Ordering};
use core::task::{Context, Waker};
use core::cell::UnsafeCell;

use crate::task::task_id::TaskId;
use crate::task::task_state::{AtomicTaskState, TaskState};
use crate::primitives::wait_queue::WaitNode;

/// Size of the kernel stack per task, in bytes.
pub const KERNEL_STACK_SIZE: usize = 8192;

/// Architecture-specific saved CPU context (registers, FPU state, etc.).
/// Actual layout is defined in arch/contextswi.s.
#[repr(C, align(16))]
pub struct CpuContext {
    regs: [u8; 512], // opaque register save area
}

impl CpuContext {
    pub const fn new() -> Self {
        Self { regs: [0; 512] }
    }
}

/// Core affinity mask. Bit N set means the task may run on core N.
pub type AffinityMask = u64;

/// The task control block. Hot fields first for cache locality.
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
    /// The async body. SAFETY: executor has exclusive access during poll.
    future: UnsafeCell<Pin<Box<dyn Future<Output = ()> + Send>>>,
    waker: UnsafeCell<Option<Waker>>,
    pub wait_node: UnsafeCell<WaitNode>,
    pub saved_context: CpuContext,
    pub kernel_stack: *mut u8,
    pub current_core: AtomicU32, // u32::MAX if not running
    pub decay_counter: AtomicU32,
    pub user_tid: u64,
    /// Intrusive linked-list pointer for the run queue. Access only under the
    /// owning LevelQueue's spinlock.
    pub run_queue_next: UnsafeCell<*const TaskControlBlock>,
}

// SAFETY: future is Send, mutable access is through UnsafeCell with
// state machine enforcement, WaitNode is only accessed by owner or
// under WaitQueue's spinlock.
unsafe impl Send for TaskControlBlock {}
unsafe impl Sync for TaskControlBlock {}

impl TaskControlBlock {
    /// Create a new TCB. Starts in Created state, all-core affinity.
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
        }
    }

    /// Get mutable ref to the future for polling.
    /// SAFETY: caller must be the executor on the owning core, task must be RUNNING.
    pub unsafe fn future_mut(&self) -> Pin<&mut (dyn Future<Output = ()> + Send)> {
        (*self.future.get()).as_mut()
    }

    /// Set the waker. SAFETY: only call from executor before poll.
    pub unsafe fn set_waker(&self, waker: Waker) {
        *self.waker.get() = Some(waker);
    }

    /// Get mutable ref to the embedded wait node.
    /// SAFETY: caller must ensure the node isn't concurrently linked.
    pub unsafe fn wait_node_mut(&self) -> &mut WaitNode {
        &mut *self.wait_node.get()
    }

    /// Transition to READY and enqueue. Called by the waker.
    /// No-op if already READY or RUNNING (spurious wake is safe).
    pub fn wake(&self) {
        loop {
            let current = self.state.get();
            match current {
                TaskState::Blocked => {
                    if self.state.transition(TaskState::Blocked, TaskState::Ready).is_ok() {
                        // Executor will pick this up on its next poll.
                        return;
                    }
                    // CAS failed; retry.
                }
                TaskState::Created => {
                    // Created but never run, transition to Ready.
                    if self.state.transition(TaskState::Created, TaskState::Ready).is_ok() {
                        return;
                    }
                }
                _ => {
                    // Already Ready, Running, Dying, or Dead. No-op.
                    return;
                }
            }
        }
    }

    /// Set base priority. Also updates effective if no PI boost is active.
    pub fn set_priority(&self, priority: i32) {
        self.base_priority.store(priority, Ordering::Relaxed);
        let effective = self.effective_priority.load(Ordering::Relaxed);
        if effective <= priority || effective == self.base_priority.load(Ordering::Relaxed) {
            self.effective_priority.store(priority, Ordering::Relaxed);
        }
    }

    /// Apply a PI boost.
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

    /// Reset the quantum for a new scheduling period.
    pub fn reset_quantum(&self, quantum: i32) {
        self.quantum_remaining.store(quantum, Ordering::Relaxed);
    }

    /// Consume one tick of the quantum. Returns the remaining ticks.
    pub fn consume_tick(&self) -> i32 {
        let prev = self.quantum_remaining.fetch_sub(1, Ordering::Relaxed);
        prev - 1
    }

    /// Returns true if the task is eligible to run on `core_id`.
    pub fn can_run_on(&self, core_id: u32) -> bool {
        let mask = self.affinity.load(Ordering::Relaxed);
        (mask & (1u64 << core_id)) != 0
    }
}