// IRQ descriptor and dispatch table.
//
// Each hardware interrupt vector maps to an IRQ descriptor containing
// the handler function, affinity mask, and priority class. The dispatch
// table is indexed by vector number for O(1) lookup in the IDT handler.

use core::sync::atomic::{AtomicU64, Ordering};

use crate::primitives::spinlock::Spinlock;

/// Maximum number of IRQ vectors (x86-64: 256, AArch64: 1024).
pub const MAX_IRQ_VECTORS: usize = 256;

/// IRQ priority classes. Higher value = higher priority.
/// Hard IRQ handlers run with interrupts disabled on the current core.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum IrqPriority {
    /// Normal device interrupt.
    Normal = 0,
    /// High-frequency device (network, storage).
    High = 1,
    /// Timer interrupt.
    Timer = 2,
    /// IPI (inter-processor interrupt).
    Ipi = 3,
    /// Critical (panic, watchdog).
    Critical = 4,
}

/// IRQ handler function type.
///
/// Called with interrupts disabled on the current core.
/// Returns true if the interrupt was handled, false if it was spurious.
pub type IrqHandler = fn(&mut TrapFrame) -> bool;

/// Trap frame: saved CPU state at interrupt entry.
///
/// Architecture-specific. On x86-64, this contains the register save
/// area pushed by the IDT stub. On AArch64, it contains the register
/// save area pushed by the exception vector.
#[repr(C)]
pub struct TrapFrame {
    /// General-purpose registers (architecture-specific layout).
    pub regs: [u64; 31],
    /// Stack pointer.
    pub sp: u64,
    /// Program counter (return address).
    pub pc: u64,
    /// Processor status register.
    pub pstate: u64,
    /// Interrupt vector number.
    pub vector: u32,
    /// Error code (x86-64) or ESR (AArch64).
    pub error_code: u64,
}

/// IRQ descriptor.
pub struct IrqDescriptor {
    /// The handler function.
    pub handler: IrqHandler,
    /// Affinity bitmask: which cores may handle this IRQ.
    pub affinity: AtomicU64,
    /// Priority class.
    pub priority: IrqPriority,
    /// Name for debugging.
    pub name: &'static str,
}

/// Global IRQ dispatch table.
pub static IRQ_TABLE: IrqTable = IrqTable::new();

pub struct IrqTable {
    /// Descriptors indexed by vector number.
    descriptors: Spinlock<[Option<IrqDescriptor>; MAX_IRQ_VECTORS]>,
}

impl IrqTable {
    pub const fn new() -> Self {
        // Const-init: Option::None for each slot.
        // In Rust, we can't loop in const fn, so we use a macro.
        macro_rules! none_array {
            ($($i:expr),*) => { [$(None,)*] };
        }
        Self {
            descriptors: Spinlock::new(none_array!(
                0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,
                16,17,18,19,20,21,22,23,24,25,26,27,28,29,30,31,
                32,33,34,35,36,37,38,39,40,41,42,43,44,45,46,47,
                48,49,50,51,52,53,54,55,56,57,58,59,60,61,62,63,
                64,65,66,67,68,69,70,71,72,73,74,75,76,77,78,79,
                80,81,82,83,84,85,86,87,88,89,90,91,92,93,94,95,
                96,97,98,99,100,101,102,103,104,105,106,107,108,109,110,111,
                112,113,114,115,116,117,118,119,120,121,122,123,124,125,126,127,
                128,129,130,131,132,133,134,135,136,137,138,139,140,141,142,143,
                144,145,146,147,148,149,150,151,152,153,154,155,156,157,158,159,
                160,161,162,163,164,165,166,167,168,169,170,171,172,173,174,175,
                176,177,178,179,180,181,182,183,184,185,186,187,188,189,190,191,
                192,193,194,195,196,197,198,199,200,201,202,203,204,205,206,207,
                208,209,210,211,212,213,214,215,216,217,218,219,220,221,222,223,
                224,225,226,227,228,229,230,231,232,233,234,235,236,237,238,239,
                240,241,242,243,244,245,246,247,248,249,250,251,252,253,254,255
            )),
        }
    }

    /// Register an IRQ handler for the given vector.
    pub fn register(
        &self,
        vector: u8,
        handler: IrqHandler,
        affinity: u64,
        priority: IrqPriority,
        name: &'static str,
    ) {
        let mut table = self.descriptors.lock();
        table[vector as usize] = Some(IrqDescriptor {
            handler,
            affinity: AtomicU64::new(affinity),
            priority,
            name,
        });
    }

    /// Dispatch an interrupt. Called from the IDT/exception vector stub.
    ///
    /// Returns true if the interrupt was handled.
    pub fn dispatch(&self, frame: &mut TrapFrame) -> bool {
        let vector = frame.vector as usize;
        if vector >= MAX_IRQ_VECTORS {
            return false;
        }

        let table = self.descriptors.lock();
        if let Some(ref desc) = table[vector] {
            (desc.handler)(frame)
        } else {
            false
        }
    }
}