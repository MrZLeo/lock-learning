use std::{
    cell::UnsafeCell,
    mem::MaybeUninit,
    sync::{
        atomic::{
            AtomicBool,
            Ordering::{Acquire, Relaxed, Release},
        },
        Arc,
    },
};

/// Inner type, which is just the implementation detail inside the lib
struct TypeSafeChannel<T> {
    message: UnsafeCell<MaybeUninit<T>>,
    ready: AtomicBool,
}

impl<T> Drop for TypeSafeChannel<T> {
    fn drop(&mut self) {
        if *self.ready.get_mut() {
            unsafe {
                self.message.get_mut().assume_init_drop();
            }
        }
    }
}

unsafe impl<T> Sync for TypeSafeChannel<T> where T: Send {}

pub struct Sender<T> {
    channel: Arc<TypeSafeChannel<T>>,
}

pub struct Receiver<T> {
    channel: Arc<TypeSafeChannel<T>>,
}

impl<T> Sender<T> {
    pub fn send(self, message: T) {
        unsafe {
            (*self.channel.message.get()).write(message);
        }
        self.channel.ready.store(true, Release);
    }
}

impl<T> Receiver<T> {
    pub fn is_ready(&self) -> bool {
        self.channel.ready.load(Relaxed)
    }

    pub fn receive(self) -> T {
        if !self.channel.ready.swap(false, Acquire) {
            panic!("no message available!");
        }
        unsafe { (*self.channel.message.get()).assume_init_read() }
    }
}

pub fn channel<T>() -> (Sender<T>, Receiver<T>) {
    let channel = Arc::new(TypeSafeChannel {
        message: UnsafeCell::new(MaybeUninit::uninit()),
        ready: AtomicBool::new(false),
    });

    (
        Sender {
            channel: channel.clone(),
        },
        Receiver { channel },
    )
}

#[cfg(test)]
mod test {
    use std::thread;

    use super::channel;

    #[test]
    fn type_safe_channel() {
        thread::scope(|s| {
            let (sender, receiver) = channel();
            let t = thread::current();
            s.spawn(move || {
                sender.send("hello world!");
                t.unpark();
            });
            while !receiver.is_ready() {
                thread::park();
            }
            assert_eq!(receiver.receive(), "hello world!");
        });
    }
}
