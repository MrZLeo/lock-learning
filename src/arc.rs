use std::{
    cell::UnsafeCell,
    mem::ManuallyDrop,
    ops::Deref,
    ptr::NonNull,
    sync::atomic::{
        fence, AtomicUsize,
        Ordering::{Acquire, Relaxed, Release},
    },
};

struct ArcData<T> {
    /// Number of `Arc`s.
    data_ref_count: AtomicUsize,
    /// Number of `Weak`s, plus one if there are any `Arc`s.
    alloc_ref_count: AtomicUsize,
    /// The data. Dropped if there are only weak pointers left.
    data: UnsafeCell<ManuallyDrop<T>>,
}

pub struct Arc<T> {
    ptr: NonNull<ArcData<T>>,
}

unsafe impl<T: Send + Sync> Send for Arc<T> {}
unsafe impl<T: Send + Sync> Sync for Arc<T> {}

pub struct Weak<T> {
    ptr: NonNull<ArcData<T>>,
}

unsafe impl<T: Send + Sync> Send for Weak<T> {}
unsafe impl<T: Send + Sync> Sync for Weak<T> {}

impl<T> Weak<T> {
    fn data(&self) -> &ArcData<T> {
        unsafe { self.ptr.as_ref() }
    }

    pub fn upgrade(&self) -> Option<Arc<T>> {
        let mut n = self.data().data_ref_count.load(Relaxed);
        loop {
            if n == 0 {
                return None;
            }
            assert!(n <= usize::MAX / 2);
            if let Err(e) =
                self.data()
                    .data_ref_count
                    .compare_exchange_weak(n, n + 1, Relaxed, Relaxed)
            {
                n = e;
                continue;
            }
            return Some(Arc { ptr: self.ptr });
        }
    }
}

impl<T> Arc<T> {
    pub fn new(data: T) -> Self {
        Self {
            ptr: NonNull::from(Box::leak(Box::new(ArcData {
                data_ref_count: AtomicUsize::new(1),
                // alloc_ref_count is 1 when new the first `Arc`,
                // which represents all `Arc`.
                alloc_ref_count: AtomicUsize::new(1),
                data: UnsafeCell::new(ManuallyDrop::new(data)),
            }))),
        }
    }

    /// This function must be used like:
    ///
    /// ```ignore
    /// Arc::get_mut(&mut a);
    /// ```
    ///
    /// since `T` could be types that implement `Deref`, which
    /// will cause ambiguity in dereferencing `Arc` or `T` if we allow:
    ///
    /// ```ignore
    /// a.get_mut(); // dereference T or Arc?
    /// ```
    ///
    pub fn get_mut(arc: &mut Self) -> Option<&mut T> {
        // Acquire matches Weak::drop's Release decrement,
        // to make sure any upgraded pointer are visible
        // in the next `data_ref_count.load()`
        //
        // Swap usize::Max to alloc_ref_count to make sure
        // no upgrade can happen until we finish.
        if arc
            .data()
            .alloc_ref_count
            .compare_exchange(1, usize::MAX, Acquire, Relaxed)
            .is_err()
        {
            return None;
        }

        let is_unique = arc.data().data_ref_count.load(Relaxed) == 1;

        // Release matches Acquire increment in `downgrade`,
        // to make sure any changes to the `data_ref_count` that
        // come after `downgrade` don't change the is_unique above
        arc.data().alloc_ref_count.store(1, Release);

        if !is_unique {
            return None;
        }
        // Acquire to match Arc::drop's Release decrement,
        // to make sure nothing else is accessing the data.
        fence(Acquire);

        // Safety: We only have one Arc and no Weak
        unsafe { Some(&mut *arc.data().data.get()) }
    }

    pub fn downgrade(arc: &Self) -> Weak<T> {
        let mut n = arc.data().alloc_ref_count.load(Relaxed);
        loop {
            if n == usize::MAX {
                std::hint::spin_loop();
                n = arc.data().alloc_ref_count.load(Relaxed);
                continue;
            }
            assert!(n <= usize::MAX / 2);

            // Acquire synchronises with `get_mut`'s release-store
            if let Err(e) =
                arc.data()
                    .alloc_ref_count
                    .compare_exchange_weak(n, n + 1, Acquire, Relaxed)
            {
                n = e;
                continue;
            }
            return Weak { ptr: arc.ptr };
        }
    }

    fn data(&self) -> &ArcData<T> {
        unsafe { self.ptr.as_ref() }
    }
}

impl<T> Deref for Arc<T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        // Safety: Since there's an Arc to the data,
        // the data exists and may be shared.
        unsafe { &*self.data().data.get() }
    }
}

impl<T> Clone for Weak<T> {
    fn clone(&self) -> Self {
        // Simple way to handle overflows
        if self.data().alloc_ref_count.fetch_add(1, Relaxed) > usize::MAX / 2 {
            std::process::abort();
        }

        // Just move `self.ptr` into new Arc because NonNull is `Copy`
        Weak { ptr: self.ptr }
    }
}

impl<T> Clone for Arc<T> {
    fn clone(&self) -> Self {
        if self.data().data_ref_count.fetch_add(1, Relaxed) > usize::MAX / 2 {
            std::process::abort();
        }
        Arc { ptr: self.ptr }
    }
}

impl<T> Drop for Weak<T> {
    fn drop(&mut self) {
        // We need to guarantee last fetch of `ref_count`
        // **happens after** previous operations.
        // In other words, previous store and operations before store
        // must **happens before** last fetch.
        // So We need `Release` to store and `Acquire` to fetch,
        // therefore `AcqRel` should be used here
        //
        // ```
        // if self.data().ref_count.fetch_sub(1, AcqRel) == 1 {
        //     unsafe {
        //         drop(Box::from_raw(self.ptr.as_ptr()));
        //     }
        // }
        // ```
        //
        // However, We just need the `Acquire` for the last one fetch,
        // other fetch can be `Relaxed`, so `fence()` is a better choice
        //
        if self.data().alloc_ref_count.fetch_sub(1, Release) == 1 {
            fence(Acquire);
            unsafe {
                drop(Box::from_raw(self.ptr.as_ptr()));
            }
        }
    }
}

impl<T> Drop for Arc<T> {
    fn drop(&mut self) {
        if self.data().data_ref_count.fetch_sub(1, Relaxed) == 1 {
            fence(Acquire);

            // Safety: The data reference counter is zero,
            // so nothing will access it.
            unsafe {
                // Take the ownership of T inside Option,
                // and it's lifetime ended here,
                // which trigger the `Drop::drop()` of T
                ManuallyDrop::drop(&mut *self.data().data.get());
            }

            // Now that there's no `Arc<T>`s left,
            // drop the implicit weak pointer that represented all `Arc`
            drop(Weak { ptr: self.ptr })
        }
    }
}

#[cfg(test)]
mod test {
    #[deny(warnings)]
    use std::sync::atomic::AtomicUsize;
    use std::sync::atomic::Ordering::Relaxed;

    use crate::arc::Arc;

    macro_rules! init_detect_drop {
        () => {
            static NUM_DROPS: AtomicUsize = AtomicUsize::new(0);

            struct DetectDrop;

            impl Drop for DetectDrop {
                fn drop(&mut self) {
                    NUM_DROPS.fetch_add(1, Relaxed);
                }
            }
        };
    }

    #[test]
    fn arc() {
        init_detect_drop!();

        // Create two Arcs sharing an object containing a string
        // and a DetectDrop to detect when it's dropped.
        let x = Arc::new(("hello", DetectDrop));
        let y = x.clone();

        // Send x to another thread, and use it there.
        let t = std::thread::spawn(move || {
            assert_eq!(x.0, "hello");
        });

        // In parallel, y should still be useable here.
        assert_eq!(y.0, "hello");

        // Wait for the thread to finish.
        t.join().unwrap();

        // x has been dropped, but y still exist,
        // so NUM_DROPS should be 0
        assert_eq!(NUM_DROPS.load(Relaxed), 0);

        // Drop y and Arc should be dropped now.
        drop(y);

        // Now NUM_DROPS increase
        assert_eq!(NUM_DROPS.load(Relaxed), 1);
    }

    #[test]
    fn arc_weak() {
        init_detect_drop!();

        // Create an Arc with two weak pointers.
        let x = Arc::new(("hello", DetectDrop));
        let y = Arc::downgrade(&x);
        let z = Arc::downgrade(&x);

        let t = std::thread::spawn(move || {
            // Weak pointer should be upgradable at this point.
            let y = y.upgrade().unwrap();
            assert_eq!(y.0, "hello");
        });
        assert_eq!(x.0, "hello");
        t.join().unwrap();

        // The data should not be dropped yet,
        // and the weak pointer should be upgradable.
        assert_eq!(NUM_DROPS.load(Relaxed), 0);
        assert!(z.upgrade().is_some());

        // drop all Arc
        drop(x);

        // Now, the data should be dropped, and the
        // weak pointer should no longer be upgradable.
        assert_eq!(NUM_DROPS.load(Relaxed), 1);
        assert!(z.upgrade().is_none());
    }
}
