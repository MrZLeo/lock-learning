use std::{
    cell::UnsafeCell,
    marker::PhantomData,
    mem::MaybeUninit,
    sync::atomic::{
        AtomicBool,
        Ordering::{Acquire, Relaxed, Release},
    },
    thread::{self, Thread},
};

pub struct TypeSafeChannel<T> {
    message: UnsafeCell<MaybeUninit<T>>,
    ready: AtomicBool,
}

impl<T> TypeSafeChannel<T> {
    pub const fn new() -> Self {
        Self {
            message: UnsafeCell::new(MaybeUninit::uninit()),
            ready: AtomicBool::new(false),
        }
    }

    pub fn split(&mut self) -> (Sender<T>, Receiver<T>) {
        *self = Self::new();
        (
            Sender {
                channel: self,
                receving_thread: thread::current(),
            },
            Receiver {
                channel: self,
                _no_send: PhantomData,
            },
        )
    }
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

pub struct Sender<'a, T> {
    channel: &'a TypeSafeChannel<T>,
    receving_thread: Thread,
}

pub struct Receiver<'a, T> {
    channel: &'a TypeSafeChannel<T>,
    /// Don't allow Receiver to be sent in order to no confused Sender
    _no_send: PhantomData<*const ()>,
}

impl<T> Sender<'_, T> {
    pub fn send(self, message: T) {
        unsafe {
            (*self.channel.message.get()).write(message);
        }
        self.channel.ready.store(true, Release);

        // raise the sleeping thread
        self.receving_thread.unpark();
    }
}

impl<T> Receiver<'_, T> {
    pub fn is_ready(&self) -> bool {
        self.channel.ready.load(Relaxed)
    }

    pub fn receive(self) -> T {
        while !self.channel.ready.swap(false, Acquire) {
            thread::park();
        }
        unsafe { (*self.channel.message.get()).assume_init_read() }
    }
}

#[cfg(test)]
mod test {
    #[deny(warnings)]
    use std::thread;

    use super::TypeSafeChannel;

    #[test]
    fn type_safe_channel() {
        let mut channel = TypeSafeChannel::new();
        thread::scope(|s| {
            let (sender, receiver) = channel.split();
            s.spawn(move || {
                sender.send("hello world!");
            });
            assert_eq!(receiver.receive(), "hello world!");
        });
    }
}
