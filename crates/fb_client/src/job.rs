//! Blocking work (HTTP requests, a probe connection) on a thread of its own, polled from systems.
use std::thread::{self, JoinHandle};

/// Named `fb-…`: a panic in it is not a crash of the game (`crash.rs`).
pub struct Job<T>(JoinHandle<T>);

impl<T: Send + 'static> Job<T> {
    pub fn spawn(name: &str, f: impl FnOnce() -> T + Send + 'static) -> std::io::Result<Self> {
        thread::Builder::new().name(format!("fb-{name}")).spawn(f).map(Self)
    }
}

impl<T> Job<T> {
    pub fn ready(&self) -> bool {
        self.0.is_finished()
    }

    /// Its result (None: it panicked); blocks until it is done.
    pub fn join(self) -> Option<T> {
        self.0.join().ok()
    }

    /// The result of the job in `slot` once it is done, taking it out (Some(None): it panicked).
    pub fn take(slot: &mut Option<Self>) -> Option<Option<T>> {
        if !slot.as_ref()?.ready() {
            return None;
        }
        slot.take().map(Self::join)
    }
}
