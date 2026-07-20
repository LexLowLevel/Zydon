// Hierarchical timing wheel.
//
// 4 levels, 256 slots each: level 0 has 1-tick granularity, level 1
// has 256-tick, level 2 has 65536-tick, level 3 has 16777216-tick.
// add_timer is O(1); tick is O(1) amortized via cascading.
// Used for quantum preemption, IPC/futex timeouts, housekeeping.

use core::sync::atomic::{AtomicU64, Ordering};

use crate::primitives::spinlock::Spinlock;

const LEVELS: usize = 4;
const SLOTS_PER_LEVEL: usize = 256;

/// Timer callback.
pub type TimerCallback = fn(*mut ());

/// Timer entry in the wheel.
struct TimerEntry {
    callback: TimerCallback,
    data: *mut (),
    /// Absolute tick at which this timer fires.
    deadline: u64,
    next: *mut TimerEntry,
}

// SAFETY: TimerEntry is only accessed through the Spinlock.
unsafe impl Send for TimerEntry {}
unsafe impl Sync for TimerEntry {}

/// Per-core timing wheel.
pub struct TimingWheel {
    tick_count: AtomicU64,
    levels: [Level; LEVELS],
}

struct Level {
    slots: [Spinlock<Slot>; SLOTS_PER_LEVEL],
}

struct Slot {
    head: *mut TimerEntry,
}

// SAFETY: accessed through Spinlock.
unsafe impl Send for Slot {}
unsafe impl Sync for Slot {}

impl TimingWheel {
    pub const fn new() -> Self {
        macro_rules! init_slots {
            ($($i:expr),*) => {
                [$(Spinlock::new(Slot { head: core::ptr::null_mut() }),)*]
            };
        }
        macro_rules! init_levels {
            ($($i:expr),*) => {
                [Level { slots: init_slots!(
                    0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,
                    16,17,18,19,20,21,22,23,24,25,26,27,28,29,30,31,
                    32,33,34,35,36,37,38,39,40,41,42,43,44,45,46,47,
                    48,49,50,51,52,53,54,55,56,57,58,59,60,61,62,63,
                    64,65,66,67,68,69,70,71,72,73,74,75,76,77,78,79,
                    80,81,82,83,84,85,86,87,88,89,90,91,92,93,94,95,
                    96,97,98,99,100,101,102,103,104,105,106,107,108,109,110,111,
                    112,113,114,115,116,117,118,119,120,121,122,123,124,125,126,127,
                    128,129,130,131,132,133,134,135,136,137,138,139,140,141,142,143,
                    144,145,146,147,148,149,150,151,152,153,154,155,156,157,158,159,
                    160,161,162,163,164,165,166,167,168,169,170,171,172,173,174,175,
                    176,177,178,179,180,181,182,183,184,185,186,187,188,189,190,191,
                    192,193,194,195,196,197,198,199,200,201,202,203,204,205,206,207,
                    208,209,210,211,212,213,214,215,216,217,218,219,220,221,222,223,
                    224,225,226,227,228,229,230,231,232,233,234,235,236,237,238,239,
                    240,241,242,243,244,245,246,247,248,249,250,251,252,253,254,255
                ) },)*]
            };
        }
        Self {
            tick_count: AtomicU64::new(0),
            levels: init_levels!(0,1,2,3),
        }
    }

    /// Add a timer that fires at `deadline` ticks.
    ///
    /// Returns a raw pointer for cancellation.
    pub fn add_timer(
        &self,
        deadline: u64,
        callback: TimerCallback,
        data: *mut (),
    ) -> *mut TimerEntry {
        let now = self.tick_count.load(Ordering::Relaxed);
        let delta = deadline.saturating_sub(now);

        let (level, slot) = self.slot_for_delta(delta);

        let entry = Box::leak(Box::new(TimerEntry {
            callback,
            data,
            deadline,
            next: core::ptr::null_mut(),
        }));

        // Head insertion into the slot's list.
        let mut slot_guard = self.levels[level].slots[slot].lock();
        entry.next = slot_guard.head;
        slot_guard.head = entry;
        entry
    }

    /// Advance the wheel by one tick. Fires any expired timers.
    pub fn tick(&self) {
        let now = self.tick_count.fetch_add(1, Ordering::Relaxed) + 1;

        let slot = (now & 0xFF) as usize;
        self.process_slot(0, slot, now);

        // Cascade from higher levels on level-0 wrap.
        if slot == 0 {
            let l1_slot = ((now >> 8) & 0xFF) as usize;
            self.cascade_level(1, l1_slot, now);

            if l1_slot == 0 {
                let l2_slot = ((now >> 16) & 0xFF) as usize;
                self.cascade_level(2, l2_slot, now);

                if l2_slot == 0 {
                    let l3_slot = ((now >> 24) & 0xFF) as usize;
                    self.cascade_level(3, l3_slot, now);
                }
            }
        }
    }

    /// Get the current tick count.
    pub fn now(&self) -> u64 {
        self.tick_count.load(Ordering::Relaxed)
    }

    fn slot_for_delta(&self, delta: u64) -> (usize, usize) {
        if delta < 256 {
            (0, delta as usize)
        } else if delta < 65536 {
            (1, ((delta >> 8) & 0xFF) as usize)
        } else if delta < 16777216 {
            (2, ((delta >> 16) & 0xFF) as usize)
        } else {
            (3, ((delta >> 24) & 0xFF) as usize)
        }
    }

    fn process_slot(&self, level: usize, slot: usize, now: u64) {
        let mut guard = self.levels[level].slots[slot].lock();
        let mut entry = guard.head;
        guard.head = core::ptr::null_mut();
        drop(guard);

        // Walk the list and fire expired timers.
        while !entry.is_null() {
            let next = unsafe { (*entry).next };
            if unsafe { (*entry).deadline } <= now {
                let cb = unsafe { (*entry).callback };
                let data = unsafe { (*entry).data };
                cb(data);
                // Free the entry.
                // SAFETY: we allocated it with Box::leak.
                unsafe { drop(Box::from_raw(entry)); }
            } else {
                // Not yet expired. Re-insert.
                let (new_level, new_slot) = self.slot_for_delta(unsafe { (*entry).deadline } - now);
                let mut slot_guard = self.levels[new_level].slots[new_slot].lock();
                unsafe { (*entry).next = slot_guard.head; }
                slot_guard.head = entry;
            }
            entry = next;
        }
    }

    fn cascade_level(&self, level: usize, slot: usize, now: u64) {
        let mut guard = self.levels[level].slots[slot].lock();
        let mut entry = guard.head;
        guard.head = core::ptr::null_mut();
        drop(guard);

        while !entry.is_null() {
            let next = unsafe { (*entry).next };
            let deadline = unsafe { (*entry).deadline };
            let delta = deadline.saturating_sub(now);
            let (new_level, new_slot) = self.slot_for_delta(delta);

            let mut slot_guard = self.levels[new_level].slots[new_slot].lock();
            unsafe { (*entry).next = slot_guard.head; }
            slot_guard.head = entry;
            entry = next;
        }
    }
}