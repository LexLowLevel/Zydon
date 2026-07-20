// Virtual Memory Object (VMO).
// Reference-counted, page-granular memory container.
// Supports demand paging, clone-on-write, multi-mapping.

use core::sync::atomic::{AtomicU64, Ordering};

use crate::memory::address::{PhysAddr, VirtAddr};
use crate::memory::page_table::PagePerms;
use crate::primitives::spinlock::Spinlock;

/// Unique VMO ID (monotonic).
static NEXT_VMO_ID: AtomicU64 = AtomicU64::new(1);

/// A page in the VMO's page cache.
#[derive(Clone)]
pub struct PageRef {
    /// Physical frame number.
    pub frame: usize,
    /// Refcount; drops to 1 when exclusively owned (COW resolved).
    pub refcount: AtomicU64,
}

impl PageRef {
    pub fn new(frame: usize) -> Self {
        Self {
            frame,
            refcount: AtomicU64::new(1),
        }
    }

    pub fn add_ref(&self) {
        self.refcount.fetch_add(1, Ordering::Relaxed);
    }

    pub fn release(&self) -> u64 {
        self.refcount.fetch_sub(1, Ordering::Release) - 1
    }

    pub fn is_exclusive(&self) -> bool {
        self.refcount.load(Ordering::Acquire) == 1
    }
}

/// A virtual memory object.
pub struct Vmo {
    id: u64,
    kind: VmoKind,
    /// Page cache: page index -> PageRef (sparse).
    pages: Spinlock<alloc::collections::BTreeMap<usize, PageRef>>,
    /// Size in bytes.
    size: u64,
    /// Parent VMO (if cloned).
    parent: Option<alloc::boxed::Box<Vmo>>,
    /// Byte offset in parent where this clone starts.
    clone_offset: u64,
}

enum VmoKind {
    /// Demand-paged anonymous memory.
    Anonymous,
    /// Fixed physical range (MMIO).
    Physical { base: PhysAddr },
    /// External pager backed.
    Paged,
}

impl Vmo {
    /// Create anonymous VMO.
    pub fn new_anonymous(size: u64) -> Self {
        let id = NEXT_VMO_ID.fetch_add(1, Ordering::Relaxed);
        Self {
            id,
            kind: VmoKind::Anonymous,
            pages: Spinlock::new(alloc::collections::BTreeMap::new()),
            size,
            parent: None,
            clone_offset: 0,
        }
    }

    /// Create VMO backed by physical range.
    pub fn new_physical(base: PhysAddr, size: u64) -> Self {
        let id = NEXT_VMO_ID.fetch_add(1, Ordering::Relaxed);
        Self {
            id,
            kind: VmoKind::Physical { base },
            pages: Spinlock::new(alloc::collections::BTreeMap::new()),
            size,
            parent: None,
            clone_offset: 0,
        }
    }

    pub fn id(&self) -> u64 {
        self.id
    }

    pub fn size(&self) -> u64 {
        self.size
    }

    /// Get or allocate a page at byte offset.
    /// Anonymous: allocates on first access. Cloned: falls back to
    /// parent on miss, marking parent's page COW if shared.
    pub fn get_or_alloc_page(&self, offset: u64) -> Option<PageRef> {
        let page_idx = (offset >> 12) as usize;
        let mut pages = self.pages.lock();

        if let Some(page) = pages.get(&page_idx) {
            return Some(page.clone());
        }

        // Page miss. Try parent (clone semantics).
        if let Some(ref parent) = self.parent {
            let parent_offset = self.clone_offset + offset;
            if let Some(parent_page) = parent.get_page(parent_offset) {
                // Share parent's page (COW).
                parent_page.add_ref();
                pages.insert(page_idx, parent_page.clone());
                return pages.get(&page_idx).cloned();
            }
        }

        // Allocate new frame.
        let frame = crate::memory::buddy::alloc_frame()?;
        let page_ref = PageRef::new(frame);
        pages.insert(page_idx, page_ref.clone());
        Some(page_ref)
    }

    /// Get page without allocating (None on miss).
    pub fn get_page(&self, offset: u64) -> Option<PageRef> {
        let page_idx = (offset >> 12) as usize;
        self.pages.lock().get(&page_idx).cloned()
    }

    /// Clone this VMO (copy-on-write).
    /// Shares all pages via parent pointer; pages are marked COW.
    pub fn clone_cow(&self) -> Vmo {
        let id = NEXT_VMO_ID.fetch_add(1, Ordering::Relaxed);

        // Bump refcount on existing pages (mark shared/COW).
        {
            let pages = self.pages.lock();
            for page in pages.values() {
                page.add_ref();
            }
        }

        Vmo {
            id,
            kind: VmoKind::Anonymous,
            pages: Spinlock::new(alloc::collections::BTreeMap::new()),
            size: self.size,
            parent: Some(alloc::boxed::Box::new(Vmo {
                id: self.id,
                kind: VmoKind::Anonymous,
                pages: Spinlock::new(alloc::collections::BTreeMap::new()),
                size: self.size,
                parent: self.parent.as_ref().map(|p| alloc::boxed::Box::new(p.as_ref().clone_cow())),
                clone_offset: self.clone_offset,
            })),
            clone_offset: 0,
        }
    }

    /// Handle write fault on a COW page.
    /// If exclusively owned (refcount==1), mark writable. Otherwise
    /// allocate new frame, copy, decrement old refcount.
    pub fn handle_cow_fault(&self, offset: u64) -> Option<usize> {
        let page_idx = (offset >> 12) as usize;
        let mut pages = self.pages.lock();
        let page = pages.get(&page_idx)?;

        if page.is_exclusive() {
            return Some(page.frame);
        }

        // Shared. Allocate new frame and copy.
        let new_frame = crate::memory::buddy::alloc_frame()?;
        let old_frame = page.frame;

        // Copy page content.
        // SAFETY: both frames are valid direct-map addresses.
        unsafe {
            let src = crate::memory::address::KvAddr::from_phys(
                crate::memory::address::PhysAddr::new(old_frame << 12), 0
            );
            let dst = crate::memory::address::KvAddr::from_phys(
                crate::memory::address::PhysAddr::new(new_frame << 12), 0
            );
            core::ptr::copy_nonoverlapping(
                src.raw() as *const u8,
                dst.raw() as *mut u8,
                4096,
            );
        }

        // Decrement old page refcount.
        page.release();

        // Replace with exclusive page.
        pages.insert(page_idx, PageRef::new(new_frame));
        Some(new_frame)
    }
}