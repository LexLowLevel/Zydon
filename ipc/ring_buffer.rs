// Lock-free SPSC ring buffer for IPC messages.
//
// Producer and consumer each own their head/tail on separate cache lines
// to avoid false sharing. Small messages are copied inline; large ones
// use zero-copy VMO path with only a descriptor enqueued.
//
// Memory ordering:
//   Producer: write data, Release head.
//   Consumer: Acquire head, read data, Release tail.

use core::sync::atomic::{AtomicUsize, Ordering};
use core::cell::UnsafeCell;

use crate::primitives::cache_padded::CachePadded;
use crate::ipc::message::Message;

/// Lock-free SPSC ring buffer for IPC messages.
pub struct SpscRing {
    head: CachePadded<AtomicUsize>,  // next write slot (producer-only)
    tail: CachePadded<AtomicUsize>,  // next read slot (consumer-only)
    buf: UnsafeCell<RingBuffer>,
    capacity: usize,
}

struct RingBuffer {
    slots: *mut Slot,
    capacity: usize,
}

/// A single slot. `occupied` flag indicates whether it contains a message.
struct Slot {
    occupied: AtomicUsize,
    message: UnsafeCell<Option<Message>>,
}

// SAFETY: SPSC - producer only writes head/message, consumer only writes tail.
// They never write to the same slot simultaneously.
unsafe impl Send for SpscRing {}
unsafe impl Sync for SpscRing {}

impl SpscRing {
    /// Create a new ring with given capacity (must be power of 2).
    pub fn new(capacity: usize) -> Self {
        assert!(capacity > 0 && capacity.is_power_of_two(), "capacity must be a power of 2");
        let layout = core::alloc::Layout::array::<Slot>(capacity)
            .expect("ring capacity overflow");
        let slots = unsafe { alloc::alloc::alloc(layout) } as *mut Slot;
        assert!(!slots.is_null(), "ring allocation failed");

        for i in 0..capacity {
            unsafe {
                let slot = slots.add(i);
                core::ptr::write(&mut (*slot).occupied, AtomicUsize::new(0));
                core::ptr::write(&mut (*slot).message, UnsafeCell::new(None));
            }
        }

        Self {
            head: CachePadded::new(AtomicUsize::new(0)),
            tail: CachePadded::new(AtomicUsize::new(0)),
            buf: UnsafeCell::new(RingBuffer { slots, capacity }),
            capacity,
        }
    }

    /// Try to enqueue a message. Non-blocking, always lock-free.
    pub fn try_send(&self, msg: Message) -> Result<(), TrySendError> {
        let head = self.head.load(Ordering::Relaxed);
        let tail = self.tail.load(Ordering::Acquire);

        let available = self.capacity - (head - tail);
        if available == 0 {
            return Err(TrySendError::Full(msg));
        }

        let idx = head & (self.capacity - 1);
        let buf = unsafe { &*self.buf.get() };
        let slot = unsafe { &*buf.slots.add(idx) };

        // Write message into slot.
        unsafe {
            *slot.message.get() = Some(msg);
        }

        slot.occupied.store(1, Ordering::Release);
        self.head.store(head + 1, Ordering::Release);

        Ok(())
    }

    /// Try to dequeue a message. Non-blocking, always lock-free.
    pub fn try_recv(&self) -> Option<Message> {
        let tail = self.tail.load(Ordering::Relaxed);
        let head = self.head.load(Ordering::Acquire);

        if tail == head {
            return None;
        }

        let idx = tail & (self.capacity - 1);
        let buf = unsafe { &*self.buf.get() };
        let slot = unsafe { &*buf.slots.add(idx) };

        // Wait for slot to be marked occupied (defense against reordering).
        while slot.occupied.load(Ordering::Acquire) == 0 {
            core::hint::spin_loop();
        }

        let msg = unsafe { (*slot.message.get()).take() };
        slot.occupied.store(0, Ordering::Release);
        self.tail.store(tail + 1, Ordering::Release);

        msg
    }

    pub fn len(&self) -> usize {
        let head = self.head.load(Ordering::Relaxed);
        let tail = self.tail.load(Ordering::Relaxed);
        head - tail
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }
}

impl Drop for SpscRing {
    fn drop(&mut self) {
        // Drain remaining messages.
        while self.try_recv().is_some() {}

        // Free the slot array.
        let layout = core::alloc::Layout::array::<Slot>(self.capacity)
            .expect("ring capacity");
        unsafe {
            alloc::alloc::dealloc((*self.buf.get()).slots as *mut u8, layout);
        }
    }
}

#[derive(Debug)]
pub enum TrySendError {
    Full(Message),
}
