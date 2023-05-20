use std::{
    cell::UnsafeCell,
    ops::{Deref, DerefMut},
    sync::atomic::{
        AtomicU32,
        Ordering::{Acquire, Release},
    },
};

use atomic_wait::{wait, wake_one};

pub struct Mutex<T> {
    /// State to indicate Lock:
    /// - 0: unlocked
    /// - 1: locked
    state: AtomicU32,
    value: UnsafeCell<T>,
}

unsafe impl<T> Sync for Mutex<T> where T: Send {}

pub struct MutexGuard<'a, T> {
    mutex: &'a Mutex<T>,
}

impl<T> Deref for MutexGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        unsafe { &*self.mutex.value.get() }
    }
}

impl<T> DerefMut for MutexGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *self.mutex.value.get() }
    }
}

impl<T> Mutex<T> {
    pub const fn new(value: T) -> Self {
        Self {
            state: AtomicU32::new(0),
            value: UnsafeCell::new(value),
        }
    }

    pub fn lock(&self) -> MutexGuard<T> {
        // Set the state to 1
        while self.state.swap(1, Acquire) == 1 {
            // If it was already locked,
            // then wait unless the state is on longer 1
            wait(&self.state, 1);
        }
        MutexGuard { mutex: self }
    }
}

impl<T> Drop for MutexGuard<'_, T> {
    fn drop(&mut self) {
        // Set the state back to 0
        self.mutex.state.store(0, Release);

        // Wake up one of the waiting threads, if any.
        wake_one(&self.mutex.state);
    }
}
