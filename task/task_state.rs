// Task state machine (AtomicU32, CAS transitions).
//
//   CREATED → READY → RUNNING → DYING → DEAD
//               ▲        │
//               └── BLOCKED

use core::sync::atomic::{AtomicU32, Ordering};
use core::fmt;

    /// Task lifecycle states.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TaskState {
    Created = 0,
    Ready = 1,
    Running = 2,
    Blocked = 3,
    Dying = 4,
    Dead = 5,
}

impl TaskState {
    /// Decode from u32; None if invalid.
    pub fn from_u32(val: u32) -> Option<Self> {
        match val {
            0 => Some(TaskState::Created),
            1 => Some(TaskState::Ready),
            2 => Some(TaskState::Running),
            3 => Some(TaskState::Blocked),
            4 => Some(TaskState::Dying),
            5 => Some(TaskState::Dead),
            _ => None,
        }
    }
}

impl fmt::Display for TaskState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TaskState::Created => write!(f, "CREATED"),
            TaskState::Ready => write!(f, "READY"),
            TaskState::Running => write!(f, "RUNNING"),
            TaskState::Blocked => write!(f, "BLOCKED"),
            TaskState::Dying => write!(f, "DYING"),
            TaskState::Dead => write!(f, "DEAD"),
        }
    }
}

    /// Atomic wrapper with CAS-based state transitions.
pub struct AtomicTaskState {
    state: AtomicU32,
}

impl AtomicTaskState {
    pub const fn new(initial: TaskState) -> Self {
        Self {
            state: AtomicU32::new(initial as u32),
        }
    }

    /// Current state (Acquire).
    pub fn get(&self) -> TaskState {
        let val = self.state.load(Ordering::Acquire);
        TaskState::from_u32(val).expect("invalid task state in AtomicTaskState")
    }

    /// CAS transition. Err(actual) if current != expected.
    pub fn transition(
        &self,
        expected: TaskState,
        new: TaskState,
    ) -> Result<(), TaskState> {
        self.state
            .compare_exchange(
                expected as u32,
                new as u32,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .map(|_| ())
            .map_err(|val| {
                TaskState::from_u32(val).expect("invalid task state")
            })
    }

    /// Unconditional store. Bypasses state machine.
    /// SAFETY: only during creation/destruction, no concurrent access.
    pub unsafe fn set(&self, new: TaskState) {
        self.state.store(new as u32, Ordering::Release);
    }
}
