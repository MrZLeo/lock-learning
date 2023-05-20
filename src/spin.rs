use std::cell::UnsafeCell;
use std::ops::{Deref, DerefMut};
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering::{Acquire, Release};

pub struct SpinLock<T> {
    locked: AtomicBool,
    value: UnsafeCell<T>,
}

pub struct SpinGuard<'a, T> {
    lock: &'a SpinLock<T>,
}

impl<T> Deref for SpinGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        // Safety: The very existence of this Guard
        // guarantees we've exclusively locked the lock.
        unsafe { &*self.lock.value.get() }
    }
}

impl<T> DerefMut for SpinGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        // Safety: The very existence of this Guard
        // guarantees we've exclusively locked the lock.
        unsafe { &mut *self.lock.value.get() }
    }
}

impl<T> Drop for SpinGuard<'_, T> {
    fn drop(&mut self) {
        self.lock.locked.store(false, Release)
    }
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

    pub fn lock(&self) -> SpinGuard<T> {
        while self.locked.swap(true, Acquire) {
            // tells the processor that we're spinning,
            // this hint will result in a special instruction that
            // causes the processor core to optimizeits behavior
            std::hint::spin_loop();
        }
        SpinGuard { lock: self }
    }
}

#[cfg(test)]
mod test {
    use std::thread;

    use super::SpinLock;

    #[test]
    fn spin_lock() {
        let x = SpinLock::new(Vec::new());
        thread::scope(|s| {
            s.spawn(|| x.lock().push(1));
            s.spawn(|| {
                let mut g = x.lock();
                g.push(2);
                g.push(2);
            });
        });
        let g = x.lock();
        assert!(g.as_slice() == [1, 2, 2] || g.as_slice() == [2, 2, 1]);
    }
}
