// Assert macros.
// Enabled at debug levels controlled by |assert_level| build arg.

use core::panic::Location;

/// Assert failure panic (file, line, expr).
#[cold]
#[inline(never)]
#[track_caller]
pub fn assert_fail(file: &str, line: u32, expression: &str) -> ! {
    panic!("assertion failed at {}:{}: {}", file, line, expression);
}

/// Assert failure panic with message.
#[cold]
#[inline(never)]
#[track_caller]
pub fn assert_fail_msg(file: &str, line: u32, expression: &str, msg: &str) -> ! {
    panic!("assertion failed at {}:{}: {}\n{}", file, line, expression, msg);
}

/// Always-on assert.
#[macro_export]
macro_rules! zydon_assert {
    ($cond:expr) => {
        if !$cond {
            $crate::variables::assert_macro::assert_fail(file!(), line!(), stringify!($cond));
        }
    };
}

/// Always-on assert with message.
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

/// Debug-only assert.
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

/// Debug-only assert with message.
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

/// Unconditional panic macro.
#[macro_export]
macro_rules! zydon_panic {
    ($($arg:tt)*) => {
        panic!("{}", format_args!($($arg)*))
    };
}

/// Debug-only assert (body gated).
#[macro_export]
macro_rules! zydon_debug_assert_cond {
    ($cond:expr) => {
        #[cfg(debug_assertions)]
        {
            $crate::zydon_debug_assert!($cond);
        }
    };
}

/// Debug-only assert with message (body gated).
#[macro_export]
macro_rules! zydon_debug_assert_msg_cond {
    ($cond:expr, $($arg:tt)*) => {
        #[cfg(debug_assertions)]
        {
            $crate::zydon_debug_assert_msg!($cond, $($arg)*);
        }
    };
}
