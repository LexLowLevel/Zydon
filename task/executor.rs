// Per-core async executor.
//
// Polls tasks from a local Chase-Lev deque, steals from peers when idle,
// and enters a low-power C-state when quiescent.

use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use core::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};

use crate::task::task_control_block::TaskControlBlock;
use crate::task::task_id::TaskId;
use crate::task::task_state::TaskState;
use crate::task::run_queue::RunQueue;
use crate::task::stealer::{self, Worker, Stealer};

pub struct Executor {
    core_id: u32,
    /// LIFO end of the local deque.
    worker: Worker<*const TaskControlBlock>,
    /// Remote stealers (indexed by core_id).
    stealers: Vec<Stealer<*const TaskControlBlock>>,
    /// Priority-aware run queue from interrupt handlers.
    run_queue: RunQueue,
    default_quantum: i32,
    should_halt: AtomicBool,
    // stats
    polls_executed: u64,
    tasks_completed: u64,
    steals_attempted: u64,
    steals_succeeded: u64,
    idle_cycles: u64,
}

impl Executor {
    pub fn new(
        core_id: u32,
        worker: Worker<*const TaskControlBlock>,
        stealers: Vec<Stealer<*const TaskControlBlock>>,
        default_quantum: i32,
    ) -> Self {
        Self {
            core_id,
            worker,
            stealers,
            run_queue: RunQueue::new(),
            default_quantum,
            should_halt: AtomicBool::new(false),
            polls_executed: 0,
            tasks_completed: 0,
            steals_attempted: 0,
            steals_succeeded: 0,
            idle_cycles: 0,
        }
    }

    /// Enqueue a task (called by wakers).
    pub fn enqueue_task(&self, task: &TaskControlBlock) {
        self.run_queue.enqueue(task);
    }

    /// Main scheduling loop. Runs until `should_halt`.
    pub fn run(&mut self) -> ! {
        loop {
            if self.should_halt.load(Ordering::Acquire) {
                self.shutdown();
            }

            self.drain_run_queue();

            if let Some(task_ptr) = self.worker.pop() {
                let task = unsafe { &*task_ptr };
                self.poll_task(task);
                continue;
            }

            if self.try_steal() {
                continue;
            }

            self.run_pending_work();
            self.idle_enter();
        }
    }

    fn drain_run_queue(&mut self) {
        while let Some(task_ptr) = self.run_queue.dequeue() {
            self.worker.push(task_ptr);
        }
    }

    /// Poll one task, handling state transitions and quantum expiry.
    fn poll_task(&mut self, task: &TaskControlBlock) {
        if task.state.transition(TaskState::Ready, TaskState::Running).is_err() {
            // Concurrent wake moved it; skip.
            return;
        }

        task.current_core.store(self.core_id, Ordering::Relaxed);
        task.reset_quantum(self.default_quantum);
        let waker = self.make_waker(task);

        loop {
            // SAFETY: RUNNING state grants exclusive future access.
            let future = unsafe { task.future_mut() };
            let cx_ref = &mut Context::from_waker(&waker);

            match future.as_mut().poll(cx_ref) {
                Poll::Ready(()) => {
                    let _ = task.state.transition(TaskState::Running, TaskState::Dying);
                    debug_assert!(task.state.get() == TaskState::Dying);
                    unsafe { task.state.set(TaskState::Dead); }
                    task.current_core.store(u32::MAX, Ordering::Relaxed);
                    self.tasks_completed += 1;
                    self.polls_executed += 1;
                    return;
                }
                Poll::Pending => {
                    self.polls_executed += 1;
                    let remaining = task.consume_tick();
                    if remaining <= 0 {
                        let state = task.state.get();
                        if state == TaskState::Running {
                            let _ = task.state.transition(TaskState::Running, TaskState::Ready);
                            task.current_core.store(u32::MAX, Ordering::Relaxed);
                            self.worker.push(task as *const _);
                        } else {
                            task.current_core.store(u32::MAX, Ordering::Relaxed);
                        }

                        let decay = task.decay_counter.fetch_add(1, Ordering::Relaxed) + 1;
                        if decay % 4 == 0 {
                            let eff = task.effective_priority.load(Ordering::Relaxed);
                            let base = task.base_priority.load(Ordering::Relaxed);
                            if eff > base - 8 {
                                task.effective_priority.fetch_sub(1, Ordering::Relaxed);
                            }
                        }
                        return;
                    }
                }
            }
        }
    }

    /// Steal work from a random victim.
    fn try_steal(&mut self) -> bool {
        if self.stealers.is_empty() {
            return false;
        }

        self.steals_attempted += 1;
        let victim = self.pick_victim();

        // Steal up to half the victim's deque.
        let mut batch: [*const TaskControlBlock; 64] = [core::ptr::null(); 64];
        let stolen = self.stealers[victim].steal_batch(&mut batch);

        if stolen == 0 {
            return false;
        }

        self.steals_succeeded += 1;

        for i in 0..stolen {
            if !batch[i].is_null() {
                self.worker.push(batch[i]);
            }
        }

        true
    }

    fn pick_victim(&self) -> usize {
        // xorshift32 PRNG from poll count.
        let mut seed = (self.polls_executed as u32).wrapping_mul(1664525).wrapping_add(1013904223);
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        (seed as usize) % self.stealers.len()
    }

    /// Deferred work (softirq, timers).
    /// TODO: check softirq bitmap and timer wheel.
    fn run_pending_work(&mut self) {
    }

    /// Enter deepest valid C-state until next interrupt.
    fn idle_enter(&mut self) {
        self.idle_cycles += 1;

        #[cfg(target_arch = "x86_64")]
        {
            // SAFETY: ring-0 HLT, resumes on interrupt.
            unsafe { core::arch::asm!("sti; hlt"); }
        }

        #[cfg(target_arch = "aarch64")]
        {
            // SAFETY: EL1 WFI.
            unsafe { core::arch::asm!("wfi"); }
        }

        #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
        {
            core::hint::spin_loop();
        }
    }

    /// Halt this core (no return).
    fn shutdown(&self) -> ! {
        // TODO: migrate tasks, flush caches, ack halt IPI.
        loop {
            #[cfg(target_arch = "x86_64")]
            unsafe { core::arch::asm!("cli; hlt"); }
            #[cfg(target_arch = "aarch64")]
            unsafe { core::arch::asm!("wfi"); }
            #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
            core::hint::spin_loop();
        }
    }

    /// Create a Waker that transitions BLOCKED→READY on wake.
    fn make_waker(&self, task: &TaskControlBlock) -> Waker {
        task.waker_refcount.fetch_add(1, Ordering::Relaxed);
        let data = task as *const TaskControlBlock as *const ();
        unsafe { Waker::from_raw(RawWaker::new(data, &WAKER_VTABLE)) }
    }
}

// Waker vtable. TCB's waker_refcount ensures TCB outlives all Wakers.
const WAKER_VTABLE: RawWakerVTable = RawWakerVTable::new(
    waker_clone,
    waker_wake,
    waker_wake_by_ref,
    waker_drop,
);

unsafe fn waker_clone(data: *const ()) -> RawWaker {
    let task = &*(data as *const TaskControlBlock);
    task.waker_refcount.fetch_add(1, Ordering::Relaxed);
    RawWaker::new(data, &WAKER_VTABLE)
}

unsafe fn waker_wake(data: *const ()) {
    let task = &*(data as *const TaskControlBlock);
    task.wake();
    task.waker_refcount.fetch_sub(1, Ordering::Release);
}

unsafe fn waker_wake_by_ref(data: *const ()) {
    let task = &*(data as *const TaskControlBlock);
    task.wake();
}

unsafe fn waker_drop(data: *const ()) {
    let task = &*(data as *const TaskControlBlock);
    task.waker_refcount.fetch_sub(1, Ordering::Release);
}

/// Bootstrap an executor on the current core.
pub fn bootstrap_executor(
    core_id: u32,
    worker: Worker<*const TaskControlBlock>,
    stealers: Vec<Stealer<*const TaskControlBlock>>,
) -> ! {
    let mut executor = Executor::new(core_id, worker, stealers, 10);
    executor.run()
}
