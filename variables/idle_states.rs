// x86 idle states (C-states) for power management.
// Translated from zircon/kernel/arch/x86/include/arch/x86/idle_states.h
// Every processor must support at least C1.

use core::sync::atomic::{AtomicU32, Ordering};
use crate::variables::time::{ZxDuration, zx_duration_from_usec, zx_duration_add_duration};
use crate::variables::bits::bits_shift;

pub const X86_MAX_CSTATES: usize = 12;

// C1 constants

/// C1 is the shallowest idle state, always supported.
pub const X86_CSTATE_C1_NAME: &str = "C1";
pub const X86_CSTATE_C1_MWAIT_HINT: u32 = 0x00;

/// Only allow C1.
pub const K_X86_IDLE_STATE_MASK_C1_ONLY: u32 = 0x1;

// Raw idle state data (from hardware config)

/// A single idle state definition.
#[derive(Debug, Clone, Copy)]
pub struct X86IdleStateRaw {
    pub name: &'static str,
    pub mwait_hint: u32,
    pub exit_latency: u32,  // microseconds
    pub flushes_tlb: bool,
}

/// Construct a C1 idle state with the given exit latency.
pub const fn c1(exit_latency_us: u32) -> X86IdleStateRaw {
    X86IdleStateRaw {
        name: X86_CSTATE_C1_NAME,
        mwait_hint: X86_CSTATE_C1_MWAIT_HINT,
        exit_latency: exit_latency_us,
        flushes_tlb: false,
    }
}

/// List of idle states supported by the system.
/// Sorted by descending latency. Must end with C1.
#[derive(Debug, Clone)]
pub struct X86IdleStatesConfig {
    pub states: [X86IdleStateRaw; X86_MAX_CSTATES],
    /// Bitmask of allowed MWAIT hints. Bit x = MWAIT(Cx+1).
    /// Always allows C1 (bit 0 is ignored). Overridable via `k idlestates setmask`.
    pub default_state_mask: u32,
}

impl Default for X86IdleStatesConfig {
    fn default() -> Self {
        Self {
            states: [X86IdleStateRaw {
                name: "",
                mwait_hint: 0,
                exit_latency: 0,
                flushes_tlb: false,
            }; X86_MAX_CSTATES],
            default_state_mask: 0,
        }
    }
}

/// Returns true if the state is the base C1 idle state.
pub fn x86_is_base_idle_state(state: &X86IdleStateRaw) -> bool {
    state.mwait_hint == X86_CSTATE_C1_MWAIT_HINT
}

/// Returns the number of states in `states`.
/// If `states` is invalid (i.e. does not contain X86_CSTATE_C1), returns None.
pub fn x86_num_idle_states(states: &X86IdleStatesConfig) -> Option<usize> {
    for num in 0..X86_MAX_CSTATES {
        if x86_is_base_idle_state(&states.states[num]) {
            return Some(num + 1);
        }
    }
    None
}

// Runtime idle state (with counters)

/// Single idle state with runtime tracking.
#[derive(Debug, Clone)]
pub struct X86IdleState {
    state: X86IdleStateRaw,
    times_entered: u64,
    total_duration: ZxDuration,
}

impl X86IdleState {
    pub fn new(state: &X86IdleStateRaw) -> Self {
        Self {
            state: *state,
            times_entered: 0,
            total_duration: 0,
        }
    }

    pub fn name(&self) -> &str {
        self.state.name
    }

    pub fn mwait_hint(&self) -> u32 {
        self.state.mwait_hint
    }

    /// Exit latency in nanoseconds.
    pub fn exit_latency(&self) -> ZxDuration {
        zx_duration_from_usec(self.state.exit_latency as i64)
    }

    pub fn is_base_state(&self) -> bool {
        x86_is_base_idle_state(&self.state)
    }

    pub fn flushes_tlb(&self) -> bool {
        self.state.flushes_tlb
    }

    pub fn times_entered(&self) -> u64 {
        self.times_entered
    }

    pub fn cumulative_duration(&self) -> ZxDuration {
        self.total_duration
    }

    pub fn reset_counters(&mut self) {
        self.times_entered = 0;
        self.total_duration = 0;
    }

    pub fn count_entry(&mut self) {
        self.times_entered += 1;
    }

    pub fn record_duration(&mut self, duration: ZxDuration) {
        self.total_duration = zx_duration_add_duration(self.total_duration, duration);
    }
}

// Runtime idle states manager

const IDLE_DURATION_FACTOR: u32 = 3;

fn state_number_from_mwait_hint(hint: u32) -> u32 {
    bits_shift(hint, 8, 4) + 1
}

/// Manages idle states for a CPU and picks the best state to enter.
#[derive(Debug)]
pub struct X86IdleStates {
    states: [Option<X86IdleState>; X86_MAX_CSTATES],
    num_states: usize,
    last_idle_duration: ZxDuration,
    state_mask: AtomicU32,
}

impl X86IdleStates {
    /// Construct from hardware configuration.
    pub fn new(config: &X86IdleStatesConfig) -> Self {
        let num_states = x86_num_idle_states(config)
            .expect("Invalid C-state configuration: Expected at least C1 to be defined.");

        let mut states: [Option<X86IdleState>; X86_MAX_CSTATES] =
            [const { None }; X86_MAX_CSTATES];
        for i in 0..num_states {
            states[i] = Some(X86IdleState::new(&config.states[i]));
        }

        let state_mask = config.default_state_mask | 0x1; // always allow C1

        Self {
            states,
            num_states,
            last_idle_duration: 0,
            state_mask: AtomicU32::new(state_mask),
        }
    }

    pub fn states(&mut self) -> &mut [Option<X86IdleState>; X86_MAX_CSTATES] {
        &mut self.states
    }

    pub fn const_states(&self) -> &[Option<X86IdleState>; X86_MAX_CSTATES] {
        &self.states
    }

    pub fn num_states(&self) -> usize {
        self.num_states
    }

    /// Pick an idle state. Returns the deepest valid state whose exit
    /// latency is less than IDLE_DURATION_FACTOR * last_idle_duration.
    pub fn pick_idle_state(&mut self) -> Option<&mut X86IdleState> {
        if self.last_idle_duration == 0 {
            // No history, use shallowest state (C1).
            return self.states[self.num_states - 1].as_mut();
        }

        let valid_state_mask = self.state_mask.load(Ordering::Relaxed);

        // Pick the deepest valid state with acceptable exit latency.
        for i in 0..self.num_states {
            if let Some(ref state) = self.states[i] {
                let state_num = state_number_from_mwait_hint(state.mwait_hint());
                if (valid_state_mask & (1 << (state_num - 1))) == 0 {
                    continue;
                }
                if state.exit_latency() < (IDLE_DURATION_FACTOR as i64) * self.last_idle_duration {
                    return self.states[i].as_mut();
                }
            }
        }

        self.states[self.num_states - 1].as_mut()
    }

    /// Record the actual idle duration for future state selection.
    pub fn record_duration(&mut self, duration: ZxDuration) {
        self.last_idle_duration = duration;
    }

    /// Update the mask of valid C-states.
    pub fn set_state_mask(&self, mask: u32) {
        self.state_mask.store(mask | 0x1, Ordering::Relaxed);
    }
}
