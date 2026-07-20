// Per-core multilevel feedback run queue.
//
// 32 priority levels backed by lock-free MPSC queues.
// Bitmap + leading_zeros() for O(1) dequeue of highest-priority task.
// Priority decays on full-quantum burn, recovers on I/O yield.

use core::sync::atomic::{AtomicU32, Ordering};

use crate::primitives::spinlock::Spinlock;
use crate::task::task_control_block::TaskControlBlock;

/// Number of priority levels (higher index = higher priority).
const NUM_LEVELS: usize = 32;

type OccupancyBitmap = u32;

/// Per-core run queue. 32 levels, each an intrusive MPSC behind a spinlock.
pub struct RunQueue {
    /// Intrusive MPSC queues (minimal contention: per-core).
    levels: [Spinlock<LevelQueue>; NUM_LEVELS],
    occupancy: AtomicU32, // bit N set if level N non-empty
    len: AtomicU32,
}

struct LevelQueue {
    head: *const TaskControlBlock,
    tail: *const TaskControlBlock,
    count: u32,
}

// SAFETY: LevelQueue only accessed through RunQueue's Spinlock.
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

    #[inline]
    fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Push to tail. SAFETY: task valid until popped.
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

    /// Pop from head.
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
        Self {
            levels: core::array::from_fn(|_| Spinlock::new(LevelQueue::new())),
            occupancy: AtomicU32::new(0),
            len: AtomicU32::new(0),
        }
    }

    /// Enqueue at effective priority level.
    pub fn enqueue(&self, task: &TaskControlBlock) {
        let priority = task.effective_priority.load(Ordering::Relaxed);
        let level = Self::priority_to_level(priority);

        // SAFETY: task is ref-counted, outlives queue residence.
        unsafe {
            self.levels[level].lock().push(task as *const _);
        }

        self.occupancy.fetch_or(1u32 << level, Ordering::Release);
        self.len.fetch_add(1, Ordering::Release);
    }

    /// Dequeue highest-priority task.
    pub fn dequeue(&self) -> Option<*const TaskControlBlock> {
        loop {
            let bitmap = self.occupancy.load(Ordering::Acquire);
            if bitmap == 0 {
                return None;
            }

            let level = 31 - bitmap.leading_zeros() as usize;

            let task = unsafe { self.levels[level].lock().pop() };

            if let Some(t) = task {
                if self.levels[level].lock().is_empty() {
                    self.occupancy.fetch_and(!(1u32 << level), Ordering::Release);
                }
                self.len.fetch_sub(1, Ordering::Release);
                return Some(t);
            }
        }
    }

    /// Total tasks enqueued.
    pub fn len(&self) -> u32 {
        self.len.load(Ordering::Relaxed)
    }

    /// True if empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Signed priority → level index [0, 31]. 0 maps to 15.
    #[inline]
    fn priority_to_level(priority: i32) -> usize {
        let clamped = priority.max(-16).min(15);
        (clamped + 16) as usize
    }
}