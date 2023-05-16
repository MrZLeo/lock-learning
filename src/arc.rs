use std::{
    ops::Deref,
    ptr::NonNull,
    sync::atomic::{
        fence, AtomicUsize,
        Ordering::{Acquire, Relaxed, Release},
    },
};

struct ArcData<T> {
    ref_count: AtomicUsize,
    data: T,
}

pub struct Arc<T> {
    ptr: NonNull<ArcData<T>>,
}

unsafe impl<T: Send + Sync> Send for Arc<T> {}
unsafe impl<T: Send + Sync> Sync for Arc<T> {}

impl<T> Arc<T> {
    pub fn new(data: T) -> Self {
        Self {
            ptr: NonNull::from(Box::leak(Box::new(ArcData {
                ref_count: AtomicUsize::new(1),
                data,
            }))),
        }
    }

    fn data(&self) -> &ArcData<T> {
        unsafe { self.ptr.as_ref() }
    }
}

impl<T> Deref for Arc<T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        &self.data().data
    }
}

impl<T> Clone for Arc<T> {
    fn clone(&self) -> Self {
        // Simple way to handle overflows
        if self.data().ref_count.fetch_add(1, Relaxed) > usize::MAX / 2 {
            std::process::abort();
        }

        // Just move `self.ptr` into new Arc because NonNull is `Copy`
        Arc { ptr: self.ptr }
    }
}

impl<T> Drop for Arc<T> {
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
        if self.data().ref_count.fetch_sub(1, Release) == 1 {
            fence(Acquire);
            unsafe {
                drop(Box::from_raw(self.ptr.as_ptr()));
            }
        }
    }
}

#[cfg(test)]
mod test {
    use std::sync::atomic::AtomicUsize;
    use std::sync::atomic::Ordering::Relaxed;

    use crate::arc::Arc;

    #[test]
    fn arc() {
        static NUM_DROPS: AtomicUsize = AtomicUsize::new(0);

        struct DetectDrop;

        impl Drop for DetectDrop {
            fn drop(&mut self) {
                NUM_DROPS.fetch_add(1, Relaxed);
            }
        }

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
}
