use std::{
    cell::UnsafeCell,
    ops::{Deref, DerefMut},
    sync::atomic::{
        AtomicU32,
        Ordering::{Acquire, Relaxed, Release},
    },
};

use atomic_wait::{wait, wake_one};

pub struct Mutex<T> {
    /// State to indicate Lock:
    /// - 0: unlocked
    /// - 1: locked, no other threads waiting
    /// - 2: locked, other threads waiting
    state: AtomicU32,
    value: UnsafeCell<T>,
}

unsafe impl<T> Sync for Mutex<T> where T: Send {}

pub struct MutexGuard<'a, T> {
    pub(crate) mutex: &'a Mutex<T>,
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
        // compare_exchange from 0 to 1:
        // - if success, then state is actually 0(unlocked), get the lock
        // - else, state is 1 or 2 (locked).
        //
        // In that situation, swap 2 into state:
        //  - if state is 1, then it move to 2 now
        //  - if state is 2, nothing happen
        //
        // After swap, wait for state become not 2, and check again,
        // - if state is 0, then we got the locked and move state to 2?
        // - if state is not 0, means other thread got the lock before
        //
        // INFO: We don't know actual number of threads that are waiting,
        // so if one thread get into state 2, then once it get a 0,
        // state needs to become 2 to avoid lost of wait().
        // But if we don't have thread get into state 2, then it's safe
        // to just avoid wait() and wake_one()
        if self.state.compare_exchange(0, 1, Acquire, Relaxed).is_err() {
            lock_contended(&self.state);
        }
        MutexGuard { mutex: self }
    }
}

fn lock_contended(state: &AtomicU32) {
    const SPIN_LIMIT: usize = 100;
    let mut spin_count = 0;

    while state.load(Relaxed) == 1 && spin_count < SPIN_LIMIT {
        spin_count += 1;
        std::hint::spin_loop();
    }

    if state.compare_exchange(0, 1, Acquire, Relaxed).is_ok() {
        return;
    }

    while state.swap(2, Acquire) != 0 {
        wait(state, 2);
    }
}

impl<T> Drop for MutexGuard<'_, T> {
    fn drop(&mut self) {
        // Wake up one of the waiting threads, if any.
        if self.mutex.state.swap(0, Release) == 2 {
            wake_one(&self.mutex.state);
        }
    }
}
