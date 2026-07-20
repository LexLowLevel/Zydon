// Bidirectional async IPC channel.
//
// Two SPSC ring buffers, one per direction. Each endpoint owns a tx
// ring and a reference to the peer's rx ring. On close, peer gets
// Err(PeerClosed). call() provides synchronous request-reply via tx_id.

use alloc::boxed::Box;
use core::sync::atomic::{AtomicU32, Ordering};

use crate::ipc::ring_buffer::{SpscRing, TrySendError};
use crate::ipc::message::Message;
use crate::primitives::wait_queue::{WaitQueue, WaitNode};
use crate::task::task_id::TaskId;

const STATE_OPEN: u32 = 0;
const STATE_PEER_CLOSED: u32 = 1;

/// Construct a channel pair.
pub fn channel_create(capacity: usize) -> (Endpoint, Endpoint) {
    let ring_a_to_b = SpscRing::new(capacity);
    let ring_b_to_a = SpscRing::new(capacity);

    let inner_a = EndpointInner {
        tx: ring_a_to_b,
        rx: ring_b_to_a,
        peer_state: AtomicU32::new(STATE_OPEN),
        recv_waiters: WaitQueue::new(),
    };

    let inner_b = EndpointInner {
        tx: ring_b_to_a,
        rx: ring_a_to_b,
        peer_state: AtomicU32::new(STATE_OPEN),
        recv_waiters: WaitQueue::new(),
    };

    let inner_a = Box::into_raw(Box::new(inner_a));
    let inner_b = Box::into_raw(Box::new(inner_b));

    (Endpoint { inner: inner_a }, Endpoint { inner: inner_b })
}

struct EndpointInner {
    tx: SpscRing,
    rx: SpscRing,
    peer_state: AtomicU32,
    recv_waiters: WaitQueue,
}

// SAFETY: SpscRing is Send+Sync. The spinlock in WaitQueue is Send+Sync.
unsafe impl Send for EndpointInner {}
unsafe impl Sync for EndpointInner {}

/// Channel endpoint. Send and receive messages.
pub struct Endpoint {
    inner: *mut EndpointInner,
}

// SAFETY: EndpointInner is Send+Sync.
unsafe impl Send for Endpoint {}
unsafe impl Sync for Endpoint {}

impl Endpoint {
    /// Non-blocking send. Err if full or peer closed.
    pub fn try_send(&self, msg: Message) -> Result<(), ChannelSendError> {
        let inner = unsafe { &*self.inner };
        if inner.peer_state.load(Ordering::Acquire) == STATE_PEER_CLOSED {
            return Err(ChannelSendError::PeerClosed(msg));
        }
        inner.tx.try_send(msg).map_err(|e| match e {
            TrySendError::Full(m) => ChannelSendError::Full(m),
        })
    }

    /// Async send. Parks if ring full.
    pub fn send(&self, msg: Message) -> SendFuture<'_> {
        SendFuture {
            endpoint: unsafe { &*self.inner },
            msg: Some(msg),
            wait_node: WaitNode::new(),
            registered: false,
        }
    }

    /// Async receive. Parks if ring empty.
    pub fn recv(&self) -> RecvFuture<'_> {
        RecvFuture {
            endpoint: unsafe { &*self.inner },
            wait_node: WaitNode::new(),
            registered: false,
        }
    }

    /// Non-blocking receive.
    pub fn try_recv(&self) -> Result<Message, ChannelRecvError> {
        let inner = unsafe { &*self.inner };
        match inner.rx.try_recv() {
            Some(msg) => Ok(msg),
            None => {
                if inner.peer_state.load(Ordering::Acquire) == STATE_PEER_CLOSED {
                    Err(ChannelRecvError::PeerClosed)
                } else {
                    Err(ChannelRecvError::Empty)
                }
            }
        }
    }

    /// Async call: send request, await reply matching tx_id.
    pub fn call(&self, msg: Message) -> CallFuture<'_> {
        CallFuture {
            endpoint: unsafe { &*self.inner },
            state: CallState::Sending(SendFuture {
                endpoint: unsafe { &*self.inner },
                msg: Some(msg),
                wait_node: WaitNode::new(),
                registered: false,
            }),
        }
    }
}

impl Drop for Endpoint {
    fn drop(&mut self) {
        let inner = unsafe { &*self.inner };
        inner.peer_state.store(STATE_PEER_CLOSED, Ordering::Release);
        inner.recv_waiters.wake_all();
        unsafe { drop(Box::from_raw(self.inner)); }
    }
}

// SendFuture

pub struct SendFuture<'a> {
    endpoint: &'a EndpointInner,
    msg: Option<Message>,
    wait_node: WaitNode,
    registered: bool,
}

impl<'a> core::future::Future for SendFuture<'a> {
    type Output = Result<(), ChannelSendError>;

    fn poll(mut self: core::pin::Pin<&mut Self>, cx: &mut core::task::Context<'_>) -> core::task::Poll<Self::Output> {
        if self.endpoint.peer_state.load(Ordering::Acquire) == STATE_PEER_CLOSED {
            let msg = self.msg.take().unwrap();
            return core::task::Poll::Ready(Err(ChannelSendError::PeerClosed(msg)));
        }

        if let Some(msg) = self.msg.take() {
            match self.endpoint.tx.try_send(msg) {
                Ok(()) => {
        // Wake receiver.
                    self.endpoint.recv_waiters.wake_one();
                    if self.registered {
                        // SAFETY: unlinked during poll.
                        unsafe { self.endpoint.recv_waiters.remove(&mut self.wait_node); }
                        self.registered = false;
                    }
                    return core::task::Poll::Ready(Ok(()));
                }
                Err(TrySendError::Full(m)) => {
                    self.msg = Some(m);
                }
            }
        }

        // Ring full. Park.
        if !self.registered {
            self.wait_node.set_waker(cx.waker().clone());
            // SAFETY: wait_node lives in this future, which is pinned.
            unsafe { self.endpoint.recv_waiters.enqueue(&mut self.wait_node); }
            self.registered = true;
        } else {
            self.wait_node.set_waker(cx.waker().clone());
        }

        core::task::Poll::Pending
    }
}

impl<'a> Drop for SendFuture<'a> {
    fn drop(&mut self) {
        if self.registered {
            // SAFETY: wait_node valid during drop.
            unsafe { self.endpoint.recv_waiters.remove(&mut self.wait_node); }
        }
    }
}

// RecvFuture

pub struct RecvFuture<'a> {
    endpoint: &'a EndpointInner,
    wait_node: WaitNode,
    registered: bool,
}

impl<'a> core::future::Future for RecvFuture<'a> {
    type Output = Result<Message, ChannelRecvError>;

    fn poll(mut self: core::pin::Pin<&mut Self>, cx: &mut core::task::Context<'_>) -> core::task::Poll<Self::Output> {
        // Try receive ring.
        if let Some(msg) = self.endpoint.rx.try_recv() {
            if self.registered {
                // SAFETY: unlinked during poll.
                unsafe { self.endpoint.recv_waiters.remove(&mut self.wait_node); }
                self.registered = false;
            }
            return core::task::Poll::Ready(Ok(msg));
        }

        // Check peer.
        if self.endpoint.peer_state.load(Ordering::Acquire) == STATE_PEER_CLOSED {
            if self.registered {
                unsafe { self.endpoint.recv_waiters.remove(&mut self.wait_node); }
                self.registered = false;
            }
            return core::task::Poll::Ready(Err(ChannelRecvError::PeerClosed));
        }

        // Park.
        if !self.registered {
            self.wait_node.set_waker(cx.waker().clone());
            // SAFETY: wait_node lives in this future.
            unsafe { self.endpoint.recv_waiters.enqueue(&mut self.wait_node); }
            self.registered = true;
        } else {
            self.wait_node.set_waker(cx.waker().clone());
        }

        // Re-check after registering.
        if let Some(msg) = self.endpoint.rx.try_recv() {
            unsafe { self.endpoint.recv_waiters.remove(&mut self.wait_node); }
            self.registered = false;
            return core::task::Poll::Ready(Ok(msg));
        }

        if self.endpoint.peer_state.load(Ordering::Acquire) == STATE_PEER_CLOSED {
            unsafe { self.endpoint.recv_waiters.remove(&mut self.wait_node); }
            self.registered = false;
            return core::task::Poll::Ready(Err(ChannelRecvError::PeerClosed));
        }

        core::task::Poll::Pending
    }
}

impl<'a> Drop for RecvFuture<'a> {
    fn drop(&mut self) {
        if self.registered {
            // SAFETY: wait_node valid during drop.
            unsafe { self.endpoint.recv_waiters.remove(&mut self.wait_node); }
        }
    }
}

// CallFuture

enum CallState<'a> {
    Sending(SendFuture<'a>),
    Receiving(RecvFuture<'a>),
    Done,
}

pub struct CallFuture<'a> {
    endpoint: &'a EndpointInner,
    state: CallState<'a>,
}

impl<'a> core::future::Future for CallFuture<'a> {
    type Output = Result<Message, ChannelCallError>;

    fn poll(mut self: core::pin::Pin<&mut Self>, cx: &mut core::task::Context<'_>) -> core::task::Poll<Self::Output> {
        loop {
            match &mut self.state {
                CallState::Sending(fut) => {
                    let pin = unsafe { core::pin::Pin::new_unchecked(fut) };
                    match pin.poll(cx) {
                        core::task::Poll::Ready(Ok(())) => {
                            let recv = RecvFuture {
                                endpoint: self.endpoint,
                                wait_node: WaitNode::new(),
                                registered: false,
                            };
                            self.state = CallState::Receiving(recv);
                            continue;
                        }
                        core::task::Poll::Ready(Err(e)) => {
                            self.state = CallState::Done;
                            return core::task::Poll::Ready(Err(match e {
                                ChannelSendError::PeerClosed(_) => ChannelCallError::PeerClosed,
                                ChannelSendError::Full(_) => ChannelCallError::Full,
                            }));
                        }
                        core::task::Poll::Pending => return core::task::Poll::Pending,
                    }
                }
                CallState::Receiving(fut) => {
                    let pin = unsafe { core::pin::Pin::new_unchecked(fut) };
                    match pin.poll(cx) {
                        core::task::Poll::Ready(Ok(msg)) => {
                            self.state = CallState::Done;
                            return core::task::Poll::Ready(Ok(msg));
                        }
                        core::task::Poll::Ready(Err(e)) => {
                            self.state = CallState::Done;
                            return core::task::Poll::Ready(Err(match e {
                                ChannelRecvError::PeerClosed => ChannelCallError::PeerClosed,
                                ChannelRecvError::Empty => ChannelCallError::Empty,
                            }));
                        }
                        core::task::Poll::Pending => return core::task::Poll::Pending,
                    }
                }
                CallState::Done => unreachable!("poll after completion"),
            }
        }
    }
}

// Errors

#[derive(Debug)]
pub enum ChannelSendError {
    Full(Message),
    PeerClosed(Message),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelRecvError {
    Empty,
    PeerClosed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelCallError {
    Full,
    PeerClosed,
    Empty,
}
