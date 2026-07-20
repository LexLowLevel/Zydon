// Cache-line padding to eliminate false sharing.
//
// On x86-64 and AArch64, a cache line is 64 bytes. Placing two atomics
// that are frequently written by different cores on the same cache line
// forces unnecessary cache-coherence traffic (false sharing). Wrapping
// each in CachePadded forces them onto separate lines.

use core::ops::{Deref, DerefMut};

const CACHE_LINE_SIZE: usize = 64;

#[repr(align(64))]
pub struct CachePadded<T> {
    value: T,
}

impl<T> CachePadded<T> {
    pub const fn new(value: T) -> Self {
        Self { value }
    }

    pub fn into_inner(self) -> T {
        self.value
    }
}

impl<T> Deref for CachePadded<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.value
    }
}

impl<T> DerefMut for CachePadded<T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.value
    }
}