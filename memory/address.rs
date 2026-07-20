// Strongly-typed virtual and physical addresses.
// Newtypes prevent mixing address spaces at the type level.
// All arithmetic is checked; overflow returns None.

use core::fmt;

/// A virtual address in a process's address space.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct VirtAddr(pub usize);

/// A physical address in system memory.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct PhysAddr(pub usize);

/// A kernel virtual address (direct-mapped region).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct KvAddr(pub usize);

impl VirtAddr {
    pub const NULL: VirtAddr = VirtAddr(0);

    pub const fn new(addr: usize) -> Self {
        Self(addr)
    }

    pub const fn raw(&self) -> usize {
        self.0
    }

    /// Offset by `bytes`. None on overflow.
    pub const fn offset(&self, bytes: usize) -> Option<VirtAddr> {
        match self.0.checked_add(bytes) {
            Some(addr) => Some(VirtAddr(addr)),
            None => None,
        }
    }

    /// Align up to ALIGN (must be power of 2).
    pub const fn align_up<const ALIGN: usize>(&self) -> VirtAddr {
        VirtAddr((self.0 + ALIGN - 1) & !(ALIGN - 1))
    }

    /// Align down to ALIGN (must be power of 2).
    pub const fn align_down<const ALIGN: usize>(&self) -> VirtAddr {
        VirtAddr(self.0 & !(ALIGN - 1))
    }

    /// True if aligned to ALIGN.
    pub const fn is_aligned<const ALIGN: usize>(&self) -> bool {
        self.0 % ALIGN == 0
    }

    /// Page index (addr >> 12).
    pub const fn page_index(&self) -> usize {
        self.0 >> 12
    }

    /// Offset within page.
    pub const fn page_offset(&self) -> usize {
        self.0 & 0xFFF
    }
}

impl PhysAddr {
    pub const fn new(addr: usize) -> Self {
        Self(addr)
    }

    pub const fn raw(&self) -> usize {
        self.0
    }

    pub const fn offset(&self, bytes: usize) -> Option<PhysAddr> {
        match self.0.checked_add(bytes) {
            Some(addr) => Some(PhysAddr(addr)),
            None => None,
        }
    }

    pub const fn align_up<const ALIGN: usize>(&self) -> PhysAddr {
        PhysAddr((self.0 + ALIGN - 1) & !(ALIGN - 1))
    }

    pub const fn align_down<const ALIGN: usize>(&self) -> PhysAddr {
        PhysAddr(self.0 & !(ALIGN - 1))
    }

    pub const fn is_aligned<const ALIGN: usize>(&self) -> bool {
        self.0 % ALIGN == 0
    }

    /// Frame number (addr >> 12).
    pub const fn frame_number(&self) -> usize {
        self.0 >> 12
    }
}

impl KvAddr {
    pub const fn new(addr: usize) -> Self {
        Self(addr)
    }

    pub const fn raw(&self) -> usize {
        self.0
    }

    /// Kernel virtual to physical (direct map offset).
    pub fn to_phys(&self, direct_map_base: usize) -> PhysAddr {
        PhysAddr(self.0 - direct_map_base)
    }

    /// Physical to kernel virtual (direct map offset).
    pub fn from_phys(pa: PhysAddr, direct_map_base: usize) -> KvAddr {
        KvAddr(pa.0 + direct_map_base)
    }
}

impl fmt::Debug for VirtAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "VirtAddr({:#x})", self.0)
    }
}

impl fmt::Debug for PhysAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PhysAddr({:#x})", self.0)
    }
}

impl fmt::Debug for KvAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "KvAddr({:#x})", self.0)
    }
}

impl fmt::Display for VirtAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:#x}", self.0)
    }
}

impl fmt::Display for PhysAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:#x}", self.0)
    }
}
