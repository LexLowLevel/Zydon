// IPC message.
//
// Fixed header + inline payload (≤256B) + handle array.
// Fits in one cache line for small control messages.
// Large payloads use zero-copy VMO path.

use core::fmt;

use crate::object::handle::Handle;

/// Maximum inline payload size.
pub const MAX_INLINE_PAYLOAD: usize = 256;

/// Maximum handles per message.
pub const MAX_HANDLES: usize = 16;

pub mod flags {
    pub const INLINE_PAYLOAD: u16 = 1 << 0;
    pub const ZERO_COPY_VMO: u16 = 1 << 1;
    pub const WANT_REPLY: u16 = 1 << 2;   // sender expects a reply
    pub const IS_REPLY: u16 = 1 << 3;     // this is a reply message
}

/// Fixed layout for ABI stability.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct MessageHeader {
    pub sender: crate::task::task_id::TaskId,
    pub tx_id: u64,
    pub payload_size: u32,
    pub num_handles: u16,
    pub flags: u16,
}

impl MessageHeader {
    pub const fn new(
        sender: crate::task::task_id::TaskId,
        tx_id: u64,
        payload_size: u32,
        num_handles: u16,
        flags: u16,
    ) -> Self {
        Self {
            sender,
            tx_id,
            payload_size,
            num_handles,
            flags,
        }
    }
}

impl fmt::Debug for MessageHeader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MessageHeader")
            .field("sender", &self.sender)
            .field("tx_id", &self.tx_id)
            .field("payload_size", &self.payload_size)
            .field("num_handles", &self.num_handles)
            .field("flags", &format_args!("0x{:04x}", self.flags))
            .finish()
    }
}

/// Kernel IPC message with inline payload and handles.
pub struct Message {
    pub header: MessageHeader,
    payload: [u8; MAX_INLINE_PAYLOAD],
    handles: [Handle; MAX_HANDLES],
}

impl Message {
    /// Construct with inline payload.
    pub fn new_inline(
        sender: crate::task::task_id::TaskId,
        tx_id: u64,
        data: &[u8],
        handles: &[Handle],
    ) -> Result<Self, MessageError> {
        if data.len() > MAX_INLINE_PAYLOAD {
            return Err(MessageError::PayloadTooLarge);
        }
        if handles.len() > MAX_HANDLES {
            return Err(MessageError::TooManyHandles);
        }

        let mut payload = [0u8; MAX_INLINE_PAYLOAD];
        payload[..data.len()].copy_from_slice(data);

        let mut harray = [Handle::INVALID; MAX_HANDLES];
        harray[..handles.len()].copy_from_slice(handles);

        Ok(Self {
            header: MessageHeader::new(
                sender,
                tx_id,
                data.len() as u32,
                handles.len() as u16,
                flags::INLINE_PAYLOAD,
            ),
            payload,
            handles: harray,
        })
    }

    /// Construct with VMO reference for zero-copy.
    pub fn new_zero_copy(
        sender: crate::task::task_id::TaskId,
        tx_id: u64,
        vmo_handle: Handle,
        vmo_offset: u64,
        vmo_size: u64,
        handles: &[Handle],
    ) -> Result<Self, MessageError> {
        if handles.len() + 1 > MAX_HANDLES {
            return Err(MessageError::TooManyHandles);
        }

        // Encode VMO reference: [handle: u64][offset: u64][size: u64]
        let mut payload = [0u8; MAX_INLINE_PAYLOAD];
        let vmo_ref = VmoReference {
            handle: vmo_handle.to_raw(),
            offset: vmo_offset,
            size: vmo_size,
        };
        let bytes = unsafe {
            core::slice::from_raw_parts(
                &vmo_ref as *const VmoReference as *const u8,
                core::mem::size_of::<VmoReference>(),
            )
        };
        payload[..bytes.len()].copy_from_slice(bytes);

        let mut harray = [Handle::INVALID; MAX_HANDLES];
        harray[0] = vmo_handle;
        if !handles.is_empty() {
            harray[1..=handles.len()].copy_from_slice(handles);
        }

        Ok(Self {
            header: MessageHeader::new(
                sender,
                tx_id,
                bytes.len() as u32,
                (handles.len() + 1) as u16,
                flags::ZERO_COPY_VMO,
            ),
            payload,
            handles: harray,
        })
    }

    pub fn payload(&self) -> &[u8] {
        &self.payload[..self.header.payload_size as usize]
    }

    pub fn handles(&self) -> &[Handle] {
        &self.handles[..self.header.num_handles as usize]
    }

    /// Transfer handles out of the message (for receiver).
    pub fn take_handles(&mut self) -> heapless::Vec<Handle, MAX_HANDLES> {
        let mut result = heapless::Vec::new();
        for i in 0..self.header.num_handles as usize {
            let _ = result.push(self.handles[i]);
            self.handles[i] = Handle::INVALID;
        }
        self.header.num_handles = 0;
        result
    }

    pub fn is_zero_copy(&self) -> bool {
        (self.header.flags & flags::ZERO_COPY_VMO) != 0
    }

    /// Extract VMO reference from a zero-copy message.
    pub fn vmo_reference(&self) -> Option<VmoReference> {
        if !self.is_zero_copy() {
            return None;
        }
        if core::mem::size_of::<VmoReference>() > self.header.payload_size as usize {
            return None;
        }
        Some(unsafe {
            core::ptr::read(self.payload.as_ptr() as *const VmoReference)
        })
    }
}

/// VMO region reference for zero-copy transfer.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VmoReference {
    pub handle: u64,
    pub offset: u64,
    pub size: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageError {
    PayloadTooLarge,
    TooManyHandles,
}

// heapless Vec (see wait_queue)
mod heapless {
    pub struct Vec<T, const N: usize> {
        buf: [core::mem::MaybeUninit<T>; N],
        len: usize,
    }

    impl<T, const N: usize> Vec<T, N> {
        pub fn new() -> Self {
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
    }
}