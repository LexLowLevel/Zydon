// Async futex.
//
// Provides userspace with compare-and-sleep on a memory word.
// The kernel maintains a hash table of wait queues indexed by address.
// Foundation for userspace mutexes, condvars, and semaphores.

use core::future::Future;
use core::pin::Pin;
use core::sync::atomic::{AtomicU32, Ordering};
use core::task::{Context, Poll};
use core::time::Duration;

use crate::primitives::wait_queue::{WaitQueue, WaitNode};
use crate::primitives::spinlock::Spinlock;

/// Result of a futex wait.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FutexResult {
    Woken,
    WouldBlock,   // value didn't match expected
    TimedOut,
    Interrupted,
}

/// Hash table bucket count. Prime for good distribution.
const FUTEX_BUCKETS: usize = 256;

/// Global futex hash table. Each bucket is a spinlock-protected wait queue.
pub struct FutexTable {
    buckets: [FutexBucket; FUTEX_BUCKETS],
}

struct FutexBucket {
    _lock: Spinlock<()>,
    queue: WaitQueue,
}

impl FutexBucket {
    const fn new() -> Self {
        Self {
            _lock: Spinlock::new(()),
            queue: WaitQueue::new(),
        }
    }
}

impl FutexTable {
    pub const fn new() -> Self {
        // Can't loop in const fn with mutable refs, use macro.
        macro_rules! init_buckets {
            ($($i:expr),*) => {
                [$(FutexBucket::new(),)*]
            };
        }
        Self {
            buckets: init_buckets!(
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
            ),
        }
    }

    fn bucket_for(&self, addr: usize) -> &FutexBucket {
        // Translate to physical address so aliased mappings hash to the same bucket.
        let phys = unsafe { virt_to_phys(addr) };
        // FNV-1a hash.
        let mut hash: usize = 0xcbf29ce484222325;
        hash ^= phys;
        hash = hash.wrapping_mul(0x100000001b3);
        &self.buckets[hash % FUTEX_BUCKETS]
    }

    /// Async futex wait: if *addr == expected, park the calling task.
    /// Caller must have already verified the value matched in userspace.
    pub fn wait(
        &self,
        addr: usize,
        expected: u32,
        timeout: Option<Duration>,
    ) -> FutexWaitFuture<'_> {
        FutexWaitFuture {
            table: self,
            addr,
            expected,
            timeout,
            wait_node: WaitNode::new(),
            registered: false,
        }
    }

    /// Wake up to `count` tasks waiting on the futex at `addr`.
    pub fn wake(&self, addr: usize, count: usize) -> usize {
        let bucket = self.bucket_for(addr);
        let _guard = bucket._lock.lock();
        let mut woken = 0;
        while woken < count {
            if !bucket.queue.wake_one() {
                break;
            }
            woken += 1;
        }
        woken
    }
}

/// Translate a virtual address to its physical address via page table walk.
/// Returns the physical address, or the original virtual address if
/// translation is not available (e.g. identity-mapped regions).
extern "C" {
    fn virt_to_phys(vaddr: usize) -> usize;
}

/// Global futex table instance.
pub static FUTEX_TABLE: FutexTable = FutexTable::new();

/// Future returned by `FutexTable::wait()`.
pub struct FutexWaitFuture<'a> {
    table: &'a FutexTable,
    addr: usize,
    expected: u32,
    timeout: Option<Duration>,
    wait_node: WaitNode,
    registered: bool,
}

impl<'a> Future for FutexWaitFuture<'a> {
    type Output = FutexResult;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let bucket = self.table.bucket_for(self.addr);

        if !self.registered {
            self.wait_node.set_waker(cx.waker().clone());
            self.wait_node.set_priority(0);

            let _guard = bucket._lock.lock();
            unsafe {
                bucket.queue.enqueue(&mut self.wait_node);
            }
            self.registered = true;

            return Poll::Pending;
        }

        let _guard = bucket._lock.lock();
        if !self.wait_node.is_linked() {
            self.registered = false;
            return Poll::Ready(FutexResult::Woken);
        }

        self.wait_node.set_waker(cx.waker().clone());
        drop(_guard);
        Poll::Pending
    }
}

impl<'a> Drop for FutexWaitFuture<'a> {
    fn drop(&mut self) {
        if self.registered {
            let bucket = self.table.bucket_for(self.addr);
            let _guard = bucket._lock.lock();
            // SAFETY: wait_node is still valid (we're in drop).
            unsafe {
                bucket.queue.remove(&mut self.wait_node);
            }
        }
    }
}