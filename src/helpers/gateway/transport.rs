use std::marker::PhantomData;

use crate::{
    helpers::{
        buffers::UnorderedReceiver,
        gateway::{receive::UR, send::GatewaySendStream},
        ChannelId, GatewayConfig, Role, RoleAssignment, RouteId, Transport,
    },
    protocol::{step::Gate, QueryId},
};

/// Transport adapter that resolves [`Role`] -> [`HelperIdentity`] mapping. As gateways created
/// per query, it is not ambiguous.
///
/// [`HelperIdentity`]: crate::helpers::HelperIdentity
#[derive(Clone)]
pub(super) struct RoleResolvingTransport<T: Transport<G>, G: Gate> {
    pub query_id: QueryId,
    pub roles: RoleAssignment,
    pub config: GatewayConfig,
    pub inner: T,
    _marker: PhantomData<G>,
}

impl<T: Transport<G>, G: Gate> RoleResolvingTransport<T, G> {
    pub(crate) async fn send(
        &self,
        channel_id: &ChannelId<G>,
        data: GatewaySendStream<G>,
    ) -> Result<(), T::Error> {
        let dest_identity = self.roles.identity(channel_id.role);
        assert_ne!(
            dest_identity,
            self.inner.identity(),
            "can't send message to itself"
        );

        self.inner
            .send(
                dest_identity,
                (RouteId::Records, self.query_id, channel_id.step.clone()),
                data,
            )
            .await
    }

    pub(crate) fn receive(&self, channel_id: &ChannelId<G>) -> UR<T, G> {
        let peer = self.roles.identity(channel_id.role);
        assert_ne!(
            peer,
            self.inner.identity(),
            "can't receive message from itself"
        );

        UnorderedReceiver::new(
            Box::pin(
                self.inner
                    .receive(peer, (self.query_id, channel_id.step.clone())),
            ),
            self.config.active_work(),
        )
    }

    pub(crate) fn role(&self) -> Role {
        self.roles.role(self.inner.identity())
    }
}
