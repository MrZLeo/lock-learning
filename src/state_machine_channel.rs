use std::{
    cell::UnsafeCell,
    mem::MaybeUninit,
    sync::atomic::{
        AtomicU8,
        Ordering::{Acquire, Relaxed, Release},
    },
};

enum State {
    Empty,
    Writing,
    Ready,
    Reading,
}

impl From<State> for u8 {
    fn from(value: State) -> Self {
        value as u8
    }
}

pub struct StateChannel<T> {
    message: UnsafeCell<MaybeUninit<T>>,
    state: AtomicU8,
}

impl<T> StateChannel<T> {
    pub const fn new() -> Self {
        Self {
            message: UnsafeCell::new(MaybeUninit::uninit()),
            state: AtomicU8::new(State::Empty as u8),
        }
    }

    pub fn send(&self, message: T) {
        if self
            .state
            .compare_exchange(State::Empty.into(), State::Writing.into(), Relaxed, Relaxed)
            .is_err()
        {
            panic!("can't send more than one message!");
        }

        unsafe {
            (*self.message.get()).write(message);
        }

        self.state.store(State::Ready.into(), Release);
    }

    pub fn is_ready(&self) -> bool {
        self.state.load(Relaxed) == State::Ready.into()
    }

    pub fn receive(&self) -> T {
        if self
            .state
            .compare_exchange(State::Ready.into(), State::Reading.into(), Acquire, Relaxed)
            .is_err()
        {
            panic!("no message available!");
        }
        unsafe { (*self.message.get()).assume_init_read() }
    }
}

impl<T> Drop for StateChannel<T> {
    fn drop(&mut self) {
        if *self.state.get_mut() == State::Ready.into() {
            unsafe { self.message.get_mut().assume_init_drop() }
        }
    }
}
