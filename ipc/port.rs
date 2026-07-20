// Named port registry for service discovery.
//
// Rendezvous point: server calls listen(), client calls connect().
// connect() creates a channel and delivers server-side endpoint.
// Early clients park until a server listens. Backlog capped at 16.
// Registry is a 16-stripe spinlock-protected hash map.

use core::sync::atomic::{AtomicUsize, Ordering};

use crate::primitives::spinlock::Spinlock;
use crate::primitives::wait_queue::{WaitQueue, WaitNode};
use crate::ipc::channel::{self, Endpoint};

const NUM_STRIPES: usize = 16;

/// Global port registry.
pub static PORT_REGISTRY: PortRegistry = PortRegistry::new();

pub struct PortRegistry {
    stripes: [PortStripe; NUM_STRIPES],
}

struct PortStripe {
    _lock: Spinlock<()>,
    /// Ports in this stripe. Linear scan; ports are few.
    ports: core::cell::UnsafeCell<heapless::Vec<Port, 64>>,
}

// SAFETY: access is gated by the spinlock.
unsafe impl Send for PortStripe {}
unsafe impl Sync for PortStripe {}

impl PortRegistry {
    pub const fn new() -> Self {
        macro_rules! init_stripes {
            ($($i:expr),*) => {
                [$(PortStripe {
                    _lock: Spinlock::new(()),
                    ports: core::cell::UnsafeCell::new(heapless::Vec::new()),
                },)*]
            };
        }
        Self {
            stripes: init_stripes!(
                0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15
            ),
        }
    }

    fn stripe_for(&self, name: &str) -> &PortStripe {
        let hash = fnv1a(name);
        &self.stripes[hash % NUM_STRIPES]
    }

    /// Connect to a named port. Creates a channel and delivers the
    /// server-side endpoint to the port. Returns the client-side endpoint.
    /// Parks if no server is listening yet.
    pub fn connect(&self, name: &str, capacity: usize) -> ConnectFuture<'_> {
        ConnectFuture {
            registry: self,
            name,
            capacity,
            wait_node: WaitNode::new(),
            registered: false,
        }
    }

    /// Listen on a named port. Returns a future that yields client
    /// endpoints as clients connect. Each call to recv on the returned
    /// endpoint awaits the next client.
    pub fn listen(&self, name: &str, capacity: usize) -> ListenFuture<'_> {
        ListenFuture {
            registry: self,
            name,
            capacity,
            wait_node: WaitNode::new(),
            registered: false,
        }
    }
}

struct Port {
    name: &'static str,
    /// Parked clients waiting for a server.
    client_waiters: WaitQueue,
    /// Parked servers waiting for clients.
    server_waiters: WaitQueue,
    /// Pending server endpoints from connect() calls, waiting for a listen().
    pending_servers: heapless::Vec<Endpoint, 16>,
    /// Channel capacity for new connections.
    capacity: usize,
}

/// Future returned by PortRegistry::connect().
pub struct ConnectFuture<'a> {
    registry: &'a PortRegistry,
    name: &'a str,
    capacity: usize,
    wait_node: WaitNode,
    registered: bool,
}

impl<'a> core::future::Future for ConnectFuture<'a> {
    type Output = Result<Endpoint, PortError>;

    fn poll(mut self: core::pin::Pin<&mut Self>, cx: &mut core::task::Context<'_>) -> core::task::Poll<Self::Output> {
        let stripe = self.registry.stripe_for(self.name);
        let _guard = stripe._lock.lock();
        let ports = unsafe { &mut *stripe.ports.get() };

        // Look for an existing port.
        let port = ports.iter_mut().find(|p| p.name == self.name);

        if let Some(port) = port {
            // Port exists. Create a channel and deliver server endpoint.
            let (client_ep, server_ep) = channel::channel_create(self.capacity);

            // Store the server endpoint for a pending listen(), or wake a server.
            if port.server_waiters.has_waiters() {
                // A server is parked; store the endpoint and wake it.
                let _ = port.pending_servers.push(server_ep);
                drop(_guard);
                port.server_waiters.wake_one();
            } else {
                // No server yet; park the client and store the endpoint.
                let _ = port.pending_servers.push(server_ep);
                drop(_guard);
            }

            return core::task::Poll::Ready(Ok(client_ep));
        }

        // No port registered yet. Register it and park as a client.
        let (client_ep, server_ep) = channel::channel_create(self.capacity);

        // Create the port with the server endpoint pending.
        let mut new_port = Port {
            name: self.name,
            client_waiters: WaitQueue::new(),
            server_waiters: WaitQueue::new(),
            pending_servers: heapless::Vec::new(),
            capacity: self.capacity,
        };
        let _ = new_port.pending_servers.push(server_ep);
        let _ = ports.push(new_port);

        core::task::Poll::Ready(Ok(client_ep))
    }
}

/// Future returned by PortRegistry::listen().
pub struct ListenFuture<'a> {
    registry: &'a PortRegistry,
    name: &'a str,
    capacity: usize,
    wait_node: WaitNode,
    registered: bool,
}

impl<'a> core::future::Future for ListenFuture<'a> {
    type Output = Result<Endpoint, PortError>;

    fn poll(mut self: core::pin::Pin<&mut Self>, cx: &mut core::task::Context<'_>) -> core::task::Poll<Self::Output> {
        let stripe = self.registry.stripe_for(self.name);
        let _guard = stripe._lock.lock();
        let ports = unsafe { &mut *stripe.ports.get() };

        let port = ports.iter_mut().find(|p| p.name == self.name);

        if let Some(port) = port {
            // Check for a pending server endpoint from a connect() call.
            if let Some(server_ep) = port.pending_servers.pop() {
                return core::task::Poll::Ready(Ok(server_ep));
            }

            // No pending client; park and wait for one.
            if !self.registered {
                self.wait_node.set_waker(cx.waker().clone());
                unsafe {
                    port.server_waiters.enqueue(&mut self.wait_node);
                }
                self.registered = true;
            } else {
                self.wait_node.set_waker(cx.waker().clone());
            }

            // Re-check after registering.
            if let Some(server_ep) = port.pending_servers.pop() {
                unsafe { port.server_waiters.remove(&mut self.wait_node); }
                self.registered = false;
                return core::task::Poll::Ready(Ok(server_ep));
            }

            return core::task::Poll::Pending;
        }

        // Port doesn't exist yet. Create it and park as server.
        let new_port = Port {
            name: self.name,
            client_waiters: WaitQueue::new(),
            server_waiters: WaitQueue::new(),
            pending_servers: heapless::Vec::new(),
            capacity: self.capacity,
        };
        let _ = ports.push(new_port);

        // Park as a server waiting for clients.
        let port = ports.iter_mut().find(|p| p.name == self.name).unwrap();
        self.wait_node.set_waker(cx.waker().clone());
        unsafe {
            port.server_waiters.enqueue(&mut self.wait_node);
        }
        self.registered = true;

        core::task::Poll::Pending
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortError {
    NotFound,
    AlreadyBound,
}

fn fnv1a(s: &str) -> usize {
    let mut hash: usize = 0xcbf29ce484222325;
    for b in s.bytes() {
        hash ^= b as usize;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

// heapless Vec
mod heapless {
    pub struct Vec<T, const N: usize> {
        buf: [core::mem::MaybeUninit<T>; N],
        len: usize,
    }

    impl<T, const N: usize> Vec<T, N> {
        pub const fn new() -> Self {
            Self {
                buf: [const { core::mem::MaybeUninit::uninit() }; N],
                len: 0,
            }
        }

        pub fn push(&mut self, val: T) -> Result<(), T> {
            if self.len >= N {
                return Err(val);
            }
            self.buf[self.len] = core::mem::MaybeUninit::new(val);
            self.len += 1;
            Ok(())
        }

        pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut T> {
            (0..self.len).map(move |i| unsafe { self.buf[i].assume_init_mut() })
        }
    }
}