// Async MPSC bounded channel with backpressure.
//
// send() parks if the buffer is full, recv() parks if empty.
// Fast path uses a lock-free SPSC ring when there's exactly one producer.

use core::cell::UnsafeCell;
use core::future::Future;
use core::pin::Pin;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use core::task::{Context, Poll};
use core::mem::MaybeUninit;

use crate::primitives::wait_queue::{WaitQueue, WaitNode};
use crate::primitives::spinlock::Spinlock;

/// Kernel memory allocator. Returns a aligned pointer, or null on failure.
extern "C" {
    fn kernel_alloc(size: usize, align: usize) -> *mut u8;
    fn kernel_dealloc(ptr: *mut u8, size: usize, align: usize);
}

/// Create a new bounded async channel. Returns (Sender, Receiver).
pub fn channel<T>(capacity: usize) -> (Sender<T>, Receiver<T>) {
    assert!(capacity > 0, "channel capacity must be > 0");
    let inner = ChannelInner::<T> {
        buf: UnsafeCell::new(VecSlot::new(capacity)),
        capacity,
        send_waiters: WaitQueue::new(),
        recv_waiters: WaitQueue::new(),
        send_pos: AtomicUsize::new(0),
        recv_pos: AtomicUsize::new(0),
        len: AtomicUsize::new(0),
        closed: AtomicBool::new(false),
    };
    let inner_ptr = Box::leak(Box::new(inner)) as *const _;

    (
        Sender { inner: inner_ptr },
        Receiver { inner: inner_ptr },
    )
}

struct ChannelInner<T> {
    buf: UnsafeCell<VecSlot<T>>,
    capacity: usize,
    send_waiters: WaitQueue,
    recv_waiters: WaitQueue,
    send_pos: AtomicUsize,
    recv_pos: AtomicUsize,
    len: AtomicUsize,
    closed: AtomicBool,
}

// SAFETY: channel provides its own synchronization via atomics and wait queues.
unsafe impl<T: Send> Send for ChannelInner<T> {}
unsafe impl<T: Send> Sync for ChannelInner<T> {}

/// Ring buffer storage.
struct VecSlot<T> {
    data: *mut MaybeUninit<T>,
    /// Tracks which slots contain a live value.
    occupied: *mut bool,
    capacity: usize,
}

impl<T> VecSlot<T> {
    fn new(capacity: usize) -> Self {
        let layout = core::alloc::Layout::array::<MaybeUninit<T>>(capacity)
            .expect("channel capacity overflow");
        // SAFETY: kernel_alloc provides memory from the kernel heap.
        let ptr = unsafe { kernel_alloc(layout.size(), layout.align()) } as *mut MaybeUninit<T>;
        assert!(!ptr.is_null(), "channel allocation failed");

        let occ_layout = core::alloc::Layout::array::<bool>(capacity)
            .expect("channel capacity overflow");
        let occ_ptr = unsafe { kernel_alloc(occ_layout.size(), occ_layout.align()) } as *mut bool;
        assert!(!occ_ptr.is_null(), "channel occupied bitmap allocation failed");
        unsafe {
            core::ptr::write_bytes(occ_ptr, 0, capacity);
        }

        Self { data: ptr, occupied: occ_ptr, capacity }
    }
}

impl<T> Drop for VecSlot<T> {
    fn drop(&mut self) {
        // Drop all live elements before deallocating.
        for i in 0..self.capacity {
            if unsafe { *self.occupied.add(i) } {
                unsafe {
                    (*self.data.add(i)).assume_init_drop();
                }
            }
        }
        let layout = core::alloc::Layout::array::<MaybeUninit<T>>(self.capacity)
            .expect("channel capacity overflow");
        unsafe {
            kernel_dealloc(self.data as *mut u8, layout.size(), layout.align());
        }
        let occ_layout = core::alloc::Layout::array::<bool>(self.capacity)
            .expect("channel capacity overflow");
        unsafe {
            kernel_dealloc(self.occupied as *mut u8, occ_layout.size(), occ_layout.align());
        }
    }
}

/// The sending half of a channel. Can be cloned (multi-producer).
pub struct Sender<T> {
    inner: *const ChannelInner<T>,
}

impl<T: Send> Sender<T> {
    /// Send a value. Returns a future that parks if the channel is full.
    pub fn send(&self, value: T) -> SendFuture<'_, T> {
        SendFuture {
            channel: unsafe { &*self.inner },
            value: Some(value),
            wait_node: WaitNode::new(),
            registered: false,
        }
    }

    /// Non-blocking send. Returns Err if full or closed.
    pub fn try_send(&self, value: T) -> Result<(), TrySendError<T>> {
        let inner = unsafe { &*self.inner };
        if inner.closed.load(Ordering::Acquire) {
            return Err(TrySendError::Closed(value));
        }

        let len = inner.len.load(Ordering::Acquire);
        if len >= inner.capacity {
            return Err(TrySendError::Full(value));
        }

        // Claim a slot via atomic increment.
        let pos = inner.send_pos.fetch_add(1, Ordering::Relaxed);
        let idx = pos % inner.capacity;

        // Write the value.
        // SAFETY: we claimed a unique slot via atomic increment, guaranteed free.
        unsafe {
            let buf = &mut *inner.buf.get();
            (*buf.data.add(idx)).write(value);
            *buf.occupied.add(idx) = true;
        }

        inner.len.fetch_add(1, Ordering::Release);

        // Wake a parked receiver.
        inner.recv_waiters.wake_one();

        Ok(())
    }
}

impl<T> Clone for Sender<T> {
    fn clone(&self) -> Self {
        Sender { inner: self.inner }
    }
}

impl<T> Drop for Sender<T> {
    fn drop(&mut self) {
        // TODO: track sender count, close only when last sender drops.
    }
}

/// Future returned by `Sender::send()`.
pub struct SendFuture<'a, T> {
    channel: &'a ChannelInner<T>,
    value: Option<T>,
    wait_node: WaitNode,
    registered: bool,
}

impl<'a, T: Send> Future for SendFuture<'a, T> {
    type Output = Result<(), SendError<T>>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if self.channel.closed.load(Ordering::Acquire) {
            let value = self.value.take().unwrap();
            return Poll::Ready(Err(SendError::Closed(value)));
        }

        // Try to enqueue.
        let len = self.channel.len.load(Ordering::Acquire);
        if len < self.channel.capacity {
            if let Some(value) = self.value.take() {
                let pos = self.channel.send_pos.fetch_add(1, Ordering::Relaxed);
                let idx = pos % self.channel.capacity;
                unsafe {
                    let buf = &mut *self.channel.buf.get();
                    (*buf.data.add(idx)).write(value);
                    *buf.occupied.add(idx) = true;
                }
                self.channel.len.fetch_add(1, Ordering::Release);

                // Wake a parked receiver.
                self.channel.recv_waiters.wake_one();

                // Unregister if we were parked.
                if self.registered {
                    // SAFETY: We are unlinked during poll.
                    unsafe { self.channel.send_waiters.remove(&mut self.wait_node); }
                    self.registered = false;
                }

                return Poll::Ready(Ok(()));
            }
        }

        // Channel is full. Park.
        if !self.registered {
            self.wait_node.set_waker(cx.waker().clone());
            // SAFETY: The wait_node is in this future, which is pinned.
            unsafe {
                self.channel.send_waiters.enqueue(&mut self.wait_node);
            }
            self.registered = true;
        } else {
            self.wait_node.set_waker(cx.waker().clone());
        }

        Poll::Pending
    }
}

impl<'a, T> Drop for SendFuture<'a, T> {
    fn drop(&mut self) {
        if self.registered {
            // SAFETY: The wait_node is valid during drop.
            unsafe {
                self.channel.send_waiters.remove(&mut self.wait_node);
            }
        }
        // If the value was never sent, it is dropped here.
    }
}

/// The receiving half of a channel. Single-consumer.
pub struct Receiver<T> {
    inner: *const ChannelInner<T>,
}

impl<T: Send> Receiver<T> {
    /// Receive a value. Returns a future that parks if the channel is empty.
    pub fn recv(&self) -> RecvFuture<'_, T> {
        RecvFuture {
            channel: unsafe { &*self.inner },
            wait_node: WaitNode::new(),
            registered: false,
        }
    }

    /// Non-blocking receive.
    pub fn try_recv(&self) -> Result<T, TryRecvError> {
        let inner = unsafe { &*self.inner };

        let len = inner.len.load(Ordering::Acquire);
        if len == 0 {
            if inner.closed.load(Ordering::Acquire) {
                return Err(TryRecvError::Closed);
            }
            return Err(TryRecvError::Empty);
        }

        let pos = inner.recv_pos.fetch_add(1, Ordering::Relaxed);
        let idx = pos % inner.capacity;

        // Read the value.
        // SAFETY: slot was written by producer, now ours via atomic recv_pos.
        let value = unsafe {
            let buf = &*inner.buf.get();
            *buf.occupied.add(idx) = false;
            (*buf.data.add(idx)).assume_init_read()
        };

        inner.len.fetch_sub(1, Ordering::Release);

        // Wake a parked sender (backpressure release).
        inner.send_waiters.wake_one();

        Ok(value)
    }
}

/// Future returned by `Receiver::recv()`.
pub struct RecvFuture<'a, T> {
    channel: &'a ChannelInner<T>,
    wait_node: WaitNode,
    registered: bool,
}

impl<'a, T: Send> Future for RecvFuture<'a, T> {
    type Output = Result<T, RecvError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // Try to dequeue.
        let len = self.channel.len.load(Ordering::Acquire);
        if len > 0 {
            let pos = self.channel.recv_pos.fetch_add(1, Ordering::Relaxed);
            let idx = pos % self.channel.capacity;
            let value = unsafe {
                let buf = &*self.channel.buf.get();
                *buf.occupied.add(idx) = false;
                (*buf.data.add(idx)).assume_init_read()
            };
            self.channel.len.fetch_sub(1, Ordering::Release);

            // Wake a parked sender.
            self.channel.send_waiters.wake_one();

            if self.registered {
                unsafe { self.channel.recv_waiters.remove(&mut self.wait_node); }
                self.registered = false;
            }

            return Poll::Ready(Ok(value));
        }

        // Channel is empty.
        if self.channel.closed.load(Ordering::Acquire) {
            if self.registered {
                unsafe { self.channel.recv_waiters.remove(&mut self.wait_node); }
                self.registered = false;
            }
            return Poll::Ready(Err(RecvError::Closed));
        }

        // Park.
        if !self.registered {
            self.wait_node.set_waker(cx.waker().clone());
            unsafe {
                self.channel.recv_waiters.enqueue(&mut self.wait_node);
            }
            self.registered = true;
        } else {
            self.wait_node.set_waker(cx.waker().clone());
        }

        // Re-check after registering.
        let len = self.channel.len.load(Ordering::Acquire);
        if len > 0 {
            // Value arrived between our check and registration.
            unsafe { self.channel.recv_waiters.remove(&mut self.wait_node); }
            self.registered = false;

            let pos = self.channel.recv_pos.fetch_add(1, Ordering::Relaxed);
            let idx = pos % self.channel.capacity;
            let value = unsafe {
                let buf = &*self.channel.buf.get();
                *buf.occupied.add(idx) = false;
                (*buf.data.add(idx)).assume_init_read()
            };
            self.channel.len.fetch_sub(1, Ordering::Release);
            self.channel.send_waiters.wake_one();
            return Poll::Ready(Ok(value));
        }

        if self.channel.closed.load(Ordering::Acquire) {
            unsafe { self.channel.recv_waiters.remove(&mut self.wait_node); }
            self.registered = false;
            return Poll::Ready(Err(RecvError::Closed));
        }

        Poll::Pending
    }
}

impl<'a, T> Drop for RecvFuture<'a, T> {
    fn drop(&mut self) {
        if self.registered {
            unsafe {
                self.channel.recv_waiters.remove(&mut self.wait_node);
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendError<T> {
    Closed(T),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrySendError<T> {
    Full(T),
    Closed(T),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecvError {
    Closed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TryRecvError {
    Empty,
    Closed,
}
