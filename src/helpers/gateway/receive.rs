use crate::{
    helpers::{buffers::UnorderedReceiver, ChannelId, Error, Message, Transport},
    protocol::{step::Gate, RecordId},
};
use dashmap::DashMap;
use futures::Stream;
use std::marker::PhantomData;

/// Receiving end end of the gateway channel.
pub struct ReceivingEnd<T: Transport, M: Message> {
    unordered_rx: UR<T>,
    _phantom: PhantomData<M>,
}

/// Receiving channels, indexed by (role, step).
pub(super) struct GatewayReceivers<T: Transport, G: Gate> {
    inner: DashMap<ChannelId<G>, UR<T>>,
}

pub(super) type UR<T> = UnorderedReceiver<
    <T as Transport>::RecordsStream,
    <<T as Transport>::RecordsStream as Stream>::Item,
>;

impl<T: Transport, M: Message> ReceivingEnd<T, M> {
    pub(super) fn new(rx: UR<T>) -> Self {
        Self {
            unordered_rx: rx,
            _phantom: PhantomData,
        }
    }

    /// Receive message associated with the given record id. This method does not return until
    /// message is actually received and deserialized.
    ///
    /// ## Errors
    /// Returns an error if receiving fails
    ///
    /// ## Panics
    /// This will panic if message size does not fit into 8 bytes and it somehow got serialized
    /// and sent to this helper.
    pub async fn receive(&self, record_id: RecordId) -> Result<M, Error> {
        // TODO: proper error handling
        let v = self.unordered_rx.recv::<M, _>(record_id).await?;
        Ok(v)
    }
}

impl<T: Transport, G: Gate> Default for GatewayReceivers<T, G> {
    fn default() -> Self {
        Self {
            inner: DashMap::default(),
        }
    }
}

impl<T: Transport, G: Gate> GatewayReceivers<T, G> {
    pub fn get_or_create<F: FnOnce() -> UR<T>>(&self, channel_id: &ChannelId<G>, ctr: F) -> UR<T> {
        let receivers = &self.inner;
        if let Some(recv) = receivers.get(channel_id) {
            recv.clone()
        } else {
            let stream = ctr();
            receivers.insert(channel_id.clone(), stream.clone());
            stream
        }
    }
}
