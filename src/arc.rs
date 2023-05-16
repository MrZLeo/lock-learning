use std::{
    cell::UnsafeCell,
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
    /// Number of `Arc`s + `Weak`s
    alloc_ref_count: AtomicUsize,
    /// The data. `None` if only `Weak` remain.
    data: UnsafeCell<Option<T>>,
}

pub struct Arc<T> {
    weak: Weak<T>,
}

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
            return Some(Arc { weak: self.clone() });
        }
    }
}

impl<T> Arc<T> {
    pub fn new(data: T) -> Self {
        Self {
            weak: Weak {
                ptr: NonNull::from(Box::leak(Box::new(ArcData {
                    data_ref_count: AtomicUsize::new(1),
                    alloc_ref_count: AtomicUsize::new(1),
                    data: UnsafeCell::new(Some(data)),
                }))),
            },
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
        // Make sure that every operations before the Drop's `Release`
        // store **happens before** this load.
        //
        // We just need to make sure that Ordering is `Acquire`
        // when `ref_count` == 1, so `fence()` is better option.
        if arc.weak.data().alloc_ref_count.load(Relaxed) == 1 {
            fence(Acquire);
            // Safety: Nothing else can access the data, since
            // there's only one Arc, to which we have exclusive access,
            // and no Weak pointers.
            let arcdata = unsafe { arc.weak.ptr.as_mut() };
            let option = arcdata.data.get_mut();
            // We know that data is still available since we
            // have an Arc to it, so this won't panic.
            let data = option.as_mut().unwrap();
            Some(data)
        } else {
            None
        }
    }

    pub fn downgrade(arc: &Self) -> Weak<T> {
        arc.weak.clone()
    }
}

impl<T> Deref for Arc<T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        let ptr = self.weak.data().data.get();

        // Safety: Since there's an Arc to the data,
        // the data exists and may be shared.
        unsafe { (*ptr).as_ref().unwrap() }
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
        let weak = self.weak.clone();
        if weak.data().data_ref_count.fetch_add(1, Relaxed) > usize::MAX / 2 {
            std::process::abort();
        }
        Arc { weak }
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
        if self.weak.data().data_ref_count.fetch_sub(1, Relaxed) == 1 {
            fence(Acquire);

            // If it's the last Arc, release the data itself
            let ptr = self.weak.data().data.get();

            // Safety: The data reference counter is zero,
            // so nothing will access it.
            unsafe {
                // Take the ownership of T inside Option,
                // and it's lifetime ended here,
                // which trigger the `Drop::drop()` of T
                let _ = (*ptr).take();
            }
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
