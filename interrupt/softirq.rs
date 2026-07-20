// Deferred work executed with interrupts enabled.
// Hard IRQ handlers run with interrupts disabled; completion must be swift.
// Deferred work is scheduled via a per-core bitmap; the executor polls
// it after hard IRQ return. Higher-numbered types have higher priority.

use core::sync::atomic::{AtomicU64, Ordering};

/// Softirq type.
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SoftirqType {
    /// Timer expiry.
    Timer = 0,
    /// Cross-core task wakeup.
    IpcWake = 1,
    /// Scheduler rebalance.
    Sched = 2,
    /// TLB shootdown ack.
    TlbShootdown = 3,
    /// Network packet processing.
    Net = 4,
    /// Block I/O completion.
    Block = 5,
}

/// Number of softirq types.
pub const NUM_SOFTIRQ: usize = 6;

/// Per-core softirq bitmap. Bit N set indicates type N is pending.
pub static SOFTIRQ_BITMAPS: [AtomicU64; 64] = {
    // SAFETY: AtomicU64(0) is valid.
    const ZERO: AtomicU64 = AtomicU64::new(0);
    [ZERO; 64]
};

/// Softirq handler function type.
type SoftirqHandler = fn();

/// Global softirq handler table.
pub static SOFTIRQ_HANDLERS: [Option<SoftirqHandler>; NUM_SOFTIRQ] = [
    None, // Timer
    None, // IpcWake
    None, // Sched
    None, // TlbShootdown
    None, // Net
    None, // Block
];

/// Raise a softirq on the current core.
///
/// Sets the corresponding bit; the executor will run the handler on its next poll.
pub fn raise(irq_type: SoftirqType) {
    let core_id = current_core_id();
    SOFTIRQ_BITMAPS[core_id].fetch_or(1u64 << (irq_type as u8), Ordering::Release);
}

/// Raise a softirq on a specific core.
pub fn raise_on_core(core_id: usize, irq_type: SoftirqType) {
    SOFTIRQ_BITMAPS[core_id].fetch_or(1u64 << (irq_type as u8), Ordering::Release);
}

/// Run all pending softirqs on the current core.
///
/// Called on the idle path when the run queue is empty.
/// Handlers execute in priority order (highest first).
pub fn run_pending() {
    let core_id = current_core_id();
    let mut bitmap = SOFTIRQ_BITMAPS[core_id].swap(0, Ordering::Acquire);

    while bitmap != 0 {
        let bit = 63 - bitmap.leading_zeros() as usize;
        bitmap &= !(1u64 << bit);

        if bit < NUM_SOFTIRQ {
            if let Some(handler) = SOFTIRQ_HANDLERS[bit] {
                handler();
            }
        }
    }
}

/// Returns true if any softirq is pending on the current core.
pub fn has_pending() -> bool {
    let core_id = current_core_id();
    SOFTIRQ_BITMAPS[core_id].load(Ordering::Acquire) != 0
}

fn current_core_id() -> usize {
    // Per-CPU register read. Placeholder.
    0
}