use crate::{
    helpers::transport::stream::{StreamCollection, StreamKey},
    protocol::step::Gate,
};
use futures::Stream;
use futures_util::StreamExt;
use std::{
    error::Error as StdError,
    pin::Pin,
    task::{Context, Poll},
};
use tracing::error;

/// Adapt a stream of `Result<T: Into<Vec<u8>>, Error>` to a stream of `Vec<u8>`.
///
/// If an error is encountered, the error is logged, and the stream is terminated.
pub struct LogErrors<S, T, E>
where
    S: Stream<Item = Result<T, E>> + Unpin,
    T: Into<Vec<u8>>,
    E: StdError,
{
    inner: S,
}

impl<S, T, E> LogErrors<S, T, E>
where
    S: Stream<Item = Result<T, E>> + Unpin,
    T: Into<Vec<u8>>,
    E: StdError,
{
    pub fn new(inner: S) -> Self {
        Self { inner }
    }
}

impl<S, T, E> Stream for LogErrors<S, T, E>
where
    S: Stream<Item = Result<T, E>> + Unpin,
    T: Into<Vec<u8>>,
    E: StdError,
{
    type Item = Vec<u8>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match Pin::get_mut(self).inner.poll_next_unpin(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Some(Ok(chunk))) => Poll::Ready(Some(chunk.into())),
            Poll::Ready(Some(Err(err))) => {
                // Report this error in the server log since it may require investigation
                // by the helper party operators. It will not be informative for a report
                // collector.
                //
                // Note that returning `Poll::Ready(None)` here will be turned back into
                // an `EndOfStream` error by `UnorderedReceiver`.
                error!("error reading records: {err}");
                Poll::Ready(None)
            }
            Poll::Ready(None) => Poll::Ready(None),
        }
    }
}

/// Represents a stream of records.
/// If stream is not received yet, each poll generates a waker that is used internally to wake up
/// the task when stream is received.
/// Once stream is received, it is moved to this struct and it acts as a proxy to it.
pub struct ReceiveRecords<S, G> {
    inner: ReceiveRecordsInner<S, G>,
}

impl<S, G> ReceiveRecords<S, G> {
    pub(crate) fn new(key: StreamKey<G>, coll: StreamCollection<S, G>) -> Self {
        Self {
            inner: ReceiveRecordsInner::Pending(key, coll),
        }
    }
}

impl<S: Stream + Unpin, G: Gate> Stream for ReceiveRecords<S, G> {
    type Item = S::Item;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::get_mut(self).inner.poll_next_unpin(cx)
    }
}

/// Inner state for [`ReceiveRecords`] struct
enum ReceiveRecordsInner<S, G> {
    Pending(StreamKey<G>, StreamCollection<S, G>),
    Ready(S),
}

impl<S: Stream + Unpin, G: Gate> Stream for ReceiveRecordsInner<S, G> {
    type Item = S::Item;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = Pin::get_mut(self);
        loop {
            match this {
                Self::Pending(key, streams) => {
                    if let Some(stream) = streams.add_waker(key, cx.waker()) {
                        *this = Self::Ready(stream);
                    } else {
                        return Poll::Pending;
                    }
                }
                Self::Ready(stream) => return stream.poll_next_unpin(cx),
            }
        }
    }
}
