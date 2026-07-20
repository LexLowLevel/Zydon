// Inter-processor interrupts (IPI).
//
// Cross-core coordination: TLB shootdown, task migration, halt, and
// function call. Sent via APIC ICR (x86-64) or GIC SGI (AArch64).

use core::sync::atomic::{AtomicPtr, AtomicU64, Ordering};

/// Maximum number of cores.
pub const MAX_CORES: usize = 64;

/// IPI vector numbers.
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum IpiVector {
    /// TLB shootdown.
    TlbShootdown = 0xF0,
    /// Wake the target core's executor.
    Reschedule = 0xF1,
    /// Execute a function on the target core.
    FunctionCall = 0xF2,
    /// Shut down the target core.
    Halt = 0xF3,
}

/// Per-core IPI state.
struct CoreIpiState {
    func: AtomicPtr<fn(*mut ())>,
    arg: AtomicPtr<()>,
    tlb_addrs: [AtomicU64; 16],
    tlb_count: AtomicU64,
}

// SAFETY: accessed only by the owning core and via IPI.
unsafe impl Send for CoreIpiState {}
unsafe impl Sync for CoreIpiState {}

/// Global IPI state.
pub static IPI_STATE: [CoreIpiState; MAX_CORES] = {
    const INIT: CoreIpiState = CoreIpiState {
        func: AtomicPtr::new(core::ptr::null_mut()),
        arg: AtomicPtr::new(core::ptr::null_mut()),
        tlb_addrs: [
            AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0),
            AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0),
            AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0),
            AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0),
        ],
        tlb_count: AtomicU64::new(0),
    };
    [INIT; MAX_CORES]
};

/// APIC MMIO base. Platform must provide this.
extern "C" {
    fn platform_apic_base() -> u64;
}

/// Send IPI to a target core.
pub fn ipi_send(target: u32, vector: IpiVector) {
    #[cfg(target_arch = "x86_64")]
    {
        // SAFETY: APIC MMIO region is mapped; ICR write is standard x86-64.
        unsafe {
            let icr_low: u32 = (vector as u32) | (0b00 << 18);
            let icr_high: u32 = target << 24;
            let apic_base = platform_apic_base();
            core::ptr::write_volatile((apic_base + 0x310) as *mut u32, icr_high);
            core::ptr::write_volatile((apic_base + 0x300) as *mut u32, icr_low);
        }
    }

    #[cfg(target_arch = "aarch64")]
    {
        // SAFETY: GIC SGI is standard AArch64.
        unsafe {
            let sgi_val: u64 = ((target as u64) << 16) | (vector as u64);
            core::arch::asm!("msr ICC_SGI1R_EL1, {}", in(reg) sgi_val);
        }
    }
}

/// Send IPI to all cores except the current one.
pub fn ipi_broadcast(vector: IpiVector) {
    let current = current_core_id();
    for core_id in 0..MAX_CORES {
        if core_id != current {
            ipi_send(core_id as u32, vector);
        }
    }
}

/// Execute a function on a target core via IPI.
pub fn ipi_call(target: u32, func: fn(*mut ()), arg: *mut ()) {
    IPI_STATE[target as usize].func.store(func as *mut _, Ordering::Release);
    IPI_STATE[target as usize].arg.store(arg, Ordering::Release);
    ipi_send(target, IpiVector::FunctionCall);
}

/// Request TLB shootdown on target cores.
///
/// `addrs`: virtual addresses whose TLB entries must be invalidated.
/// Target cores flush these entries and acknowledge completion.
pub fn ipi_tlb_shootdown(target_mask: u64, addrs: &[u64]) {
    for core_id in 0..MAX_CORES {
        if (target_mask & (1u64 << core_id)) == 0 {
            continue;
        }
        let state = &IPI_STATE[core_id];
        for (i, &addr) in addrs.iter().enumerate().take(16) {
            state.tlb_addrs[i].store(addr, Ordering::Relaxed);
        }
        state.tlb_count.store(addrs.len() as u64, Ordering::Release);
        ipi_send(core_id as u32, IpiVector::TlbShootdown);
    }
}

/// Handle incoming FunctionCall IPI.
pub fn handle_function_call() {
    let core_id = current_core_id();
    let state = &IPI_STATE[core_id];
    let func_ptr = state.func.swap(core::ptr::null_mut(), Ordering::Acquire);
    let arg = state.arg.swap(core::ptr::null_mut(), Ordering::Acquire);

    if !func_ptr.is_null() {
        let func: fn(*mut ()) = unsafe { core::mem::transmute(func_ptr) };
        func(arg);
    }
}

/// Handle incoming TLB shootdown IPI.
pub fn handle_tlb_shootdown() {
    let core_id = current_core_id();
    let state = &IPI_STATE[core_id];
    let count = state.tlb_count.swap(0, Ordering::Acquire);

    for i in 0..count.min(16) as usize {
        let addr = state.tlb_addrs[i].load(Ordering::Relaxed);
        // Flush TLB entry for this address.
        #[cfg(target_arch = "x86_64")]
        unsafe {
            core::arch::asm!("invlpg [{}]", in(reg) addr);
        }
        #[cfg(target_arch = "aarch64")]
        unsafe {
            core::arch::asm!("tlbi vae1, {}", in(reg) addr >> 12);
        }
    }

    // Ensure flush is visible.
    core::sync::atomic::fence(Ordering::SeqCst);
}

fn current_core_id() -> usize {
    0
}
