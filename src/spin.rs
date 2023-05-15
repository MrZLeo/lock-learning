use std::cell::UnsafeCell;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering::{Acquire, Release};

pub struct SpinLock<T> {
    locked: AtomicBool,
    value: UnsafeCell<T>,
}

/// Make SpinLock Sync if T is Send
unsafe impl<T> Sync for SpinLock<T> where T: Send {}

impl<T> SpinLock<T> {
    pub const fn new(value: T) -> Self {
        Self {
            locked: AtomicBool::new(false),
            value: UnsafeCell::new(value),
        }
    }

    #[allow(clippy::mut_from_ref)]
    pub fn lock(&self) -> &mut T {
        while self.locked.swap(true, Acquire) {
            // tells the processor that we're spinning,
            // this hint will result in a special instruction that
            // causes the processor core to optimizeits behavior
            std::hint::spin_loop();
        }
        unsafe { &mut *self.value.get() }
    }

    /// # Safety
    /// The &mut T from `lock()` must be gone!
    /// (And no cheating by keeping reference to fields of that T around!)
    pub unsafe fn unlock(&self) {
        self.locked.store(false, Release)
    }
}
