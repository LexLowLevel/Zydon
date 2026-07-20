// Userspace-mapped IPC fastpath.
//
// Kernel maps channel SPSC ring buffers directly into userspace.
// try_send/try_recv become pure atomic ops (~20ns vs ~200ns syscall).
// Kernel only involved for full/empty rings (park/wake).

use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use crate::primitives::cache_padded::CachePadded;

/// Memory layout constants. Do not reorder.
///   0x00  head        (u64, producer-written, CachePadded)
///   0x40  tail        (u64, consumer-written, CachePadded)
///   0x80  capacity    (u64, read-only after creation)
///   0x88  slot_stride (u64, read-only after creation)
///   0x90  signal      (u32, kernel-to-user wake)
///   0xC0  ring data   (page-aligned, capacity * slot_stride bytes)
const OFF_HEAD: usize = 0x00;
const OFF_TAIL: usize = 0x40;
const OFF_CAPACITY: usize = 0x80;
const OFF_SLOT_STRIDE: usize = 0x88;
const OFF_SIGNAL: usize = 0x90;
const OFF_RING_DATA: usize = 0xC0;

/// Userspace-visible ring buffer descriptor, mapped at a fixed virtual address.
#[repr(C, align(4096))]
pub struct FastpathBuffer {
    pub head: CachePadded<AtomicU64>,         // (0x00) producer-written
    pub tail: CachePadded<AtomicU64>,         // (0x40) consumer-written
    pub capacity: u64,                        // (0x80) immutable after creation
    pub slot_stride: u64,                     // (0x88) immutable after creation
    pub signal: AtomicU32,                    // (0x90) kernel-to-user wake
    _pad: [u8; OFF_RING_DATA - OFF_SIGNAL - 4],
    _ring_data_marker: [u8; 0],
}

// SAFETY: FastpathBuffer is designed for cross-address-space sharing.
// head is producer-only, tail is consumer-only, signal is kernel-to-user.
unsafe impl Send for FastpathBuffer {}
unsafe impl Sync for FastpathBuffer {}

impl FastpathBuffer {
    pub const fn new(capacity: u64, slot_stride: u64) -> Self {
        Self {
            head: CachePadded::new(AtomicU64::new(0)),
            tail: CachePadded::new(AtomicU64::new(0)),
            capacity,
            slot_stride,
            signal: AtomicU32::new(0),
            _pad: [0; OFF_RING_DATA - OFF_SIGNAL - 4],
            _ring_data_marker: [0; 0],
        }
    }
}

/// Fastpath send: enqueue into the ring from userspace.
///
/// Returns 0 on success, -1 if full.
///
/// # Safety
/// - `buf` must point to a valid, page-aligned FastpathBuffer.
/// - `payload` must point to at least `len` readable bytes.
/// - SPSC invariant: no concurrent producer.
#[cfg(target_arch = "x86_64")]
#[naked]
pub unsafe extern "C" fn fastpath_send(
    buf: *mut FastpathBuffer,
    payload: *const u8,
    len: u64,
) -> i64 {
    // rdi = buf, rsi = payload, rdx = len
    //
    // Callee-saved: r12 = len, r13 = payload, r14 = buf.
    // 1. Load head, tail, capacity. 2. Full check. 3. lock xadd claim.
    // 4. Compute slot addr, copy. 5. sfence.
    core::arch::asm!(
        // Save callee-saved registers we use.
        "push r12",
        "push r13",
        "push r14",
        "push rbx",

        // Save arguments into callee-saved regs.
        "mov r14, rdi",                         // buf
        "mov r13, rsi",                         // payload
        "mov r12, rdx",                         // len

        // Load buf->head, buf->tail, buf->capacity.
        "mov rax, [r14 + {off_head}]",          // head
        "mov rbx, [r14 + {off_tail}]",          // tail
        "mov rcx, [r14 + {off_capacity}]",      // capacity

        // Check if full: (head - tail) >= capacity.
        "mov r8, rax",                          // r8 = head
        "sub r8, rbx",                          // r8 = head - tail
        "cmp r8, rcx",
        "jae 2f",                               // full => return -1

        // Atomically claim slot: lock xadd [buf->head], 1.
        // rax gets the old head value (our slot index).
        "mov rdx, 1",
        "lock xadd [r14 + {off_head}], rdx",
        // rdx = old head (our claimed index).

        // Double-check we didn't race past capacity.
        // (A concurrent producer may have advanced head.)
        "mov rax, rdx",
        "sub rax, rbx",                         // rax = old_head - tail
        "cmp rax, rcx",
        "jb 3f",

        // Race loser: release slot by decrementing head.
        "mov rax, -1",
        "lock xadd [r14 + {off_head}], rax",    // actually add -1
        "jmp 4f",

        "2:",
        // Ring full.
        "mov rax, -1",
        "jmp 4f",

        "3:",
        // Compute slot address.
        // slot_ptr = buf + OFF_RING_DATA + (old_head % capacity) * slot_stride
        "mov rax, rdx",                         // old_head
        "xor rdx, rdx",
        "div rcx",                              // rax = old_head / cap, rdx = old_head % cap
        "mov r8, [r14 + {off_slot_stride}]",    // slot_stride
        "imul rdx, r8",                         // byte offset within ring data
        "lea rdi, [r14 + {off_ring_data}]",     // ring data base
        "add rdi, rdx",                         // slot pointer

        // Copy payload to slot: memcpy(slot, payload, len).
        "mov rsi, r13",                         // payload
        "mov rcx, r12",                         // len
        "rep movsb",

        // Memory fence to make the write visible to the consumer.
        "sfence",

        "mov rax, 0",                           // success

        "4:",
        // Restore callee-saved registers.
        "pop rbx",
        "pop r14",
        "pop r13",
        "pop r12",
        "ret",

        off_head = const OFF_HEAD,
        off_tail = const OFF_TAIL,
        off_capacity = const OFF_CAPACITY,
        off_slot_stride = const OFF_SLOT_STRIDE,
        off_ring_data = const OFF_RING_DATA,

        options(noreturn)
    );
}

/// Fastpath receive: dequeue from the ring into userspace.
///
/// Returns 0 on success, -1 if empty.
///
/// # Safety
/// - `buf` must point to a valid, page-aligned FastpathBuffer.
/// - `out` must point to at least `slot_stride` writable bytes.
/// - `out_len` must point to a writable u64.
/// - SPSC invariant: no concurrent consumer.
#[cfg(target_arch = "x86_64")]
#[naked]
pub unsafe extern "C" fn fastpath_recv(
    buf: *mut FastpathBuffer,
    out: *mut u8,
    out_len: *mut u64,
) -> i64 {
    // rdi = buf, rsi = out, rdx = out_len
    core::arch::asm!(
        // Save callee-saved registers.
        "push r12",
        "push r13",
        "push r14",
        "push rbx",

        // Save arguments.
        "mov r14, rdi",                         // buf
        "mov r13, rsi",                         // out
        "mov r12, rdx",                         // out_len (pointer)

        // Load buf->head, buf->tail, buf->capacity.
        "mov rax, [r14 + {off_head}]",          // head
        "mov rbx, [r14 + {off_tail}]",          // tail
        "mov rcx, [r14 + {off_capacity}]",      // capacity

        // Check if empty: tail == head.
        "cmp rbx, rax",
        "je 4f",                                // empty => return -1

        // Atomically claim slot: lock xadd [buf->tail], 1.
        // rbx gets the old tail value (our slot index).
        "mov rdx, 1",
        "lock xadd [r14 + {off_tail}], rdx",
        // rdx = old tail (our claimed index).

        // Double-check we didn't race past head.
        "cmp rdx, rax",
        "jb 5f",

        // Race loser: release slot by decrementing tail.
        "mov rax, -1",
        "lock xadd [r14 + {off_tail}], rax",
        "jmp 6f",

        "4:",
        // Ring empty.
        "mov rax, -1",
        "jmp 6f",

        "5:",
        // Compute slot address.
        "mov rax, rdx",                         // old_tail
        "xor rdx, rdx",
        "div rcx",                              // rax = old_tail / cap, rdx = old_tail % cap
        "mov r8, [r14 + {off_slot_stride}]",    // slot_stride
        "imul rdx, r8",                         // byte offset
        "lea rsi, [r14 + {off_ring_data}]",     // ring data base
        "add rsi, rdx",                         // slot pointer (source)

        // Copy slot to output buffer: memcpy(out, slot, slot_stride).
        "mov rdi, r13",                         // out (dest)
        "mov rcx, r8",                          // slot_stride (count)
        "rep movsb",

        // Write the length to *out_len.
        "mov [r12], r8",                        // *out_len = slot_stride

        // Acknowledge the signal word (clear it).
        "mov dword ptr [r14 + {off_signal}], 0",

        "mov rax, 0",                           // success

        "6:",
        // Restore callee-saved registers.
        "pop rbx",
        "pop r14",
        "pop r13",
        "pop r12",
        "ret",

        off_head = const OFF_HEAD,
        off_tail = const OFF_TAIL,
        off_capacity = const OFF_CAPACITY,
        off_slot_stride = const OFF_SLOT_STRIDE,
        off_signal = const OFF_SIGNAL,
        off_ring_data = const OFF_RING_DATA,

        options(noreturn)
    );
}

/// AArch64 fastpath send. Uses ldaxr/stlxr for slot claim.
#[cfg(target_arch = "aarch64")]
#[naked]
pub unsafe extern "C" fn fastpath_send(
    buf: *mut FastpathBuffer,
    payload: *const u8,
    len: u64,
) -> i64 {
    core::arch::asm!(
        // x0 = buf, x1 = payload, x2 = len
        "stp x29, x30, [sp, #-16]!",
        "stp x19, x20, [sp, #-16]!",
        "stp x21, x22, [sp, #-16]!",

        "mov x19, x0",                         // buf
        "mov x20, x1",                         // payload
        "mov x21, x2",                         // len

        // Load head and tail.
        "ldar x3, [x19]",                      // head (acquire)
        "ldr x4, [x19, #64]",                  // tail
        "ldr x5, [x19, #128]",                 // capacity

        // Check if full.
        "sub x7, x3, x4",                      // head - tail
        "cmp x7, x5",
        "b.ge 6f",

        // Claim slot via LL/SC.
        "7:",
        "ldaxr x7, [x19]",                     // load head exclusive
        "add x8, x7, #1",
        "stlxr w9, x8, [x19]",                 // store head release
        "cbnz w9, 7b",                         // retry on contention

        // Compute slot address.
        "ldr x6, [x19, #136]",                 // slot_stride
        "udiv x9, x7, x5",                     // head / capacity
        "msub x9, x9, x5, x7",                 // head % capacity
        "mul x9, x9, x6",                      // byte offset
        "add x10, x19, #192",                  // ring data start (OFF_RING_DATA)
        "add x10, x10, x9",                    // slot pointer

        // Copy payload to slot.
        "mov x0, x10",
        "mov x1, x20",
        "mov x2, x21",
        "8:",
        "ldrb w3, [x1], #1",
        "strb w3, [x0], #1",
        "subs x2, x2, #1",
        "b.ne 8b",

        // Data memory barrier.
        "dmb ish",

        "mov x0, #0",
        "b 9f",

        "6:",
        "mov x0, #-1",

        "9:",
        "ldp x21, x22, [sp], #16",
        "ldp x19, x20, [sp], #16",
        "ldp x29, x30, [sp], #16",
        "ret",

        options(noreturn)
    );
}

/// Kernel-side: signal a parked userspace task.
pub fn fastpath_signal(buf: &FastpathBuffer) {
    buf.signal.store(1, Ordering::Release);
}

/// Userspace-side: spin briefly, then WFI/HLT.
pub fn fastpath_wait(buf: &FastpathBuffer) {
    // Spin; signal may arrive quickly.
    for _ in 0..64 {
        if buf.signal.load(Ordering::Acquire) != 0 {
            return;
        }
        core::hint::spin_loop();
    }

    // Low-power wait.
    #[cfg(target_arch = "x86_64")]
    unsafe { core::arch::asm!("sti; hlt"); }

    #[cfg(target_arch = "aarch64")]
    unsafe { core::arch::asm!("wfi"); }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::mem;

    #[test]
    fn buffer_layout_matches_assembly() {
        // Verify that the struct offsets match the assembly constants.
        let buf = FastpathBuffer::new(64, 128);
        let base = &buf as *const _ as usize;

        assert_eq!((&buf.head as *const _ as usize) - base, OFF_HEAD);
        assert_eq!((&buf.tail as *const _ as usize) - base, OFF_TAIL);
        assert_eq!((&buf.capacity as *const _ as usize) - base, OFF_CAPACITY);
        assert_eq!((&buf.slot_stride as *const _ as usize) - base, OFF_SLOT_STRIDE);
        assert_eq!((&buf.signal as *const _ as usize) - base, OFF_SIGNAL);
    }

    #[test]
    fn buffer_is_page_aligned() {
        assert_eq!(mem::align_of::<FastpathBuffer>(), 4096);
    }
}
