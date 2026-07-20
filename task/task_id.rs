// Task identifier.
//
// 64-bit: upper 32 = generation (ABA prevention), lower 32 = slot index.

use core::fmt;

    /// Globally unique task identifier.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TaskId(u64);

impl TaskId {
    /// Kernel idle task (TaskId(0)).
    pub const IDLE: TaskId = TaskId(0);

    /// From raw u64.
    pub const fn from_raw(raw: u64) -> Self {
        TaskId(raw)
    }

    /// From slot index and generation.
    pub const fn new(index: u32, generation: u32) -> Self {
        TaskId(((generation as u64) << 32) | (index as u64))
    }

    /// Raw u64.
    pub const fn raw(&self) -> u64 {
        self.0
    }

    /// Slot index (lower 32 bits).
    pub const fn index(&self) -> u32 {
        (self.0 & 0xFFFF_FFFF) as u32
    }

    /// Generation (upper 32 bits).
    pub const fn generation(&self) -> u32 {
        (self.0 >> 32) as u32
    }

    /// New TaskId with generation incremented.
    pub const fn next_generation(&self) -> TaskId {
        TaskId(self.0.wrapping_add(1u64 << 32))
    }

    /// True if this is the idle task.
    pub const fn is_idle(&self) -> bool {
        self.0 == 0
    }
}

impl fmt::Debug for TaskId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "TaskId(gen={}, idx={})", self.generation(), self.index())
    }
}

impl fmt::Display for TaskId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

    /// Atomic TaskId for lock-free structures.
pub struct AtomicTaskId {
    inner: core::sync::atomic::AtomicU64,
}

impl AtomicTaskId {
    pub const fn new(id: TaskId) -> Self {
        Self {
            inner: core::sync::atomic::AtomicU64::new(id.0),
        }
    }

    pub fn load(&self, order: core::sync::atomic::Ordering) -> TaskId {
        TaskId(self.inner.load(order))
    }

    pub fn store(&self, id: TaskId, order: core::sync::atomic::Ordering) {
        self.inner.store(id.0, order);
    }

    pub fn swap(&self, id: TaskId, order: core::sync::atomic::Ordering) -> TaskId {
        TaskId(self.inner.swap(id.0, order))
    }

    pub fn compare_exchange(
        &self,
        current: TaskId,
        new: TaskId,
        success: core::sync::atomic::Ordering,
        failure: core::sync::atomic::Ordering,
    ) -> Result<TaskId, TaskId> {
        self.inner
            .compare_exchange(current.0, new.0, success, failure)
            .map(TaskId)
            .map_err(TaskId)
    }
}
