// Kernel slab allocator.
// Per-core size-class caches for sub-page allocations.
// Steals batches from global partial lists on local exhaustion.
// Size classes: 16, 32, 64, 128, 256, 512, 1024, 2048 bytes.

use crate::memory::buddy::{self, PAGE_SIZE};
use crate::primitives::spinlock::Spinlock;

const NUM_SIZE_CLASSES: usize = 8;
const SIZE_CLASSES: [usize; NUM_SIZE_CLASSES] = [16, 32, 64, 128, 256, 512, 1024, 2048];

/// Global slab allocator instance.
pub static SLAB: SlabAllocator = SlabAllocator::new();

pub struct SlabAllocator {
    global: [Spinlock<SlabCache>; NUM_SIZE_CLASSES],
    per_core: [PerCoreSlab; MAX_CORES],
}

const MAX_CORES: usize = 64;

struct PerCoreSlab {
    caches: [Spinlock<SlabCache>; NUM_SIZE_CLASSES],
}

struct SlabCache {
    free_head: *mut u8,
    free_count: u32,
    partial_slabs: *mut SlabPage,
}

#[repr(C)]
struct SlabPage {
    next: *mut SlabPage,
    size_class: usize,
    free_count: u32,
    free_head: *mut u8,
}

unsafe impl Send for SlabCache {}
unsafe impl Sync for SlabCache {}

const EMPTY_CACHE: SlabCache = SlabCache {
    free_head: core::ptr::null_mut(),
    free_count: 0,
    partial_slabs: core::ptr::null_mut(),
};

const EMPTY_PER_CORE_SLAB: PerCoreSlab = PerCoreSlab {
    caches: [
        Spinlock::new(EMPTY_CACHE),
        Spinlock::new(EMPTY_CACHE),
        Spinlock::new(EMPTY_CACHE),
        Spinlock::new(EMPTY_CACHE),
        Spinlock::new(EMPTY_CACHE),
        Spinlock::new(EMPTY_CACHE),
        Spinlock::new(EMPTY_CACHE),
        Spinlock::new(EMPTY_CACHE),
    ],
};

impl SlabAllocator {
    pub const fn new() -> Self {
        Self {
            global: [
                Spinlock::new(EMPTY_CACHE),
                Spinlock::new(EMPTY_CACHE),
                Spinlock::new(EMPTY_CACHE),
                Spinlock::new(EMPTY_CACHE),
                Spinlock::new(EMPTY_CACHE),
                Spinlock::new(EMPTY_CACHE),
                Spinlock::new(EMPTY_CACHE),
                Spinlock::new(EMPTY_CACHE),
            ],
            per_core: [
                EMPTY_PER_CORE_SLAB, EMPTY_PER_CORE_SLAB, EMPTY_PER_CORE_SLAB, EMPTY_PER_CORE_SLAB,
                EMPTY_PER_CORE_SLAB, EMPTY_PER_CORE_SLAB, EMPTY_PER_CORE_SLAB, EMPTY_PER_CORE_SLAB,
                EMPTY_PER_CORE_SLAB, EMPTY_PER_CORE_SLAB, EMPTY_PER_CORE_SLAB, EMPTY_PER_CORE_SLAB,
                EMPTY_PER_CORE_SLAB, EMPTY_PER_CORE_SLAB, EMPTY_PER_CORE_SLAB, EMPTY_PER_CORE_SLAB,
                EMPTY_PER_CORE_SLAB, EMPTY_PER_CORE_SLAB, EMPTY_PER_CORE_SLAB, EMPTY_PER_CORE_SLAB,
                EMPTY_PER_CORE_SLAB, EMPTY_PER_CORE_SLAB, EMPTY_PER_CORE_SLAB, EMPTY_PER_CORE_SLAB,
                EMPTY_PER_CORE_SLAB, EMPTY_PER_CORE_SLAB, EMPTY_PER_CORE_SLAB, EMPTY_PER_CORE_SLAB,
                EMPTY_PER_CORE_SLAB, EMPTY_PER_CORE_SLAB, EMPTY_PER_CORE_SLAB, EMPTY_PER_CORE_SLAB,
                EMPTY_PER_CORE_SLAB, EMPTY_PER_CORE_SLAB, EMPTY_PER_CORE_SLAB, EMPTY_PER_CORE_SLAB,
                EMPTY_PER_CORE_SLAB, EMPTY_PER_CORE_SLAB, EMPTY_PER_CORE_SLAB, EMPTY_PER_CORE_SLAB,
                EMPTY_PER_CORE_SLAB, EMPTY_PER_CORE_SLAB, EMPTY_PER_CORE_SLAB, EMPTY_PER_CORE_SLAB,
                EMPTY_PER_CORE_SLAB, EMPTY_PER_CORE_SLAB, EMPTY_PER_CORE_SLAB, EMPTY_PER_CORE_SLAB,
                EMPTY_PER_CORE_SLAB, EMPTY_PER_CORE_SLAB, EMPTY_PER_CORE_SLAB, EMPTY_PER_CORE_SLAB,
                EMPTY_PER_CORE_SLAB, EMPTY_PER_CORE_SLAB, EMPTY_PER_CORE_SLAB, EMPTY_PER_CORE_SLAB,
                EMPTY_PER_CORE_SLAB, EMPTY_PER_CORE_SLAB, EMPTY_PER_CORE_SLAB, EMPTY_PER_CORE_SLAB,
                EMPTY_PER_CORE_SLAB, EMPTY_PER_CORE_SLAB, EMPTY_PER_CORE_SLAB, EMPTY_PER_CORE_SLAB,
            ],
        }
    }

    pub fn alloc(&self, size: usize, align: usize) -> *mut u8 {
        let size_class = self.size_class_for(size.max(align));
        if size_class >= NUM_SIZE_CLASSES {
            let order = ((size + PAGE_SIZE - 1) / PAGE_SIZE).next_power_of_two().trailing_zeros() as usize;
            return match buddy::BUDDY.alloc_pages(order) {
                Some(frame) => (frame * PAGE_SIZE) as *mut u8,
                None => core::ptr::null_mut(),
            };
        }

        let core_id = current_core_id();
        let mut cache = self.per_core[core_id].caches[size_class].lock();

        if cache.free_count > 0 {
            let ptr = cache.free_head;
            cache.free_head = unsafe { *(ptr as *const *mut u8) };
            cache.free_count -= 1;
            return ptr;
        }

        drop(cache);
        let mut global = self.global[size_class].lock();
        if global.free_count > 0 {
            let ptr = global.free_head;
            global.free_head = unsafe { *(ptr as *const *mut u8) };
            global.free_count -= 1;
            drop(global);

            self.refill_local(core_id, size_class);
            return ptr;
        }

        drop(global);
        let frame = match buddy::BUDDY.alloc_frame() {
            Some(f) => f,
            None => return core::ptr::null_mut(),
        };
        let page = (frame * PAGE_SIZE) as *mut u8;

        let obj_size = SIZE_CLASSES[size_class];
        let header_size = core::mem::size_of::<SlabPage>();
        let start = (page as usize + header_size + obj_size - 1) & !(obj_size - 1);
        let end = (page as usize) + PAGE_SIZE;
        let num_objects = (end - start) / obj_size;

        let mut ptr = start as *mut u8;
        for i in 0..num_objects - 1 {
            let next = (start + (i + 1) * obj_size) as *mut u8;
            unsafe { *(ptr as *mut *mut u8) = next; }
            ptr = next;
        }
        unsafe { *(ptr as *mut *mut u8) = core::ptr::null_mut(); }

        let slab_page = page as *mut SlabPage;
        unsafe {
            (*slab_page).next = core::ptr::null_mut();
            (*slab_page).size_class = size_class;
            (*slab_page).free_count = num_objects as u32 - 1;
            (*slab_page).free_head = (start + obj_size) as *mut u8;
        }

        let result = start as *mut u8;
        let mut cache = self.per_core[core_id].caches[size_class].lock();
        cache.free_head = (start + obj_size) as *mut u8;
        cache.free_count = num_objects as u32 - 1;
        result
    }

    pub fn dealloc(&self, ptr: *mut u8, size: usize, align: usize) {
        let size_class = self.size_class_for(size.max(align));
        if size_class >= NUM_SIZE_CLASSES {
            let order = ((size + PAGE_SIZE - 1) / PAGE_SIZE).next_power_of_two().trailing_zeros() as usize;
            buddy::BUDDY.free_pages(ptr as usize / PAGE_SIZE, order);
            return;
        }

        let core_id = current_core_id();
        let mut cache = self.per_core[core_id].caches[size_class].lock();

        unsafe { *(ptr as *mut *mut u8) = cache.free_head; }
        cache.free_head = ptr;
        cache.free_count += 1;
    }

    #[inline]
    fn size_class_for(&self, size: usize) -> usize {
        for (i, &sc) in SIZE_CLASSES.iter().enumerate() {
            if size <= sc {
                return i;
            }
        }
        NUM_SIZE_CLASSES
    }

    fn refill_local(&self, core_id: usize, size_class: usize) {
        let mut global = self.global[size_class].lock();
        let mut local = self.per_core[core_id].caches[size_class].lock();

        let steal_count = global.free_count.min(16);
        if steal_count == 0 {
            return;
        }

        local.free_head = global.free_head;
        local.free_count = steal_count;

        let mut ptr = global.free_head;
        for _ in 0..steal_count {
            ptr = unsafe { *(ptr as *const *mut u8) };
        }
        global.free_head = ptr;
        global.free_count -= steal_count;
    }
}

#[inline]
fn current_core_id() -> usize {
    buddy::current_core_id_pub()
}
