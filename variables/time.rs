// Time types and arithmetic.
// Derived from Zircon time.h
// Naming: zx_<kind>_<timeline>_[units_]t
//   kind: instant | duration; timeline: mono | boot; units: ticks | ns (default)

// Monomorphic types

/// Monotonic time point (ns).
pub type ZxInstantMono = i64;
/// Monotonic time point (ticks).
pub type ZxInstantMonoTicks = i64;
/// Monotonic duration (ns).
pub type ZxDurationMono = i64;
/// Monotonic duration (ticks).
pub type ZxDurationMonoTicks = i64;

/// Boot time point (ns).
pub type ZxInstantBoot = i64;
/// Boot time point (ticks).
pub type ZxInstantBootTicks = i64;
/// Boot duration (ns).
pub type ZxDurationBoot = i64;
/// Boot duration (ticks).
pub type ZxDurationBootTicks = i64;

// Polymorphic types

/// Time point (ns).
pub type ZxTime = i64;
/// Duration (ns).
pub type ZxDuration = i64;
/// Time in hardware ticks.
pub type ZxTicks = i64;

// Special values

pub const ZX_TIME_INFINITE: i64 = i64::MAX;
pub const ZX_TIME_INFINITE_PAST: i64 = i64::MIN;

// Overflow-safe arithmetic

/// Saturating time + duration.
pub const fn zx_time_add_duration(time: ZxTime, duration: ZxDuration) -> ZxTime {
    match time.checked_add(duration) {
        Some(x) => x,
        None => {
            if time.saturating_add(duration) >= 0 {
                ZX_TIME_INFINITE_PAST
            } else {
                ZX_TIME_INFINITE
            }
        }
    }
}

/// Saturating time − duration.
pub const fn zx_time_sub_duration(time: ZxTime, duration: ZxDuration) -> ZxTime {
    match time.checked_sub(duration) {
        Some(x) => x,
        None => {
            if time.saturating_sub(duration) >= 0 {
                ZX_TIME_INFINITE_PAST
            } else {
                ZX_TIME_INFINITE
            }
        }
    }
}

/// Saturating time − time → duration.
pub const fn zx_time_sub_time(time1: ZxTime, time2: ZxTime) -> ZxDuration {
    match time1.checked_sub(time2) {
        Some(x) => x,
        None => {
            if time1.saturating_sub(time2) >= 0 {
                ZX_TIME_INFINITE_PAST
            } else {
                ZX_TIME_INFINITE
            }
        }
    }
}

/// Saturating duration + duration.
pub const fn zx_duration_add_duration(dur1: ZxDuration, dur2: ZxDuration) -> ZxDuration {
    dur1.saturating_add(dur2)
}

/// Saturating duration − duration.
pub const fn zx_duration_sub_duration(dur1: ZxDuration, dur2: ZxDuration) -> ZxDuration {
    dur1.saturating_sub(dur2)
}

/// Saturating duration × i64.
pub const fn zx_duration_mul_int64(duration: ZxDuration, multiplier: i64) -> ZxDuration {
    match duration.checked_mul(multiplier) {
        Some(x) => x,
        None => {
            if (duration > 0 && multiplier > 0) || (duration < 0 && multiplier < 0) {
                ZX_TIME_INFINITE
            } else {
                ZX_TIME_INFINITE_PAST
            }
        }
    }
}

pub const fn zx_nsec_from_duration(n: ZxDuration) -> i64 {
    n
}

// Conversions

pub const fn zx_duration_from_nsec(n: i64) -> ZxDuration {
    zx_duration_mul_int64(1, n)
}

pub const fn zx_duration_from_usec(n: i64) -> ZxDuration {
    zx_duration_mul_int64(1_000, n)
}

pub const fn zx_duration_from_msec(n: i64) -> ZxDuration {
    zx_duration_mul_int64(1_000_000, n)
}

pub const fn zx_duration_from_sec(n: i64) -> ZxDuration {
    zx_duration_mul_int64(1_000_000_000, n)
}

pub const fn zx_duration_from_min(n: i64) -> ZxDuration {
    zx_duration_mul_int64(60_000_000_000, n)
}

pub const fn zx_duration_from_hour(n: i64) -> ZxDuration {
    zx_duration_mul_int64(3_600_000_000_000, n)
}

// Tick ops

pub const fn zx_ticks_add_ticks(ticks1: ZxTicks, ticks2: ZxTicks) -> ZxTicks {
    ticks1.saturating_add(ticks2)
}

pub const fn zx_ticks_sub_ticks(ticks1: ZxTicks, ticks2: ZxTicks) -> ZxTicks {
    ticks1.saturating_sub(ticks2)
}

pub const fn zx_ticks_mul_int64(ticks: ZxTicks, multiplier: i64) -> ZxTicks {
    match ticks.checked_mul(multiplier) {
        Some(x) => x,
        None => {
            if (ticks > 0 && multiplier > 0) || (ticks < 0 && multiplier < 0) {
                ZX_TIME_INFINITE
            } else {
                ZX_TIME_INFINITE_PAST
            }
        }
    }
}

// Convenience wrappers

pub const fn zx_nsec(n: i64) -> ZxDuration {
    zx_duration_from_nsec(n)
}

pub const fn zx_usec(n: i64) -> ZxDuration {
    zx_duration_from_usec(n)
}

pub const fn zx_msec(n: i64) -> ZxDuration {
    zx_duration_from_msec(n)
}

pub const fn zx_sec(n: i64) -> ZxDuration {
    zx_duration_from_sec(n)
}

pub const fn zx_min(n: i64) -> ZxDuration {
    zx_duration_from_min(n)
}

pub const fn zx_hour(n: i64) -> ZxDuration {
    zx_duration_from_hour(n)
}
