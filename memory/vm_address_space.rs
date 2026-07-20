// Per-process virtual memory address space.
// Maps virtual ranges to VMOs via BTreeMap for O(log n) lookup.
// Page fault handler resolves address then delegates to VMO.

use alloc::collections::BTreeMap;

use crate::memory::address::VirtAddr;
use crate::memory::page_table::{PagePerms, PageSize, PageTableError};
use crate::memory::vmo::Vmo;

/// A mapping in the address space.
#[derive(Clone)]
pub struct VmaEntry {
    /// VMO backing this region.
    pub vmo_id: u64,
    /// Byte offset within the VMO.
    pub vmo_offset: u64,
    /// Mapping size in bytes.
    pub size: u64,
    /// Permissions.
    pub perms: PagePerms,
}

/// Per-process virtual memory address space.
pub struct VmAddressSpace {
    /// Virtual address -> VMA entry, sorted by address.
    vmas: BTreeMap<VirtAddr, VmaEntry>,
    /// Address space ID for TLB tagging.
    asid: u32,
}

impl VmAddressSpace {
    pub fn new(asid: u32) -> Self {
        Self {
            vmas: BTreeMap::new(),
            asid,
        }
    }

    pub fn asid(&self) -> u32 {
        self.asid
    }

    /// Map a VMO region into the address space.
    pub fn map(
        &mut self,
        va: VirtAddr,
        vmo_id: u64,
        vmo_offset: u64,
        size: u64,
        perms: PagePerms,
    ) -> Result<(), VmError> {
        if !va.is_aligned::<4096>() {
            return Err(VmError::NotAligned);
        }

        // Check overlap with existing mappings.
        let end = va.raw() as u64 + size;
        if let Some((_, existing)) = self.vmas.range(..va).next_back() {
            let existing_end = existing.vmo_offset + existing.size;
            if (va.raw() as u64) < existing_end {
                return Err(VmError::Overlap);
            }
        }
        if let Some((next_va, _)) = self.vmas.range(va..).next() {
            if end > next_va.raw() as u64 {
                return Err(VmError::Overlap);
            }
        }

        self.vmas.insert(va, VmaEntry {
            vmo_id,
            vmo_offset,
            size,
            perms,
        });

        Ok(())
    }

    /// Unmap a virtual address range.
    pub fn unmap(&mut self, va: VirtAddr, size: u64) -> Result<(), VmError> {
        if !va.is_aligned::<4096>() {
            return Err(VmError::NotAligned);
        }

        self.vmas.remove(&va).ok_or(VmError::NotFound)?;
        Ok(())
    }

    /// Look up VMA for a faulting address.
    pub fn lookup(&self, va: VirtAddr) -> Option<&VmaEntry> {
        // Find mapping whose start <= va.
        self.vmas.range(..=va).next_back().and_then(|(start, entry)| {
            let end = start.raw() as u64 + entry.size;
            if (va.raw() as u64) < end {
                Some(entry)
            } else {
                None
            }
        })
    }

    /// Handle a page fault at the given virtual address.
    /// Resolves the VMO, allocates a page, installs the mapping.
    pub fn handle_fault(
        &self,
        va: VirtAddr,
        vmos: &alloc::collections::BTreeMap<u64, Vmo>,
        page_table: &mut dyn crate::memory::page_table::PageTable,
    ) -> Result<(), VmError> {
        let entry = self.lookup(va).ok_or(VmError::NotFound)?;
        let vmo = vmos.get(&entry.vmo_id).ok_or(VmError::VmoNotFound)?;

        let offset = entry.vmo_offset + (va.raw() as u64 - self.vma_start_for(va).ok_or(VmError::NotFound)? as u64);

        // Get or allocate page from VMO.
        // SAFETY: Vmo uses an internal Spinlock for page allocation.
        let page = vmo.get_or_alloc_page(offset).ok_or(VmError::NoMemory)?;

        let pa = crate::memory::address::PhysAddr::new(page.frame << 12);
        page_table.map(va, pa, PageSize::Small, entry.perms)
            .map_err(|_| VmError::MapFailed)?;

        Ok(())
    }

    fn vma_start_for(&self, va: VirtAddr) -> Option<usize> {
        self.vmas.range(..=va).next_back().map(|(start, _)| start.raw())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VmError {
    NotAligned,
    NotFound,
    Overlap,
    VmoNotFound,
    NoMemory,
    MapFailed,
}