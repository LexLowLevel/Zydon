// Handle table.
//
// Per-process mapping from handle IDs to kernel object references.
// Uses generational indices to detect use-after-close.

use core::sync::atomic::{AtomicU32, Ordering};

use crate::primitives::spinlock::Spinlock;
use crate::object::kernel_object::KoRef;
use crate::object::rights::Rights;

/// Handle value passed to userspace. Upper 32 = generation, lower 32 = index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Handle(u64);

impl Handle {
    pub const fn from_raw(raw: u64) -> Self {
        Handle(raw)
    }

    pub const fn to_raw(&self) -> u64 {
        self.0
    }

    pub const fn index(&self) -> usize {
        (self.0 & 0xFFFF_FFFF) as usize
    }

    pub const fn generation(&self) -> u32 {
        (self.0 >> 32) as u32
    }

    pub const INVALID: Handle = Handle(u64::MAX);
}

struct Slot {
    generation: AtomicU32,
    entry: Option<HandleEntry>,
    /// Index of the next free slot, or None if this is the tail of the free list.
    next_free: Option<usize>,
}

struct HandleEntry {
    ko_ref: KoRef,
    rights: Rights,
}

/// Per-process handle table. Uses a generational-index array with free list.
pub struct HandleTable {
    slots: Spinlock<HandleTableInner>,
}

struct HandleTableInner {
    slots: Vec<Option<Slot>>,
    free_head: Option<usize>,
    count: usize,
}

impl HandleTable {
    /// Create a new handle table with given initial capacity.
    pub fn new(capacity: usize) -> Self {
        let mut slots = Vec::with_capacity(capacity);
        let mut free_head = Some(0);
        for i in 0..capacity {
            slots.push(Some(Slot {
                generation: AtomicU32::new(0),
                entry: None,
                next_free: if i + 1 < capacity { Some(i + 1) } else { None },
            }));
        }

        Self {
            slots: Spinlock::new(HandleTableInner {
                slots,
                free_head,
                count: 0,
            }),
        }
    }

    /// Create a new handle. Returns Err if table is full.
    pub fn create(&self, ko_ref: KoRef, rights: Rights) -> Result<Handle, HandleError> {
        let mut inner = self.slots.lock();

        let idx = inner.free_head.ok_or(HandleError::TableFull)?;

        // Pop from free list using the stored next_free pointer.
        let slot = inner.slots[idx].as_mut().unwrap();
        inner.free_head = slot.next_free;
        slot.next_free = None;

        let slot = inner.slots[idx].as_mut().unwrap();
        let gen = slot.generation.load(Ordering::Relaxed);
        slot.entry = Some(HandleEntry { ko_ref, rights });
        inner.count += 1;

        Ok(Handle(((gen as u64) << 32) | (idx as u64)))
    }

    /// Look up a handle. Returns None if invalid or generation mismatch.
    pub fn get(&self, handle: Handle) -> Option<(KoRef, Rights)> {
        let inner = self.slots.lock();
        let idx = handle.index();
        let gen = handle.generation();

        let slot = inner.slots.get(idx)?.as_ref()?;
        let current_gen = slot.generation.load(Ordering::Acquire);
        if current_gen != gen {
            return None;
        }

        let entry = slot.entry.as_ref()?;
        Some((entry.ko_ref.clone(), entry.rights))
    }

    /// Duplicate a handle with rights downgrade (monotonic lossy).
    /// Returns Err if source is invalid or lacks DUPLICATE right.
    pub fn duplicate(
        &self,
        handle: Handle,
        new_rights: Rights,
    ) -> Result<Handle, HandleError> {
        let mut inner = self.slots.lock();
        let idx = handle.index();
        let gen = handle.generation();

        let slot = inner.slots.get(idx).and_then(|s| s.as_ref());
        let slot = match slot {
            Some(s) => s,
            None => return Err(HandleError::InvalidHandle),
        };

        let current_gen = slot.generation.load(Ordering::Acquire);
        if current_gen != gen {
            return Err(HandleError::InvalidHandle);
        }

        let entry = match &slot.entry {
            Some(e) => e,
            None => return Err(HandleError::InvalidHandle),
        };

        if !entry.rights.contains(Rights::DUPLICATE) {
            return Err(HandleError::AccessDenied);
        }

        let effective_rights = entry.rights.downgrade(new_rights);

        // Allocate a new slot.
        let new_idx = inner.free_head.ok_or(HandleError::TableFull)?;
        let new_slot = inner.slots[new_idx].as_mut().unwrap();
        inner.free_head = new_slot.next_free;
        new_slot.next_free = None;

        let new_slot = inner.slots[new_idx].as_mut().unwrap();
        let new_gen = new_slot.generation.load(Ordering::Relaxed);
        new_slot.entry = Some(HandleEntry {
            ko_ref: entry.ko_ref.clone(),
            rights: effective_rights,
        });
        inner.count += 1;

        Ok(Handle(((new_gen as u64) << 32) | (new_idx as u64)))
    }

    /// Close a handle. Returns Err if invalid or generation mismatch.
    pub fn close(&self, handle: Handle) -> Result<(), HandleError> {
        let mut inner = self.slots.lock();
        let idx = handle.index();
        let gen = handle.generation();

        let slot = inner.slots.get_mut(idx).and_then(|s| s.as_mut());
        let slot = match slot {
            Some(s) => s,
            None => return Err(HandleError::InvalidHandle),
        };

        let current_gen = slot.generation.load(Ordering::Acquire);
        if current_gen != gen {
            return Err(HandleError::InvalidHandle);
        }

        if slot.entry.take().is_some() {
            // Bump generation to invalidate stale handles.
            slot.generation.fetch_add(1, Ordering::Release);
            inner.count -= 1;

            // Link into the head of the free list.
            slot.next_free = inner.free_head;
            inner.free_head = Some(idx);
        }

        Ok(())
    }

    /// Atomically transfer a handle to another handle table.
    /// Lock order: lower address first to prevent deadlock.
    pub fn transfer(
        &self,
        handle: Handle,
        dest: &HandleTable,
    ) -> Result<Handle, HandleError> {
        // Lock both tables. Lower address first to avoid deadlock.
        let (first, second) = if (self as *const _) < (dest as *const _) {
            (self, dest)
        } else {
            (dest, self)
        };

        let mut first_inner = first.slots.lock();
        let mut second_inner = second.slots.lock();

        let idx = handle.index();
        let gen = handle.generation();

        let slot = first_inner.slots.get(idx).and_then(|s| s.as_ref());
        let slot = match slot {
            Some(s) => s,
            None => return Err(HandleError::InvalidHandle),
        };

        let current_gen = slot.generation.load(Ordering::Acquire);
        if current_gen != gen {
            return Err(HandleError::InvalidHandle);
        }

        let entry = match &slot.entry {
            Some(e) => e,
            None => return Err(HandleError::InvalidHandle),
        };

        if !entry.rights.contains(Rights::TRANSFER) {
            return Err(HandleError::AccessDenied);
        }

        // Move entry to destination.
        let rights = entry.rights;
        let ko_ref = entry.ko_ref.clone();

        // Close in source (we already hold both locks).
        let source_idx = idx;

        // Allocate in destination.
        let dest_idx = second_inner.free_head.ok_or(HandleError::TableFull)?;
        let dest_slot = second_inner.slots[dest_idx].as_mut().unwrap();
        second_inner.free_head = dest_slot.next_free;
        dest_slot.next_free = None;

        let dest_slot = second_inner.slots[dest_idx].as_mut().unwrap();
        let dest_gen = dest_slot.generation.load(Ordering::Relaxed);
        dest_slot.entry = Some(HandleEntry { ko_ref, rights });
        second_inner.count += 1;

        // Now close in source.
        let source_slot = first_inner.slots[source_idx].as_mut().unwrap();
        source_slot.entry = None;
        source_slot.generation.fetch_add(1, Ordering::Release);
        first_inner.count -= 1;
        // Link freed source slot into its free list.
        source_slot.next_free = first_inner.free_head;
        first_inner.free_head = Some(source_idx);

        Ok(Handle(((dest_gen as u64) << 32) | (dest_idx as u64)))
    }

    pub fn count(&self) -> usize {
        self.slots.lock().count
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandleError {
    InvalidHandle,
    AccessDenied,
    TableFull,
}
