// Chase-Lev work-stealing deque [Chase & Lev, SPAA 2005].
//
// Owner (Worker) uses LIFO push/pop; thieves (Stealer) use FIFO steal.
// When idle, a core steals half a victim's deque.

use core::sync::atomic::{AtomicIsize, AtomicPtr, Ordering};

const MIN_CAPACITY: usize = 64;

const MAX_STEAL_BATCH: usize = 128;

pub struct Worker<T: Copy> {
    inner: *mut DequeInner<T>,
}

pub struct Stealer<T: Copy> {
    inner: *mut DequeInner<T>,
}

struct DequeInner<T: Copy> {
    bottom: AtomicIsize,
    top: AtomicIsize,
    buffer: AtomicPtr<DequeBuffer<T>>,
}

struct DequeBuffer<T: Copy> {
    data: *mut T,
    log_cap: u32,
    cap: usize,
}

unsafe impl<T: Copy> Send for DequeInner<T> {}
unsafe impl<T: Copy> Sync for DequeInner<T> {}

impl<T: Copy> DequeBuffer<T> {
    fn new(log_cap: u32) -> *mut Self {
        let cap = 1usize << log_cap;
        let layout = core::alloc::Layout::array::<T>(cap).expect("deque capacity overflow");
        let data = unsafe { alloc::alloc::alloc(layout) } as *mut T;
        assert!(!data.is_null(), "deque allocation failed");
        Box::into_raw(Box::new(DequeBuffer { data, log_cap, cap }))
    }

    fn mask(&self) -> usize {
        self.cap - 1
    }

    unsafe fn get(&self, i: isize) -> T {
        *self.data.add(i as usize & self.mask())
    }

    unsafe fn put(&self, i: isize, val: T) {
        *self.data.add(i as usize & self.mask()) = val;
    }
}

pub fn deque<T: Copy>() -> (Worker<T>, Stealer<T>) {
    let inner = Box::into_raw(Box::new(DequeInner {
        bottom: AtomicIsize::new(0),
        top: AtomicIsize::new(0),
        buffer: AtomicPtr::new(DequeBuffer::new(MIN_CAPACITY.trailing_zeros())),
    }));
    (Worker { inner }, Stealer { inner })
}

impl<T: Copy> Worker<T> {
    pub fn push(&self, val: T) {
        let inner = unsafe { &*self.inner };
        let b = inner.bottom.load(Ordering::Relaxed);
        let t = inner.top.load(Ordering::Acquire);
        let buf_ptr = inner.buffer.load(Ordering::Relaxed);
        let buf = unsafe { &*buf_ptr };

        let size = b - t;
        if size >= buf.cap as isize {
            self.grow(buf_ptr, t, b);
            let buf_ptr = inner.buffer.load(Ordering::Relaxed);
            let buf = unsafe { &*buf_ptr };
            unsafe { buf.put(b, val) };
        } else {
            unsafe { buf.put(b, val) };
        }

        core::sync::atomic::fence(Ordering::Release);
        inner.bottom.store(b + 1, Ordering::Relaxed);
    }

    pub fn pop(&self) -> Option<T> {
        let inner = unsafe { &*self.inner };
        let b = inner.bottom.load(Ordering::Relaxed) - 1;
        inner.bottom.store(b, Ordering::Relaxed);

        core::sync::atomic::fence(Ordering::SeqCst);

        let t = inner.top.load(Ordering::Relaxed);
        if t <= b {
            let buf_ptr = inner.buffer.load(Ordering::Relaxed);
            let buf = unsafe { &*buf_ptr };
            let val = unsafe { buf.get(b) };
            if t == b {
                match inner.top.compare_exchange(
                    t,
                    t + 1,
                    Ordering::SeqCst,
                    Ordering::Relaxed,
                ) {
                    Ok(_) => {
                        inner.bottom.store(b + 1, Ordering::Relaxed);
                        Some(val)
                    }
                    Err(_) => {
                        inner.bottom.store(b + 1, Ordering::Relaxed);
                        None
                    }
                }
            } else {
                Some(val)
            }
        } else {
            inner.bottom.store(b + 1, Ordering::Relaxed);
            None
        }
    }

    pub fn len(&self) -> usize {
        let inner = unsafe { &*self.inner };
        let b = inner.bottom.load(Ordering::Relaxed);
        let t = inner.top.load(Ordering::Relaxed);
        (b - t).max(0) as usize
    }

    fn grow(&self, old_buf_ptr: *mut DequeBuffer<T>, t: isize, b: isize) {
        let inner = unsafe { &*self.inner };
        let old_buf = unsafe { &*old_buf_ptr };
        let new_log_cap = old_buf.log_cap + 1;
        let new_buf_ptr = DequeBuffer::new(new_log_cap);
        let new_buf = unsafe { &*new_buf_ptr };

        for i in t..b {
            unsafe {
                let val = old_buf.get(i);
                new_buf.put(i, val);
            }
        }

        inner.buffer.store(new_buf_ptr, Ordering::Release);
    }
}

impl<T: Copy> Stealer<T> {
    pub fn steal(&self) -> Option<T> {
        let inner = unsafe { &*self.inner };
        loop {
            let buf_ptr = inner.buffer.load(Ordering::Acquire);
            let buf = unsafe { &*buf_ptr };
            let t = inner.top.load(Ordering::Acquire);
            core::sync::atomic::fence(Ordering::SeqCst);
            let b = inner.bottom.load(Ordering::Acquire);

            if t >= b {
                return None;
            }

            let val = unsafe { buf.get(t) };

            if inner.buffer.load(Ordering::Acquire) != buf_ptr {
                continue;
            }

            match inner.top.compare_exchange(
                t,
                t + 1,
                Ordering::SeqCst,
                Ordering::Relaxed,
            ) {
                Ok(_) => return Some(val),
                Err(_) => return None,
            }
        }
    }

    pub fn steal_batch(&self, out: &mut [T]) -> usize {
        let inner = unsafe { &*self.inner };
        loop {
            let buf_ptr = inner.buffer.load(Ordering::Acquire);
            let buf = unsafe { &*buf_ptr };
            let t = inner.top.load(Ordering::Acquire);
            core::sync::atomic::fence(Ordering::SeqCst);
            let b = inner.bottom.load(Ordering::Acquire);

            let available = (b - t).max(0) as usize;
            if available == 0 {
                return 0;
            }

            let to_steal = available.min(out.len()).min(MAX_STEAL_BATCH);

            for i in 0..to_steal {
                out[i] = unsafe { buf.get(t + i as isize) };
            }

            if inner.buffer.load(Ordering::Acquire) != buf_ptr {
                continue;
            }

            match inner.top.compare_exchange(
                t,
                t + to_steal as isize,
                Ordering::SeqCst,
                Ordering::Relaxed,
            ) {
                Ok(_) => to_steal,
                Err(_) => 0,
            }
        }
    }

    pub fn len(&self) -> usize {
        let inner = unsafe { &*self.inner };
        let t = inner.top.load(Ordering::Relaxed);
        let b = inner.bottom.load(Ordering::Relaxed);
        (b - t).max(0) as usize
    }
}
