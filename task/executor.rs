// Per-core async executor. Each core runs one of these.
// Pulls tasks from a local deque, steals from other cores when idle,
// and enters a low-power state when there's nothing to do.

use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use core::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};

use crate::task::task_control_block::TaskControlBlock;
use crate::task::task_id::TaskId;
use crate::task::task_state::TaskState;
use crate::task::run_queue::RunQueue;
use crate::task::stealer::{self, Worker, Stealer};

pub struct Executor {
    core_id: u32,
    /// LIFO end of the work-stealing deque.
    worker: Worker<*const TaskControlBlock>,
    /// Stealers from other cores (indexed by core_id).
    stealers: Vec<Stealer<*const TaskControlBlock>>,
    /// Incoming tasks from interrupt handlers on this core.
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

    /// Push a task into this core's run queue (called by wakers).
    pub fn enqueue_task(&self, task: &TaskControlBlock) {
        self.run_queue.enqueue(task);
    }

    /// Main loop. Runs forever until `should_halt` is set.
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

    /// Poll a single task. Handles state transitions and quantum expiry.
    fn poll_task(&mut self, task: &TaskControlBlock) {
        if task.state.transition(TaskState::Ready, TaskState::Running).is_err() {
            // Already moved by a concurrent wake, skip it.
            return;
        }

        task.current_core.store(self.core_id, Ordering::Relaxed);
        task.reset_quantum(self.default_quantum);
        let waker = self.make_waker(task);

        loop {
            // SAFETY: task is RUNNING, so we have exclusive access to its future.
            let future = unsafe { task.future_mut() };
            let cx_ref = &mut Context::from_waker(&waker);

            match future.as_mut().poll(cx_ref) {
                Poll::Ready(()) => {
                    // Task finished.
                    task.state.transition(TaskState::Running, TaskState::Dying)
                        .expect("invalid transition Running->Dying");
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
                        // Quantum expired. If the task didn't park itself,
                        // force it back to READY and re-enqueue.
                        let state = task.state.get();
                        if state == TaskState::Running {
                            task.state.transition(TaskState::Running, TaskState::Ready)
                                .expect("invalid transition");
                            task.current_core.store(u32::MAX, Ordering::Relaxed);
                            self.worker.push(task as *const _);
                        } else {
                            // Already parked on a wait queue, will be re-enqueued on wake.
                            task.current_core.store(u32::MAX, Ordering::Relaxed);
                        }

                        // Priority decay for burning through a full quantum.
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
                    // Still have quantum left, poll again.
                }
            }
        }
    }

    /// Try to steal work from a random victim core.
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
        // xorshift32 PRNG, seeded from poll count.
        let mut seed = (self.polls_executed as u32).wrapping_mul(1664525).wrapping_add(1013904223);
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        (seed as usize) % self.stealers.len()
    }

    /// Run deferred work (softirq, timers, etc.).
    /// TODO: check softirq bitmap and timer wheel here.
    fn run_pending_work(&mut self) {
    }

    /// Enter the deepest valid C-state until the next interrupt.
    fn idle_enter(&mut self) {
        self.idle_cycles += 1;

        #[cfg(target_arch = "x86_64")]
        {
            // SAFETY: HLT in ring 0, resumes on next interrupt.
            unsafe { core::arch::asm!("sti; hlt"); }
        }

        #[cfg(target_arch = "aarch64")]
        {
            // SAFETY: WFI at EL1.
            unsafe { core::arch::asm!("wfi"); }
        }

        #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
        {
            core::hint::spin_loop();
        }
    }

    /// Halt this core. No return.
    fn shutdown(&self) -> ! {
        // TODO: migrate tasks to other cores, flush caches, ack the halt IPI.
        loop {
            #[cfg(target_arch = "x86_64")]
            unsafe { core::arch::asm!("cli; hlt"); }
            #[cfg(target_arch = "aarch64")]
            unsafe { core::arch::asm!("wfi"); }
            #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
            core::hint::spin_loop();
        }
    }

    /// Create a Waker for a task. The waker transitions the task
    /// from BLOCKED to READY and enqueues it on wake.
    fn make_waker(&self, task: &TaskControlBlock) -> Waker {
        let data = task as *const TaskControlBlock as *const ();
        unsafe { Waker::from_raw(RawWaker::new(data, &WAKER_VTABLE)) }
    }
}

// Waker vtable. Stores a raw pointer to the TCB.
// On wake(), the task goes BLOCKED -> READY and gets enqueued.
const WAKER_VTABLE: RawWakerVTable = RawWakerVTable::new(
    waker_clone,
    waker_wake,
    waker_wake_by_ref,
    waker_drop,
);

unsafe fn waker_clone(data: *const ()) -> RawWaker {
    RawWaker::new(data, &WAKER_VTABLE)
}

unsafe fn waker_wake(data: *const ()) {
    let task = &*(data as *const TaskControlBlock);
    task.wake();
}

unsafe fn waker_wake_by_ref(data: *const ()) {
    let task = &*(data as *const TaskControlBlock);
    task.wake();
}

unsafe fn waker_drop(_data: *const ()) {
    // no-op, the TCB isn't owned by the waker
}

/// Bootstrap an executor on the current core and run it forever.
pub fn bootstrap_executor(
    core_id: u32,
    worker: Worker<*const TaskControlBlock>,
    stealers: Vec<Stealer<*const TaskControlBlock>>,
) -> ! {
    let mut executor = Executor::new(core_id, worker, stealers, 10);
    executor.run()
}
