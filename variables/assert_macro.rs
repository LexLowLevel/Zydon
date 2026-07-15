// Assert macros.
// See build argument |assert_level| for which are enabled at which debug levels.

use core::panic::Location;

/// Panic with file, line, and expression on assert failure.
#[cold]
#[inline(never)]
#[track_caller]
pub fn assert_fail(file: &str, line: u32, expression: &str) -> ! {
    panic!("assertion failed at {}:{}: {}", file, line, expression);
}

/// Panic with file, line, expression, and custom message.
#[cold]
#[inline(never)]
#[track_caller]
pub fn assert_fail_msg(file: &str, line: u32, expression: &str, msg: &str) -> ! {
    panic!("assertion failed at {}:{}: {}\n{}", file, line, expression, msg);
}

/// Assert that x is true. Always enabled.
#[macro_export]
macro_rules! zydon_assert {
    ($cond:expr) => {
        if !$cond {
            $crate::variables::assert_macro::assert_fail(file!(), line!(), stringify!($cond));
        }
    };
}

/// Assert with custom message. Always enabled.
#[macro_export]
macro_rules! zydon_assert_msg {
    ($cond:expr, $($arg:tt)*) => {
        if !$cond {
            $crate::variables::assert_macro::assert_fail_msg(
                file!(), line!(), stringify!($cond), &format_args!($($arg)*).to_string()
            );
        }
    };
}

/// Debug assert. Only enabled in debug builds.
#[macro_export]
macro_rules! zydon_debug_assert {
    ($cond:expr) => {
        #[cfg(debug_assertions)]
        {
            if !$cond {
                $crate::variables::assert_macro::assert_fail(file!(), line!(), stringify!($cond));
            }
        }
    };
}

/// Debug assert with custom message. Only enabled in debug builds.
#[macro_export]
macro_rules! zydon_debug_assert_msg {
    ($cond:expr, $($arg:tt)*) => {
        #[cfg(debug_assertions)]
        {
            if !$cond {
                $crate::variables::assert_macro::assert_fail_msg(
                    file!(), line!(), stringify!($cond), &format_args!($($arg)*).to_string()
                );
            }
        }
    };
}

/// Unconditional panic.
#[macro_export]
macro_rules! zydon_panic {
    ($($arg:tt)*) => {
        panic!("{}", format_args!($($arg)*))
    };
}

/// Conditional debug assert (body only emitted in debug builds).
#[macro_export]
macro_rules! zydon_debug_assert_cond {
    ($cond:expr) => {
        #[cfg(debug_assertions)]
        {
            $crate::zydon_debug_assert!($cond);
        }
    };
}

/// Conditional debug assert with message (body only emitted in debug builds).
#[macro_export]
macro_rules! zydon_debug_assert_msg_cond {
    ($cond:expr, $($arg:tt)*) => {
        #[cfg(debug_assertions)]
        {
            $crate::zydon_debug_assert_msg!($cond, $($arg)*);
        }
    };
}
