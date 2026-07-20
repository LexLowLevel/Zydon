// Capability rights bitflags.
// Rights are monotonically lossy on handle duplication.

use core::fmt;

/// Capability rights bitflags.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rights(u32);

impl Rights {
    pub const READ: Rights = Rights(1 << 0);
    pub const WRITE: Rights = Rights(1 << 1);
    pub const MAP: Rights = Rights(1 << 2);
    pub const EXECUTE: Rights = Rights(1 << 3);
    pub const DUPLICATE: Rights = Rights(1 << 4);
    pub const TRANSFER: Rights = Rights(1 << 5);
    pub const DESTROY: Rights = Rights(1 << 6);
    pub const INSPECT: Rights = Rights(1 << 7);
    pub const SIGNAL: Rights = Rights(1 << 8);
    pub const SIGNAL_PEER: Rights = Rights(1 << 9);
    pub const ENUMERATE: Rights = Rights(1 << 10);
    pub const MODIFY: Rights = Rights(1 << 11);
    pub const SUSPEND: Rights = Rights(1 << 12);
    pub const WAIT: Rights = Rights(1 << 13);
    /// All rights. Used for initial handle to a newly created object.
    pub const ALL: Rights = Rights((1 << 14) - 1);

    pub const fn from_bits(bits: u32) -> Self {
        Rights(bits)
    }

    pub const fn bits(&self) -> u32 {
        self.0
    }

    /// Contains all of other?
    pub const fn contains(&self, other: Rights) -> bool {
        (self.0 & other.0) == other.0
    }

    /// Contains any of other?
    pub const fn intersects(&self, other: Rights) -> bool {
        (self.0 & other.0) != 0
    }

    /// Intersection of self and other.
    pub const fn intersection(&self, other: Rights) -> Rights {
        Rights(self.0 & other.0)
    }

    /// Union of self and other.
    pub const fn union(&self, other: Rights) -> Rights {
        Rights(self.0 | other.0)
    }

    /// Self with other bits removed.
    pub const fn difference(&self, other: Rights) -> Rights {
        Rights(self.0 & !other.0)
    }

    /// Downgrade: intersect with allowed rights.
    pub const fn downgrade(&self, allowed: Rights) -> Rights {
        self.intersection(allowed)
    }

    pub const fn is_empty(&self) -> bool {
        self.0 == 0
    }
}

impl fmt::Debug for Rights {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut first = true;
        macro_rules! flag {
            ($name:expr, $val:expr) => {
                if self.contains($val) {
                    if !first {
                        write!(f, " | ")?;
                    }
                    write!(f, $name)?;
                    first = false;
                }
            };
        }
        flag!("READ", Self::READ);
        flag!("WRITE", Self::WRITE);
        flag!("MAP", Self::MAP);
        flag!("EXECUTE", Self::EXECUTE);
        flag!("DUPLICATE", Self::DUPLICATE);
        flag!("TRANSFER", Self::TRANSFER);
        flag!("DESTROY", Self::DESTROY);
        flag!("INSPECT", Self::INSPECT);
        flag!("SIGNAL", Self::SIGNAL);
        flag!("SIGNAL_PEER", Self::SIGNAL_PEER);
        flag!("ENUMERATE", Self::ENUMERATE);
        flag!("MODIFY", Self::MODIFY);
        flag!("SUSPEND", Self::SUSPEND);
        flag!("WAIT", Self::WAIT);
        if first {
            write!(f, "(none)")?;
        }
        Ok(())
    }
}

impl fmt::Display for Rights {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

impl core::ops::BitOr for Rights {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Rights(self.0 | rhs.0)
    }
}

impl core::ops::BitAnd for Rights {
    type Output = Self;
    fn bitand(self, rhs: Self) -> Self {
        Rights(self.0 & rhs.0)
    }
}

impl core::ops::BitOrAssign for Rights {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}
