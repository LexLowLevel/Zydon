// x86 feature detection and µarch config.
// Derived from Zircon feature.h

use crate::variables::idle_states::{X86IdleStatesConfig, X86_MAX_CSTATES};

// Limits
pub const MAX_SUPPORTED_CPUID: u32 = 0x17;
pub const MAX_SUPPORTED_CPUID_HYP: u32 = 0x40000001;
pub const MAX_SUPPORTED_CPUID_EXT: u32 = 0x80000021;

// CPUID leaf enum.

#[derive(Debug, Clone, Copy, Default)]
pub struct CpuidLeaf {
    pub a: u32,
    pub b: u32,
    pub c: u32,
    pub d: u32,
}

/// CPUID leaf number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum X86CpuidLeafNum {
    Base = 0x0,
    ModelFeatures = 0x1,
    CacheV1 = 0x2,
    CacheV2 = 0x4,
    Mon = 0x5,
    ThermalAndPower = 0x6,
    ExtendedFeatureFlags = 0x7,
    PerformanceMonitoring = 0xa,
    Topology = 0xb,
    Xsave = 0xd,
    Pt = 0x14,
    Tsc = 0x15,

    HypBase = 0x40000000,
    HypVendor = 0x40000000,
    KvmFeatures = 0x40000001,

    ExtBase = 0x80000000,
    Brand = 0x80000002,
    AmdTopology = 0x8000001e,
}

/// Reference to a specific bit in a CPUID leaf.
#[derive(Debug, Clone, Copy)]
pub struct X86CpuidBit {
    pub leaf: X86CpuidLeafNum,
    pub word: u8,
    pub bit: u8,
}

pub const fn cpuid_bit(leaf: X86CpuidLeafNum, word: u8, bit: u8) -> X86CpuidBit {
    X86CpuidBit { leaf, word, bit }
}

// Vendors

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum X86Vendor {
    Unknown,
    Intel,
    Amd,
}

// Microarchitectures

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum X86Microarch {
    Unknown,
    IntelNehalem,
    IntelWestmere,
    IntelSandyBridge,
    IntelIvyBridge,
    IntelBroadwell,
    IntelHaswell,
    /// Skylake, KBL, CFL, etc.
    IntelSkylake,
    IntelCannonlake,
    IntelIcelake,
    IntelTigerlake,
    IntelAlderlake,
    /// Silvermont / Airmont
    IntelSilvermont,
    /// Goldmont
    IntelGoldmont,
    IntelGoldmontPlus,
    AmdBulldozer,
    AmdJaguar,
    AmdZen,
}

// Hypervisors

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum X86Hypervisor {
    Unknown,
    None,
    Kvm,
}

// Model info

#[derive(Debug, Clone, Copy)]
pub struct X86ModelInfo {
    pub processor_type: u8,
    pub family: u8,
    pub model: u8,
    pub stepping: u8,
    pub display_family: u32,
    pub display_model: u32,
    pub patch_level: u32,
}

// Fn pointer types

/// Timer frequency getter (0 if unknown).
pub type X86GetTimerFreqFunc = fn() -> u64;

/// Reboot (may return on failure).
pub type X86RebootSystemFunc = fn();

/// Set reason flag and reboot.
pub type X86RebootReasonFunc = fn(u64);

// µarch config

/// Per-µarch kernel configuration.
pub struct X86MicroarchConfig {
    pub microarch: X86Microarch,
    pub get_apic_freq: X86GetTimerFreqFunc,
    pub get_tsc_freq: X86GetTimerFreqFunc,
    pub reboot_system: X86RebootSystemFunc,
    pub reboot_reason: X86RebootReasonFunc,
    pub disable_c1e: bool,
    /// Prefer HLT over MWAIT in idle loop.
    pub idle_prefer_hlt: bool,
    pub idle_states: X86IdleStatesConfig,
}

// Feature flags

pub static mut G_X86_FEATURE_FSGSBASE: bool = false;
pub static mut G_X86_FEATURE_INVPCID: bool = false;
/// PCID + INVPCID enabled by cmdline.
pub static mut G_X86_FEATURE_PCID_ENABLED: bool = false;
pub static mut G_X86_FEATURE_HAS_SMAP: bool = false;

pub static mut X86_VENDOR: X86Vendor = X86Vendor::Unknown;
pub static mut X86_HYPERVISOR: X86Hypervisor = X86Hypervisor::Unknown;

pub static mut G_HYPERVISOR_HAS_PV_CLOCK: bool = false;
pub static mut G_HYPERVISOR_HAS_PV_EOI: bool = false;
pub static mut G_HYPERVISOR_HAS_PV_IPI: bool = false;

pub static mut G_HAS_IBPB: bool = false;
pub static mut G_RAS_FILL_ON_CTXT_SWITCH: bool = false;
pub static mut G_CPU_VULNERABLE_TO_RSB_UNDERFLOW: bool = false;
pub static mut G_CPU_VULNERABLE_TO_RSB_CROSS_THREAD: bool = false;
pub static mut G_SHOULD_IBPB_ON_CTXT_SWITCH: bool = false;
pub static mut G_SSB_MITIGATED: bool = false;
pub static mut G_L1D_FLUSH_ON_VMENTRY: bool = false;
pub static mut G_MD_CLEAR_ON_USER_RETURN: bool = false;
pub static mut G_HAS_ENHANCED_IBRS: bool = false;
pub static mut G_HAS_MELTDOWN: bool = false;

// Global arrays (extern linkage)

extern "C" {
    pub static mut _cpuid: [CpuidLeaf; (MAX_SUPPORTED_CPUID + 1) as usize];
    pub static mut _cpuid_hyp: [CpuidLeaf; (MAX_SUPPORTED_CPUID_HYP - X86CpuidLeafNum::HypBase as u32 + 1) as usize];
    pub static mut _cpuid_ext: [CpuidLeaf; (MAX_SUPPORTED_CPUID_EXT - X86CpuidLeafNum::ExtBase as u32 + 1) as usize];
    pub static mut max_cpuid: u32;
    pub static mut max_ext_cpuid: u32;
    pub static mut max_hyp_cpuid: u32;
    pub static x86_microarch_config: *const X86MicroarchConfig;
}

// Helper functions

pub fn x86_get_cpuid_leaf(leaf: X86CpuidLeafNum) -> Option<&'static CpuidLeaf> {
    let leaf_val = leaf as u32;
    unsafe {
        if leaf_val < X86CpuidLeafNum::HypBase as u32 {
            if leaf_val > max_cpuid {
                return None;
            }
            Some(&_cpuid[leaf_val as usize])
        } else if leaf_val < X86CpuidLeafNum::ExtBase as u32 {
            if leaf_val > max_hyp_cpuid {
                return None;
            }
            Some(&_cpuid_hyp[(leaf_val - X86CpuidLeafNum::HypBase as u32) as usize])
        } else {
            if leaf_val > max_ext_cpuid {
                return None;
            }
            Some(&_cpuid_ext[(leaf_val - X86CpuidLeafNum::ExtBase as u32) as usize])
        }
    }
}

/// Test if a CPUID feature bit is set.
pub fn x86_feature_test(bit: X86CpuidBit) -> bool {
    debug_assert!(bit.word <= 3 && bit.bit <= 31);
    if bit.word > 3 || bit.bit > 31 {
        return false;
    }
    let leaf = match x86_get_cpuid_leaf(bit.leaf) {
        Some(l) => l,
        None => return false,
    };
    let reg = match bit.word {
        0 => leaf.a,
        1 => leaf.b,
        2 => leaf.c,
        3 => leaf.d,
        _ => return false,
    };
    ((1u32 << bit.bit) & reg) != 0
}

pub fn x86_get_microarch_config() -> &'static X86MicroarchConfig {
    unsafe { &*x86_microarch_config }
}

pub fn x86_hypervisor_has_pv_clock() -> bool {
    unsafe { G_HYPERVISOR_HAS_PV_CLOCK }
}

pub fn x86_hypervisor_has_pv_eoi() -> bool {
    unsafe { G_HYPERVISOR_HAS_PV_EOI }
}

pub fn x86_hypervisor_has_pv_ipi() -> bool {
    unsafe { G_HYPERVISOR_HAS_PV_IPI }
}

pub fn x86_has_hypervisor() -> bool {
    unsafe { X86_HYPERVISOR != X86Hypervisor::None }
}

pub fn x86_cpu_has_ibpb() -> bool {
    unsafe { G_HAS_IBPB }
}

pub fn x86_cpu_should_ras_fill_on_ctxt_switch() -> bool {
    unsafe { G_RAS_FILL_ON_CTXT_SWITCH }
}

pub fn x86_cpu_vulnerable_to_rsb_cross_thread() -> bool {
    unsafe { G_CPU_VULNERABLE_TO_RSB_CROSS_THREAD }
}

pub fn x86_cpu_vulnerable_to_rsb_underflow() -> bool {
    unsafe { G_CPU_VULNERABLE_TO_RSB_UNDERFLOW }
}

pub fn x86_cpu_should_ibpb_on_ctxt_switch() -> bool {
    unsafe { G_SHOULD_IBPB_ON_CTXT_SWITCH }
}

pub fn x86_cpu_should_mitigate_ssb() -> bool {
    unsafe { G_SSB_MITIGATED }
}

pub fn x86_cpu_should_l1d_flush_on_vmentry() -> bool {
    unsafe { G_L1D_FLUSH_ON_VMENTRY }
}

pub fn x86_cpu_should_md_clear_on_user_return() -> bool {
    unsafe { G_MD_CLEAR_ON_USER_RETURN }
}

pub fn x86_cpu_has_enhanced_ibrs() -> bool {
    unsafe { G_HAS_ENHANCED_IBRS }
}

pub fn x86_cpu_has_meltdown() -> bool {
    unsafe { G_HAS_MELTDOWN }
}

// Feature bit constants (leaf, word, bit)

pub const X86_FEATURE_SSE3: X86CpuidBit = cpuid_bit(X86CpuidLeafNum::ModelFeatures, 2, 0);
pub const X86_FEATURE_MON: X86CpuidBit = cpuid_bit(X86CpuidLeafNum::ModelFeatures, 2, 3);
pub const X86_FEATURE_VMX: X86CpuidBit = cpuid_bit(X86CpuidLeafNum::ModelFeatures, 2, 5);
pub const X86_FEATURE_TM2: X86CpuidBit = cpuid_bit(X86CpuidLeafNum::ModelFeatures, 2, 8);
pub const X86_FEATURE_SSSE3: X86CpuidBit = cpuid_bit(X86CpuidLeafNum::ModelFeatures, 2, 9);
pub const X86_FEATURE_PDCM: X86CpuidBit = cpuid_bit(X86CpuidLeafNum::ModelFeatures, 2, 15);
pub const X86_FEATURE_PCID: X86CpuidBit = cpuid_bit(X86CpuidLeafNum::ModelFeatures, 2, 17);
pub const X86_FEATURE_SSE4_1: X86CpuidBit = cpuid_bit(X86CpuidLeafNum::ModelFeatures, 2, 19);
pub const X86_FEATURE_SSE4_2: X86CpuidBit = cpuid_bit(X86CpuidLeafNum::ModelFeatures, 2, 20);
pub const X86_FEATURE_X2APIC: X86CpuidBit = cpuid_bit(X86CpuidLeafNum::ModelFeatures, 2, 21);
pub const X86_FEATURE_TSC_DEADLINE: X86CpuidBit = cpuid_bit(X86CpuidLeafNum::ModelFeatures, 2, 24);
pub const X86_FEATURE_AESNI: X86CpuidBit = cpuid_bit(X86CpuidLeafNum::ModelFeatures, 2, 25);
pub const X86_FEATURE_XSAVE: X86CpuidBit = cpuid_bit(X86CpuidLeafNum::ModelFeatures, 2, 26);
pub const X86_FEATURE_AVX: X86CpuidBit = cpuid_bit(X86CpuidLeafNum::ModelFeatures, 2, 28);
pub const X86_FEATURE_RDRAND: X86CpuidBit = cpuid_bit(X86CpuidLeafNum::ModelFeatures, 2, 30);
pub const X86_FEATURE_HYPERVISOR: X86CpuidBit = cpuid_bit(X86CpuidLeafNum::ModelFeatures, 2, 31);
pub const X86_FEATURE_FPU: X86CpuidBit = cpuid_bit(X86CpuidLeafNum::ModelFeatures, 3, 0);
pub const X86_FEATURE_SEP: X86CpuidBit = cpuid_bit(X86CpuidLeafNum::ModelFeatures, 3, 11);
pub const X86_FEATURE_CLFLUSH: X86CpuidBit = cpuid_bit(X86CpuidLeafNum::ModelFeatures, 3, 19);
pub const X86_FEATURE_ACPI: X86CpuidBit = cpuid_bit(X86CpuidLeafNum::ModelFeatures, 3, 22);
pub const X86_FEATURE_MMX: X86CpuidBit = cpuid_bit(X86CpuidLeafNum::ModelFeatures, 3, 23);
pub const X86_FEATURE_FXSR: X86CpuidBit = cpuid_bit(X86CpuidLeafNum::ModelFeatures, 3, 24);
pub const X86_FEATURE_SSE: X86CpuidBit = cpuid_bit(X86CpuidLeafNum::ModelFeatures, 3, 25);
pub const X86_FEATURE_SSE2: X86CpuidBit = cpuid_bit(X86CpuidLeafNum::ModelFeatures, 3, 26);
pub const X86_FEATURE_TM: X86CpuidBit = cpuid_bit(X86CpuidLeafNum::ModelFeatures, 3, 29);
pub const X86_FEATURE_DTS: X86CpuidBit = cpuid_bit(X86CpuidLeafNum::ThermalAndPower, 0, 0);
pub const X86_FEATURE_TURBO: X86CpuidBit = cpuid_bit(X86CpuidLeafNum::ThermalAndPower, 0, 1);
pub const X86_FEATURE_PLN: X86CpuidBit = cpuid_bit(X86CpuidLeafNum::ThermalAndPower, 0, 4);
pub const X86_FEATURE_PTM: X86CpuidBit = cpuid_bit(X86CpuidLeafNum::ThermalAndPower, 0, 6);
pub const X86_FEATURE_HWP: X86CpuidBit = cpuid_bit(X86CpuidLeafNum::ThermalAndPower, 0, 7);
pub const X86_FEATURE_HWP_NOT: X86CpuidBit = cpuid_bit(X86CpuidLeafNum::ThermalAndPower, 0, 8);
pub const X86_FEATURE_HWP_ACT: X86CpuidBit = cpuid_bit(X86CpuidLeafNum::ThermalAndPower, 0, 9);
pub const X86_FEATURE_HWP_PREF: X86CpuidBit = cpuid_bit(X86CpuidLeafNum::ThermalAndPower, 0, 10);
pub const X86_FEATURE_TURBO_MAX: X86CpuidBit = cpuid_bit(X86CpuidLeafNum::ThermalAndPower, 0, 14);
pub const X86_FEATURE_HW_FEEDBACK: X86CpuidBit = cpuid_bit(X86CpuidLeafNum::ThermalAndPower, 2, 0);
pub const X86_FEATURE_PERF_BIAS: X86CpuidBit = cpuid_bit(X86CpuidLeafNum::ThermalAndPower, 2, 3);
pub const X86_FEATURE_FSGSBASE: X86CpuidBit = cpuid_bit(X86CpuidLeafNum::ExtendedFeatureFlags, 1, 0);
pub const X86_FEATURE_TSC_ADJUST: X86CpuidBit = cpuid_bit(X86CpuidLeafNum::ExtendedFeatureFlags, 1, 1);
pub const X86_FEATURE_AVX2: X86CpuidBit = cpuid_bit(X86CpuidLeafNum::ExtendedFeatureFlags, 1, 5);
pub const X86_FEATURE_SMEP: X86CpuidBit = cpuid_bit(X86CpuidLeafNum::ExtendedFeatureFlags, 1, 7);
pub const X86_FEATURE_ERMS: X86CpuidBit = cpuid_bit(X86CpuidLeafNum::ExtendedFeatureFlags, 1, 9);
pub const X86_FEATURE_INVPCID: X86CpuidBit = cpuid_bit(X86CpuidLeafNum::ExtendedFeatureFlags, 1, 10);
pub const X86_FEATURE_AVX512F: X86CpuidBit = cpuid_bit(X86CpuidLeafNum::ExtendedFeatureFlags, 1, 16);
pub const X86_FEATURE_AVX512DQ: X86CpuidBit = cpuid_bit(X86CpuidLeafNum::ExtendedFeatureFlags, 1, 17);
pub const X86_FEATURE_RDSEED: X86CpuidBit = cpuid_bit(X86CpuidLeafNum::ExtendedFeatureFlags, 1, 18);
pub const X86_FEATURE_SMAP: X86CpuidBit = cpuid_bit(X86CpuidLeafNum::ExtendedFeatureFlags, 1, 20);
pub const X86_FEATURE_AVX512IFMA: X86CpuidBit = cpuid_bit(X86CpuidLeafNum::ExtendedFeatureFlags, 1, 21);
pub const X86_FEATURE_CLFLUSHOPT: X86CpuidBit = cpuid_bit(X86CpuidLeafNum::ExtendedFeatureFlags, 1, 23);
pub const X86_FEATURE_CLWB: X86CpuidBit = cpuid_bit(X86CpuidLeafNum::ExtendedFeatureFlags, 1, 24);
pub const X86_FEATURE_PT: X86CpuidBit = cpuid_bit(X86CpuidLeafNum::ExtendedFeatureFlags, 1, 25);
pub const X86_FEATURE_AVX512PF: X86CpuidBit = cpuid_bit(X86CpuidLeafNum::ExtendedFeatureFlags, 1, 26);
pub const X86_FEATURE_AVX512ER: X86CpuidBit = cpuid_bit(X86CpuidLeafNum::ExtendedFeatureFlags, 1, 27);
pub const X86_FEATURE_AVX512CD: X86CpuidBit = cpuid_bit(X86CpuidLeafNum::ExtendedFeatureFlags, 1, 28);
pub const X86_FEATURE_AVX512BW: X86CpuidBit = cpuid_bit(X86CpuidLeafNum::ExtendedFeatureFlags, 1, 30);
pub const X86_FEATURE_AVX512VL: X86CpuidBit = cpuid_bit(X86CpuidLeafNum::ExtendedFeatureFlags, 1, 31);
pub const X86_FEATURE_AVX512VBMI: X86CpuidBit = cpuid_bit(X86CpuidLeafNum::ExtendedFeatureFlags, 2, 1);
pub const X86_FEATURE_UMIP: X86CpuidBit = cpuid_bit(X86CpuidLeafNum::ExtendedFeatureFlags, 2, 2);
pub const X86_FEATURE_PKU: X86CpuidBit = cpuid_bit(X86CpuidLeafNum::ExtendedFeatureFlags, 2, 3);
pub const X86_FEATURE_AVX512VBMI2: X86CpuidBit = cpuid_bit(X86CpuidLeafNum::ExtendedFeatureFlags, 2, 6);
pub const X86_FEATURE_AVX512VNNI: X86CpuidBit = cpuid_bit(X86CpuidLeafNum::ExtendedFeatureFlags, 2, 11);
pub const X86_FEATURE_AVX512BITALG: X86CpuidBit = cpuid_bit(X86CpuidLeafNum::ExtendedFeatureFlags, 2, 12);
pub const X86_FEATURE_AVX512VPDQ: X86CpuidBit = cpuid_bit(X86CpuidLeafNum::ExtendedFeatureFlags, 2, 14);
pub const X86_FEATURE_AVX512QVNNIW: X86CpuidBit = cpuid_bit(X86CpuidLeafNum::ExtendedFeatureFlags, 3, 2);
pub const X86_FEATURE_AVX512QFMA: X86CpuidBit = cpuid_bit(X86CpuidLeafNum::ExtendedFeatureFlags, 3, 3);
pub const X86_FEATURE_MD_CLEAR: X86CpuidBit = cpuid_bit(X86CpuidLeafNum::ExtendedFeatureFlags, 3, 10);
pub const X86_FEATURE_IBRS_IBPB: X86CpuidBit = cpuid_bit(X86CpuidLeafNum::ExtendedFeatureFlags, 3, 26);
pub const X86_FEATURE_STIBP: X86CpuidBit = cpuid_bit(X86CpuidLeafNum::ExtendedFeatureFlags, 3, 27);
pub const X86_FEATURE_L1D_FLUSH: X86CpuidBit = cpuid_bit(X86CpuidLeafNum::ExtendedFeatureFlags, 3, 28);
pub const X86_FEATURE_ARCH_CAPABILITIES: X86CpuidBit = cpuid_bit(X86CpuidLeafNum::ExtendedFeatureFlags, 3, 29);
pub const X86_FEATURE_SSBD: X86CpuidBit = cpuid_bit(X86CpuidLeafNum::ExtendedFeatureFlags, 3, 31);

pub const X86_FEATURE_TOPOLOGY_SHIFT: X86CpuidBit = cpuid_bit(X86CpuidLeafNum::Topology, 0, 0);
pub const X86_FEATURE_TOPOLOGY_SINGLE_PROCESSOR: X86CpuidBit = cpuid_bit(X86CpuidLeafNum::Topology, 0, 1);
pub const X86_FEATURE_TOPOLOGY_LOGICAL_PROCESSOR: X86CpuidBit = cpuid_bit(X86CpuidLeafNum::Topology, 0, 8);
pub const X86_FEATURE_TOPOLOGY_CORE: X86CpuidBit = cpuid_bit(X86CpuidLeafNum::Topology, 0, 9);

pub const X86_FEATURE_KVM_PV_CLOCK: X86CpuidBit = cpuid_bit(X86CpuidLeafNum::KvmFeatures, 0, 3);
pub const X86_FEATURE_KVM_PV_EOI: X86CpuidBit = cpuid_bit(X86CpuidLeafNum::KvmFeatures, 0, 6);
pub const X86_FEATURE_KVM_PV_IPI: X86CpuidBit = cpuid_bit(X86CpuidLeafNum::KvmFeatures, 0, 11);
pub const X86_FEATURE_KVM_PV_CLOCK_STABLE: X86CpuidBit = cpuid_bit(X86CpuidLeafNum::KvmFeatures, 0, 24);

pub const X86_FEATURE_AMD_TOPO: X86CpuidBit = cpuid_bit(X86CpuidLeafNum::ExtBase, 2, 22);
pub const X86_FEATURE_SYSCALL: X86CpuidBit = cpuid_bit(X86CpuidLeafNum::ExtBase, 3, 11);
pub const X86_FEATURE_NX: X86CpuidBit = cpuid_bit(X86CpuidLeafNum::ExtBase, 3, 20);
pub const X86_FEATURE_HUGE_PAGE: X86CpuidBit = cpuid_bit(X86CpuidLeafNum::ExtBase, 3, 26);
pub const X86_FEATURE_RDTSCP: X86CpuidBit = cpuid_bit(X86CpuidLeafNum::ExtBase, 3, 27);
pub const X86_FEATURE_INVAR_TSC: X86CpuidBit = cpuid_bit(X86CpuidLeafNum::ExtBase, 3, 8);
pub const X86_FEATURE_INVLPGB: X86CpuidBit = cpuid_bit(X86CpuidLeafNum::ExtBase, 1, 3);
