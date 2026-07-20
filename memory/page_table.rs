// Abstract page table interface.
// Supports x86-64 4-level, AArch64 4KB granule, and radix tree.

use crate::memory::address::{VirtAddr, PhysAddr};

/// Page permissions.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct PagePerms(u8);

impl PagePerms {
    pub const READ: PagePerms = PagePerms(1 << 0);
    pub const WRITE: PagePerms = PagePerms(1 << 1);
    pub const EXECUTE: PagePerms = PagePerms(1 << 2);
    pub const USER: PagePerms = PagePerms(1 << 3);

    pub const fn contains(&self, other: PagePerms) -> bool {
        (self.0 & other.0) == other.0
    }

    pub const fn union(&self, other: PagePerms) -> PagePerms {
        PagePerms(self.0 | other.0)
    }
}

/// Virtual -> physical mapping with permissions.
#[derive(Clone, Copy)]
pub struct PageMapping {
    pub va: VirtAddr,
    pub pa: PhysAddr,
    pub perms: PagePerms,
    pub size: PageSize,
}

/// Page size variants.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PageSize {
    /// 4 KB.
    Small,
    /// 2 MB.
    Large,
    /// 1 GB.
    Huge,
}

impl PageSize {
    pub const fn bytes(&self) -> usize {
        match self {
            PageSize::Small => 4096,
            PageSize::Large => 2 * 1024 * 1024,
            PageSize::Huge => 1024 * 1024 * 1024,
        }
    }
}

/// Error from a page table operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageTableError {
    /// Virtual address already mapped.
    AlreadyMapped,
    /// Virtual address not mapped.
    NotMapped,
    /// Alignment violation.
    NotAligned,
    /// OOM for page table structures.
    NoMemory,
}

/// Abstract page table operations. Must be Send.
pub trait PageTable: Send {
    /// Map a virtual page to a physical frame.
    fn map(
        &mut self,
        va: VirtAddr,
        pa: PhysAddr,
        size: PageSize,
        perms: PagePerms,
    ) -> Result<(), PageTableError>;

    /// Unmap a virtual page. Returns the mapped physical frame.
    fn unmap(&mut self, va: VirtAddr) -> Result<PhysAddr, PageTableError>;

    /// Query mapping for a virtual address.
    fn query(&self, va: VirtAddr) -> Option<PageMapping>;

    /// Translate virtual to physical address.
    fn translate(&self, va: VirtAddr) -> Option<PhysAddr> {
        self.query(va).map(|m| {
            let offset = va.raw() - m.va.raw();
            PhysAddr(m.pa.raw() + offset)
        })
    }

    /// Change permissions on existing mapping.
    fn protect(
        &mut self,
        va: VirtAddr,
        perms: PagePerms,
    ) -> Result<(), PageTableError>;

    /// Flush TLB for a specific address.
    fn flush_tlb(&self, va: VirtAddr);

    /// Flush entire TLB.
    fn flush_tlb_all(&self);
}

/// x86-64 4-level page table.
/// PML4 -> PDPT -> PD -> PT. 4KB/2MB/1GB pages.
#[cfg(target_arch = "x86_64")]
pub struct X86PageTable {
    /// Physical address of PML4 root.
    pml4_phys: PhysAddr,
    /// Kernel virtual address of PML4 (direct-mapped).
    pml4: *mut u64,
    /// ASID for tagged TLB flushes.
    asid: u32,
}

#[cfg(target_arch = "x86_64")]
impl X86PageTable {
    pub fn new(pml4_phys: PhysAddr, pml4_kv: *mut u64, asid: u32) -> Self {
        Self { pml4_phys, pml4: pml4_kv, asid }
    }

    pub fn asid(&self) -> u32 {
        self.asid
    }

    pub fn root_phys(&self) -> PhysAddr {
        self.pml4_phys
    }
}

// x86-64 entry flags.
#[cfg(target_arch = "x86_64")]
mod x86_flags {
    pub const PRESENT: u64 = 1 << 0;
    pub const WRITABLE: u64 = 1 << 1;
    pub const USER: u64 = 1 << 2;
    pub const WRITE_THROUGH: u64 = 1 << 3;
    pub const CACHE_DISABLE: u64 = 1 << 4;
    pub const ACCESSED: u64 = 1 << 5;
    pub const DIRTY: u64 = 1 << 6;
    pub const HUGE_PAGE: u64 = 1 << 7;
    pub const NO_EXECUTE: u64 = 1 << 63;

    pub fn perms_to_flags(perms: super::PagePerms) -> u64 {
        let mut flags = PRESENT;
        if perms.contains(super::PagePerms::WRITE) {
            flags |= WRITABLE;
        }
        if perms.contains(super::PagePerms::USER) {
            flags |= USER;
        }
        if !perms.contains(super::PagePerms::EXECUTE) {
            flags |= NO_EXECUTE;
        }
        flags
    }
}