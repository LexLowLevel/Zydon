// Task state machine.
//
// Stored as an AtomicU32, all transitions are CAS-based.
//
// State diagram:
//
//     CREATED
//        │
//        ▼
//     READY ──────► RUNNING
//        ▲              │
//        │         ┌────┼────┐
//        │         ▼    │    ▼
//      BLOCKED ◄───┘    │   DYING
//        │              │    │
//        └──────────────┘    ▼
//                           DEAD

use core::sync::atomic::{AtomicU32, Ordering};
use core::fmt;

/// Task state values.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TaskState {
    Created = 0,
    Ready = 1,
    Running = 2,
    Blocked = 3,  // parked on I/O, IPC, etc.
    Dying = 4,    // future returned Ready
    Dead = 5,
}

impl TaskState {
    /// Convert a u32 to a TaskState. Returns None for invalid values.
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

/// Atomic task state with CAS-based transitions.
pub struct AtomicTaskState {
    state: AtomicU32,
}

impl AtomicTaskState {
    pub const fn new(initial: TaskState) -> Self {
        Self {
            state: AtomicU32::new(initial as u32),
        }
    }

    /// Load the current state.
    pub fn get(&self) -> TaskState {
        let val = self.state.load(Ordering::Acquire);
        TaskState::from_u32(val).expect("invalid task state in AtomicTaskState")
    }

    /// Attempt a state transition. Returns Err(current_state) if the
    /// current state doesn't match `expected`.
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

    /// Unconditionally set state. Bypasses the state machine.
    /// SAFETY: only use during creation/destruction when no concurrent access.
    pub unsafe fn set(&self, new: TaskState) {
        self.state.store(new as u32, Ordering::Release);
    }
}
