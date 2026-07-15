// Platform types.
// Translated from zircon/kernel/include/platform.h

pub const BOOT_CPU_ID: u32 = 0;

/// Platform halt actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum HaltAction {
    Halt = 0,
    Reboot,
    RebootBootloader,
    RebootRecovery,
    Shutdown,
}

/// Platform-specific, per-cpu state used to suspend/resume a CPU.
pub struct PlatformCpuResumeState {
    #[cfg(target_arch = "aarch64")]
    pub cntkctl_el1: u64,
}

impl Default for PlatformCpuResumeState {
    fn default() -> Self {
        Self {
            #[cfg(target_arch = "aarch64")]
            cntkctl_el1: 0,
        }
    }
}

/// Whether a suspend operation is limited to the calling CPU or may power down
/// the encapsulating power domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlatformAllowDomainPowerDown {
    No,
    Yes,
}

/// Whether to halt other CPUs during a panic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanicStartHaltOtherCpus {
    No,
    Yes,
}

impl PanicStartHaltOtherCpus {
    pub const fn default() -> Self {
        PanicStartHaltOtherCpus::Yes
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum ZxStatus {
    Ok = 0,
    ErrNotSupported = -2,
    ErrNoResources = -3,
    ErrNoMemory = -4,
    ErrInternal = -11,
    ErrInvalidArgs = -10,
    ErrBadState = -20,
    ErrTimedOut = -21,
    ErrShouldWait = -22,
    ErrCanceled = -23,
    ErrPeerClosed = -24,
    ErrNotFound = -25,
    ErrAlreadyExists = -26,
    ErrAlreadyBound = -27,
    ErrAccessDenied = -30,
}

/// CPU power state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerCpuState {
    Online,
    Offline,
    Suspending,
}

/// CPU number type.
pub type CpuNum = u32;

// --- Platform FFI functions ---

extern "C" {
    pub fn platform_early_init();
    pub fn platform_prevm_init();
    pub fn platform_init();
    pub fn platform_halt(suggested_action: HaltAction, reason: u32) -> !;
    pub fn platform_halt_cpu();
    pub fn platform_supports_suspend_cpu() -> bool;
    pub fn platform_suspend_cpu(allow_domain: PlatformAllowDomainPowerDown) -> ZxStatus;
    pub fn platform_serial_enabled() -> bool;
    pub fn platform_serial_prepare_for_suspend();
    pub fn platform_serial_wakeup_from_suspend();
    pub fn platform_panic_start(option: PanicStartHaltOtherCpus);
    pub fn platform_start_cpu(cpu_id: CpuNum, mpid: u64, context: u64) -> ZxStatus;
    pub fn platform_get_cpu_state(cpu_id: CpuNum) -> PowerCpuState;
}
