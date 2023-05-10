use crate::{
    helpers::HelperIdentity,
    protocol::{step::Gate, QueryId},
    sync::{Arc, Mutex},
};
use futures::Stream;
use std::{
    collections::{hash_map::Entry, HashMap},
    fmt::{Debug, Formatter},
    task::Waker,
};

/// Each stream is indexed by query id, the identity of helper where stream is originated from
/// and step.
pub type StreamKey<G: Gate> = (QueryId, HelperIdentity, G);

/// Thread-safe append-only collection of homogeneous record streams.
/// Streams are indexed by [`StreamKey`] and the lifecycle of each stream is described by the
/// [`RecordsStream`] struct.
///
/// Each stream can be inserted and taken away exactly once, any deviation from this behaviour will
/// result in panic.
pub struct StreamCollection<S, G> {
    inner: Arc<Mutex<HashMap<StreamKey<G>, RecordsStream<S>>>>,
}

impl<S, G> Default for StreamCollection<S, G> {
    fn default() -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::default())),
        }
    }
}

impl<S, G> Clone for StreamCollection<S, G> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<S: Stream, G: Gate> StreamCollection<S, G> {
    /// Adds a new stream associated with the given key.
    ///
    /// ## Panics
    /// If there was another stream associated with the same key some time in the past.
    pub fn add_stream(&self, key: StreamKey<G>, stream: S) {
        let mut streams = self.inner.lock().unwrap();
        match streams.entry(key) {
            Entry::Occupied(mut entry) => match entry.get_mut() {
                rs @ RecordsStream::Waiting(_) => {
                    let RecordsStream::Waiting(waker) = std::mem::replace(rs, RecordsStream::Ready(stream)) else {
                        unreachable!()
                    };
                    waker.wake();
                }
                rs @ (RecordsStream::Ready(_) | RecordsStream::Completed) => {
                    let state = format!("{rs:?}");
                    let key = entry.key().clone();
                    drop(streams);
                    panic!("{key:?} entry state expected to be waiting, got {state:?}");
                }
            },
            Entry::Vacant(entry) => {
                entry.insert(RecordsStream::Ready(stream));
            }
        }
    }

    /// Adds a new waker to notify when the stream is ready. If stream is ready, this method takes
    /// it out, leaving a tombstone in its place, and returns it.
    ///
    /// ## Panics
    /// If [`Waker`] that exists already inside this collection will not wake the given one.
    pub fn add_waker(&self, key: &StreamKey<G>, waker: &Waker) -> Option<S> {
        let mut streams = self.inner.lock().unwrap();

        match streams.entry(key.clone()) {
            Entry::Occupied(mut entry) => {
                match entry.get_mut() {
                    RecordsStream::Waiting(old_waker) => {
                        let will_wake = old_waker.will_wake(waker);
                        drop(streams); // avoid mutex poisoning
                        assert!(will_wake);
                        None
                    }
                    rs @ RecordsStream::Ready(_) => {
                        let RecordsStream::Ready(stream) = std::mem::replace(rs, RecordsStream::Completed) else {
                            unreachable!();
                        };

                        Some(stream)
                    }
                    RecordsStream::Completed => {
                        drop(streams);
                        panic!("{key:?} stream has been consumed already")
                    }
                }
            }
            Entry::Vacant(entry) => {
                entry.insert(RecordsStream::Waiting(waker.clone()));
                None
            }
        }
    }
}

/// Describes the lifecycle of records stream inside [`StreamCollection`]
enum RecordsStream<S> {
    /// There was a request to receive this stream, but it hasn't arrived yet
    Waiting(Waker),
    /// Stream is ready to be consumed
    Ready(S),
    /// Stream was successfully received and taken away from [`StreamCollection`].
    /// It may not be requested or received again.
    Completed,
}

impl<S> Debug for RecordsStream<S> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            RecordsStream::Waiting(_) => {
                write!(f, "Waiting")
            }
            RecordsStream::Ready(_) => {
                write!(f, "Ready")
            }
            RecordsStream::Completed => {
                write!(f, "Completed")
            }
        }
    }
}
