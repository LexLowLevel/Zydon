// x86 idle state management.
// Translated from arch/idt-state.be

use crate::variables::features::x86_get_microarch_config;
use crate::variables::idle_states::{
    X86IdleStatesConfig, X86IdleStates, X86IdleState, X86_MAX_CSTATES,
    x86_is_base_idle_state, x86_num_idle_states,
};

/// Get the idle states config for the current microarchitecture.
pub fn x86_get_idle_states() -> &'static X86IdleStatesConfig {
    &x86_get_microarch_config().idle_states
}

/// Number of states in the config. Returns None if invalid (no C1).
pub fn x86_num_idle_states_from_config(states: &X86IdleStatesConfig) -> Option<usize> {
    x86_num_idle_states(states)
}

/// Construct an X86IdleStates manager from hardware config.
pub fn x86_idle_states_new(config: &X86IdleStatesConfig) -> X86IdleStates {
    X86IdleStates::new(config)
}
