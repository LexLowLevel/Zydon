// Buddy allocator for physical page frames.
// Order 0 = 1 page (4KB), order k = 2^k pages.
// Per-core freelists for orders 0..3; global freelists beyond.
// Intrusive doubly-linked lists in free frames for O(1) coalescing.

use core::sync::atomic::{AtomicUsize, Ordering};

use crate::memory::address::PhysAddr;
use crate::primitives::spinlock::Spinlock;

pub const MAX_ORDER: usize = 11;
pub const PAGE_SIZE: usize = 4096;
const PER_CORE_ORDERS: usize = 4;
const MAX_CORES: usize = 64;
const LIST_END: usize = usize::MAX;

static DIRECT_MAP_BASE: AtomicUsize = AtomicUsize::new(0);
static CORE_ID_FN: AtomicUsize = AtomicUsize::new(0);

pub fn init_direct_map_base(base: usize) {
    DIRECT_MAP_BASE.store(base, Ordering::Release);
}

pub fn init_core_id_fn(f: fn() -> usize) {
    CORE_ID_FN.store(f as usize, Ordering::Release);
}

pub static BUDDY: BuddyAllocator = BuddyAllocator::new();

pub struct BuddyAllocator {
    global: [Spinlock<FreeList>; MAX_ORDER - PER_CORE_ORDERS],
    per_core: [PerCoreFreeList; MAX_CORES],
    total_free: AtomicUsize,
}

struct PerCoreFreeList {
    lists: [Spinlock<FreeList>; PER_CORE_ORDERS],
}

struct FreeList {
    head: usize,
    count: u32,
}

impl FreeList {
    const fn new() -> Self {
        Self { head: LIST_END, count: 0 }
    }

    #[inline]
    fn is_empty(&self) -> bool {
        self.count == 0
    }
}

const EMPTY_PER_CORE: PerCoreFreeList = PerCoreFreeList {
    lists: [
        Spinlock::new(FreeList::new()),
        Spinlock::new(FreeList::new()),
        Spinlock::new(FreeList::new()),
        Spinlock::new(FreeList::new()),
    ],
};

impl BuddyAllocator {
    pub const fn new() -> Self {
        macro_rules! init_global {
            ($($i:expr),*) => {
                [$(Spinlock::new(FreeList::new()),)*]
            };
        }
        Self {
            global: init_global!(0,1,2,3,4,5,6),
            per_core: [
                EMPTY_PER_CORE, EMPTY_PER_CORE, EMPTY_PER_CORE, EMPTY_PER_CORE,
                EMPTY_PER_CORE, EMPTY_PER_CORE, EMPTY_PER_CORE, EMPTY_PER_CORE,
                EMPTY_PER_CORE, EMPTY_PER_CORE, EMPTY_PER_CORE, EMPTY_PER_CORE,
                EMPTY_PER_CORE, EMPTY_PER_CORE, EMPTY_PER_CORE, EMPTY_PER_CORE,
                EMPTY_PER_CORE, EMPTY_PER_CORE, EMPTY_PER_CORE, EMPTY_PER_CORE,
                EMPTY_PER_CORE, EMPTY_PER_CORE, EMPTY_PER_CORE, EMPTY_PER_CORE,
                EMPTY_PER_CORE, EMPTY_PER_CORE, EMPTY_PER_CORE, EMPTY_PER_CORE,
                EMPTY_PER_CORE, EMPTY_PER_CORE, EMPTY_PER_CORE, EMPTY_PER_CORE,
                EMPTY_PER_CORE, EMPTY_PER_CORE, EMPTY_PER_CORE, EMPTY_PER_CORE,
                EMPTY_PER_CORE, EMPTY_PER_CORE, EMPTY_PER_CORE, EMPTY_PER_CORE,
                EMPTY_PER_CORE, EMPTY_PER_CORE, EMPTY_PER_CORE, EMPTY_PER_CORE,
                EMPTY_PER_CORE, EMPTY_PER_CORE, EMPTY_PER_CORE, EMPTY_PER_CORE,
                EMPTY_PER_CORE, EMPTY_PER_CORE, EMPTY_PER_CORE, EMPTY_PER_CORE,
                EMPTY_PER_CORE, EMPTY_PER_CORE, EMPTY_PER_CORE, EMPTY_PER_CORE,
                EMPTY_PER_CORE, EMPTY_PER_CORE, EMPTY_PER_CORE, EMPTY_PER_CORE,
                EMPTY_PER_CORE, EMPTY_PER_CORE, EMPTY_PER_CORE, EMPTY_PER_CORE,
            ],
            total_free: AtomicUsize::new(0),
        }
    }

    pub fn alloc_frame(&self) -> Option<usize> {
        self.alloc_pages(0)
    }

    pub fn alloc_pages(&self, order: usize) -> Option<usize> {
        if order >= MAX_ORDER {
            return None;
        }
        let frame = self.alloc_iter(order)?;
        self.total_free.fetch_sub(1 << order, Ordering::Relaxed);
        Some(frame)
    }

    fn alloc_iter(&self, order: usize) -> Option<usize> {
        let mut high_order = None;
        for o in order..MAX_ORDER {
            if self.try_pop(o).is_some() {
                high_order = Some(o);
                break;
            }
        }
        let mut high_order = high_order?;
        let mut frame = self.take_popped(high_order);
        while high_order > order {
            high_order -= 1;
            let buddy = frame + (1 << high_order);
            self.free_to_order(buddy, high_order);
        }
        Some(frame)
    }

    fn try_pop(&self, order: usize) -> Option<()> {
        if order >= MAX_ORDER {
            return None;
        }
        if order < PER_CORE_ORDERS {
            let core_id = current_core_id();
            let mut list = self.per_core[core_id].lists[order].lock();
            if list.is_empty() {
                return None;
            }
            let frame = list.head;
            list.head = self.read_next(frame);
            if list.head != LIST_END {
                self.write_prev(list.head, LIST_END);
            }
            list.count -= 1;
            LAST_POP.store(frame, Ordering::Relaxed);
            Some(())
        } else {
            let mut list = self.global[order - PER_CORE_ORDERS].lock();
            if list.is_empty() {
                return None;
            }
            let frame = list.head;
            list.head = self.read_next(frame);
            if list.head != LIST_END {
                self.write_prev(list.head, LIST_END);
            }
            list.count -= 1;
            LAST_POP.store(frame, Ordering::Relaxed);
            Some(())
        }
    }

    #[inline]
    fn take_popped(&self, _order: usize) -> usize {
        LAST_POP.load(Ordering::Relaxed)
    }

    pub fn free_pages(&self, frame: usize, order: usize) {
        self.total_free.fetch_add(1 << order, Ordering::Relaxed);
        self.coalesce_iter(frame, order);
    }

    fn coalesce_iter(&self, mut frame: usize, mut order: usize) {
        while order < MAX_ORDER - 1 {
            let buddy = frame ^ (1 << order);
            if self.try_remove_buddy(buddy, order) {
                frame = frame.min(buddy);
                order += 1;
            } else {
                break;
            }
        }
        self.free_to_order(frame, order);
    }

    pub fn free_frame(&self, frame: usize) {
        self.free_pages(frame, 0);
    }

    pub fn total_free(&self) -> usize {
        self.total_free.load(Ordering::Relaxed)
    }

    fn free_to_order(&self, frame: usize, order: usize) {
        unsafe {
            if order < PER_CORE_ORDERS {
                let core_id = current_core_id();
                let mut list = self.per_core[core_id].lists[order].lock();
                self.write_prev(frame, LIST_END);
                self.write_next(frame, list.head);
                if list.head != LIST_END {
                    self.write_prev(list.head, frame);
                }
                list.head = frame;
                list.count += 1;
            } else {
                let mut list = self.global[order - PER_CORE_ORDERS].lock();
                self.write_prev(frame, LIST_END);
                self.write_next(frame, list.head);
                if list.head != LIST_END {
                    self.write_prev(list.head, frame);
                }
                list.head = frame;
                list.count += 1;
            }
        }
    }

    fn try_remove_buddy(&self, buddy: usize, order: usize) -> bool {
        if order < PER_CORE_ORDERS {
            let core_id = current_core_id();
            let mut list = self.per_core[core_id].lists[order].lock();
            self.find_and_remove(&mut list, buddy)
        } else {
            let mut list = self.global[order - PER_CORE_ORDERS].lock();
            self.find_and_remove(&mut list, buddy)
        }
    }

    fn find_and_remove(&self, list: &mut FreeList, frame: usize) -> bool {
        let mut cur = list.head;
        while cur != LIST_END {
            if cur == frame {
                let prev = self.read_prev(frame);
                let next = self.read_next(frame);
                if list.head == frame {
                    list.head = next;
                }
                if prev != LIST_END {
                    self.write_next(prev, next);
                }
                if next != LIST_END {
                    self.write_prev(next, prev);
                }
                list.count -= 1;
                return true;
            }
            cur = self.read_next(cur);
        }
        false
    }

    #[inline]
    fn read_prev(&self, frame: usize) -> usize {
        let kv = frame_to_kv(frame);
        unsafe { *(kv as *const usize) }
    }

    #[inline]
    fn read_next(&self, frame: usize) -> usize {
        let kv = frame_to_kv(frame);
        unsafe { *((kv + core::mem::size_of::<usize>()) as *const usize) }
    }

    #[inline]
    fn write_prev(&self, frame: usize, prev: usize) {
        let kv = frame_to_kv(frame);
        unsafe { *(kv as *mut usize) = prev; }
    }

    #[inline]
    fn write_next(&self, frame: usize, next: usize) {
        let kv = frame_to_kv(frame);
        unsafe { *((kv + core::mem::size_of::<usize>()) as *mut usize) = next; }
    }
}

static LAST_POP: AtomicUsize = AtomicUsize::new(0);

#[inline]
fn frame_to_kv(frame: usize) -> usize {
    let base = DIRECT_MAP_BASE.load(Ordering::Relaxed);
    base + frame * PAGE_SIZE
}

#[inline]
pub fn current_core_id_pub() -> usize {
    current_core_id()
}

#[inline]
fn current_core_id() -> usize {
    let f = CORE_ID_FN.load(Ordering::Acquire);
    if f == 0 {
        return 0;
    }
    let g: fn() -> usize = unsafe { core::mem::transmute(f) };
    g()
}
