// Kernel object model.
// Reference-counted, accessed through handles. Intrusive refcount in header.

use core::any::Any;
use core::sync::atomic::{AtomicU32, Ordering};

use crate::object::rights::Rights;

/// Unique type identifier for kernel objects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ObjectTypeId(pub u32);

// Well-known type IDs.
pub const TYPE_TASK: ObjectTypeId = ObjectTypeId(1);
pub const TYPE_CHANNEL: ObjectTypeId = ObjectTypeId(2);
pub const TYPE_VMO: ObjectTypeId = ObjectTypeId(3);
pub const TYPE_EVENT: ObjectTypeId = ObjectTypeId(4);
pub const TYPE_PORT: ObjectTypeId = ObjectTypeId(5);
pub const TYPE_INTERRUPT: ObjectTypeId = ObjectTypeId(6);
pub const TYPE_PROCESS: ObjectTypeId = ObjectTypeId(7);
pub const TYPE_THREAD: ObjectTypeId = ObjectTypeId(8);
pub const TYPE_HANDLE_TABLE: ObjectTypeId = ObjectTypeId(9);

/// Header embedded at the start of every kernel object.
pub struct ObjectHeader {
    refcount: AtomicU32,  // starts at 1
    type_id: ObjectTypeId,
    max_rights: Rights,
    signals: AtomicU32,
}

impl ObjectHeader {
    pub const fn new(type_id: ObjectTypeId, max_rights: Rights) -> Self {
        Self {
            refcount: AtomicU32::new(1),
            type_id,
            max_rights,
            signals: AtomicU32::new(0),
        }
    }

    /// Increment refcount.
    pub fn add_ref(&self) {
        let old = self.refcount.fetch_add(1, Ordering::Relaxed);
        debug_assert!(old > 0, "add_ref on destroyed object");
        debug_assert!(old < u32::MAX - 1, "reference count overflow");
    }

    /// Decrement refcount. If returns 1, caller must destroy.
    pub fn release(&self) -> u32 {
        let old = self.refcount.fetch_sub(1, Ordering::Release);
        debug_assert!(old > 0, "release on already-destroyed object");
        old
    }

    /// Returns the current reference count.
    pub fn refcount(&self) -> u32 {
        self.refcount.load(Ordering::Acquire)
    }

    pub fn type_id(&self) -> ObjectTypeId {
        self.type_id
    }

    pub fn max_rights(&self) -> Rights {
        self.max_rights
    }

    pub fn set_signal(&self, signal: u32) {
        self.signals.fetch_or(signal, Ordering::Release);
    }

    pub fn clear_signal(&self, signal: u32) {
        self.signals.fetch_and(!signal, Ordering::Release);
    }

    pub fn signals(&self) -> u32 {
        self.signals.load(Ordering::Acquire)
    }
}

/// Trait for kernel objects.
pub trait KernelObject: Any + Send + Sync {
    fn header(&self) -> &ObjectHeader;

    fn type_id(&self) -> ObjectTypeId {
        self.header().type_id
    }

    /// Called when the last handle is closed. Override for cleanup.
    fn on_last_handle_close(&self) {}

    /// Called when refcount drops to zero. Release all resources here.
    fn on_destroy(&self) {}

    fn as_any(&self) -> &dyn Any;
}

/// Ref-counted pointer to a kernel object.
/// SAFETY: raw pointer with manual refcounting.
pub struct KoRef {
    ptr: *const dyn KernelObject,
}

// SAFETY: KernelObject requires Send + Sync, refcount ensures object outlives KoRefs.
unsafe impl Send for KoRef {}
unsafe impl Sync for KoRef {}

impl KoRef {
    /// Create a KoRef, incrementing the refcount.
    pub fn new(obj: &dyn KernelObject) -> Self {
        obj.header().add_ref();
        Self { ptr: obj as *const dyn KernelObject }
    }

    pub fn get(&self) -> &dyn KernelObject {
        // SAFETY: refcount guarantees the object is alive.
        unsafe { &*self.ptr }
    }

    pub fn header(&self) -> &ObjectHeader {
        self.get().header()
    }

    pub fn type_id(&self) -> ObjectTypeId {
        self.get().type_id()
    }

    pub fn downcast_ref<T: KernelObject>(&self) -> Option<&T> {
        self.get().as_any().downcast_ref::<T>()
    }
}

impl Clone for KoRef {
    fn clone(&self) -> Self {
        self.header().add_ref();
        Self { ptr: self.ptr }
    }
}

impl Drop for KoRef {
    fn drop(&mut self) {
        let old = self.header().release();
        if old == 1 {
            self.get().on_destroy();
        }
    }
}
