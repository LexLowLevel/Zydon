// Zydon: an async multicore microkernel.
//
// Tasks are Futures polled by per-core work-stealing executors.
// IPC uses lock-free SPSC rings with userspace-mapped fastpath.
// Scheduling is multilevel feedback with priority decay.
// Memory: buddy + slab allocator, zero-copy COW IPC, hierarchical timing wheel.

#![no_std]
#![feature(naked_functions)]
#![feature(const_mut_refs)]
// Rule 3: no implicit unsafe operations inside unsafe fns.
#![forbid(unsafe_op_in_unsafe_fn)]
// Rule 12: every pub item must be documented.
#![warn(missing_docs)]
// Rule 8: callers must observe Result returns.
#![warn(clippy::missing_panics_doc)]
#![warn(clippy::must_use_candidate)]
// Rule 11: no leaking in non-boot code; clippy::mem_forget covers forget.

extern crate alloc;

pub mod arch;
pub mod variables;
pub mod primitives;
pub mod object;
pub mod task;
pub mod ipc;
pub mod memory;
pub mod interrupt;