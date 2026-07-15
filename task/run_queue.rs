// Per-core multilevel feedback run queue.
//
// 32 priority levels, each backed by a lock-free MPSC queue.
// Dequeue uses leading_zeros() on a bitmap to find the highest
// non-empty level in O(1).
//
// Priority decays when a task burns through its full quantum without
// blocking, and recovers when it yields to I/O. This favors I/O-bound tasks.

use core::sync::atomic::{AtomicU32, Ordering};

use crate::primitives::spinlock::Spinlock;
use crate::task::task_control_block::TaskControlBlock;

/// Number of priority levels. Higher index = higher priority.
const NUM_LEVELS: usize = 32;

type OccupancyBitmap = u32;

/// Per-core run queue with 32 priority levels.
pub struct RunQueue {
    /// Each level is an intrusive MPSC queue behind a spinlock.
    /// Contention is minimal since it's per-core.
    levels: [Spinlock<LevelQueue>; NUM_LEVELS],
    occupancy: AtomicU32, // bit N set if level N is non-empty
    len: AtomicU32,
}

struct LevelQueue {
    head: *const TaskControlBlock,
    tail: *const TaskControlBlock,
    count: u32,
}

// SAFETY: LevelQueue is only accessed through the Spinlock in RunQueue.
unsafe impl Send for LevelQueue {}
unsafe impl Sync for LevelQueue {}

impl LevelQueue {
    const fn new() -> Self {
        Self {
            head: core::ptr::null(),
            tail: core::ptr::null(),
            count: 0,
        }
    }

    fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Push a task to the tail. SAFETY: task must remain valid until popped.
    unsafe fn push(&mut self, task: *const TaskControlBlock) {
        *(*task).run_queue_next.get() = core::ptr::null();
        if self.tail.is_null() {
            debug_assert!(self.head.is_null());
            self.head = task;
            self.tail = task;
        } else {
            *(*self.tail).run_queue_next.get() = task;
            self.tail = task;
        }
        self.count += 1;
    }

    /// Pop a task from the head.
    unsafe fn pop(&mut self) -> Option<*const TaskControlBlock> {
        if self.head.is_null() {
            return None;
        }
        let task = self.head;
        let next = *(*task).run_queue_next.get();
        self.head = next;
        if next.is_null() {
            self.tail = core::ptr::null();
        }
        *(*task).run_queue_next.get() = core::ptr::null();
        self.count -= 1;
        Some(task)
    }
}

impl RunQueue {
    pub fn new() -> Self {
        macro_rules! init_levels {
            ($($i:expr),*) => {
                [$(Spinlock::new(LevelQueue::new()),)*]
            };
        }
        Self {
            levels: init_levels!(
                0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,
                16,17,18,19,20,21,22,23,24,25,26,27,28,29,30,31
            ),
            occupancy: AtomicU32::new(0),
            len: AtomicU32::new(0),
        }
    }

    /// Enqueue a task at its effective priority level.
    pub fn enqueue(&self, task: &TaskControlBlock) {
        let priority = task.effective_priority.load(Ordering::Relaxed);
        let level = Self::priority_to_level(priority);

        // SAFETY: task is ref-counted and outlives its time in the queue.
        unsafe {
            self.levels[level].lock().push(task as *const _);
        }

        self.occupancy.fetch_or(1u32 << level, Ordering::Release);
        self.len.fetch_add(1, Ordering::Release);
    }

    /// Dequeue the highest-priority task.
    pub fn dequeue(&self) -> Option<*const TaskControlBlock> {
        let bitmap = self.occupancy.load(Ordering::Acquire);
        if bitmap == 0 {
            return None;
        }

        // Find highest set bit.
        let level = 31 - bitmap.leading_zeros() as usize;

        let task = unsafe { self.levels[level].lock().pop() };

        if let Some(t) = task {
            // Check if the level is now empty.
            if self.levels[level].lock().is_empty() {
                self.occupancy.fetch_and(!(1u32 << level), Ordering::Release);
            }
            self.len.fetch_sub(1, Ordering::Release);
            Some(t)
        } else {
            // Race: level was drained between bitmap read and lock. Retry.
            self.dequeue()
        }
    }

    /// Returns the total number of tasks in the queue.
    pub fn len(&self) -> u32 {
        self.len.load(Ordering::Relaxed)
    }

    /// Returns true if the queue is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Convert signed priority to level index [0, 31].
    /// Priority 0 maps to level 15 (neutral).
    fn priority_to_level(priority: i32) -> usize {
        let clamped = priority.max(-16).min(15);
        (clamped + 16) as usize
    }
}