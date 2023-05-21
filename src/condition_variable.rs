use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering::Relaxed};

use atomic_wait::{wake_all, wake_one};

use crate::mutex::MutexGuard;

pub struct Condvar {
    counter: AtomicU32,
}

impl Condvar {
    pub const fn new() -> Self {
        Self {
            counter: AtomicU32::new(0),
        }
    }

    pub fn notify_one(&self) {
            self.counter.fetch_add(1, Relaxed);
            wake_one(&self.counter);
    }

    pub fn notify_all(&self) {
            self.counter.fetch_add(1, Relaxed);
            wake_all(&self.counter);
    }

    pub fn wait<'a, T>(&self, guard: MutexGuard<'a, T>) -> MutexGuard<'a, T> {
        let counter_value = self.counter.load(Relaxed);

        let mutex = guard.mutex;
        drop(guard);

        atomic_wait::wait(&self.counter, counter_value);

        mutex.lock()
    }
}

#[cfg(test)]
mod test {
    use std::{assert_eq, thread, time::Duration};

    use crate::mutex::Mutex;

    use super::Condvar;

    #[test]
    fn condvar() {
        let mutex = Mutex::new(0);
        let condvar = Condvar::new();

        let mut wakeups = 0;

        thread::scope(|s| {
            s.spawn(|| {
                thread::sleep(Duration::from_secs(1));
                *mutex.lock() = 123;
                condvar.notify_one();
            });

            let mut m = mutex.lock();
            while *m < 100 {
                m = condvar.wait(m);
                wakeups += 1;
            }

            assert_eq!(*m, 123);
        });

        // Check we don't spinning, but allow some spurious wake ups.
        assert!(wakeups < 10);
    }
}
