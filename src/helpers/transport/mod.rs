use crate::{
    helpers::HelperIdentity,
    protocol::{
        step::{self, Gate},
        QueryId,
    },
};
use async_trait::async_trait;
use futures::Stream;
use std::borrow::Borrow;

mod bytearrstream;
pub mod callbacks;
#[cfg(feature = "in-memory-infra")]
mod in_memory;
pub mod query;
mod receive;
mod stream;

pub use bytearrstream::{AlignedByteArrStream, ByteArrStream};
#[cfg(feature = "in-memory-infra")]
pub use in_memory::{InMemoryNetwork, InMemoryTransport};
pub use receive::{LogErrors, ReceiveRecords};
pub use stream::{StreamCollection, StreamKey};

pub trait ResourceIdentifier: Sized {}
pub trait QueryIdBinding: Sized
where
    Option<QueryId>: From<Self>,
{
}
pub trait StepBinding<G: Gate>: Sized
where
    Option<G>: From<Self>,
{
}

pub struct NoResourceIdentifier;
pub struct NoQueryId;
pub struct NoStep;

#[derive(Debug, Copy, Clone)]
pub enum RouteId {
    Records,
    ReceiveQuery,
    PrepareQuery,
}

impl ResourceIdentifier for NoResourceIdentifier {}
impl ResourceIdentifier for RouteId {}

impl From<NoQueryId> for Option<QueryId> {
    fn from(_: NoQueryId) -> Self {
        None
    }
}

impl QueryIdBinding for NoQueryId {}
impl QueryIdBinding for QueryId {}

impl<G: Gate> From<NoStep> for Option<G> {
    fn from(_: NoStep) -> Self {
        None
    }
}

impl<G: Gate> StepBinding<G> for NoStep {}
impl<G: Gate> StepBinding<G> for G where Option<G>: From<G> {}

pub trait RouteParams<R: ResourceIdentifier, Q: QueryIdBinding, S: StepBinding<G>, G: Gate>:
    Send
where
    Option<QueryId>: From<Q>,
    Option<G>: From<S>,
{
    type Params: Borrow<str>;

    fn resource_identifier(&self) -> R;
    fn query_id(&self) -> Q;
    fn step(&self) -> G;

    fn extra(&self) -> Self::Params;
}

impl<G: Gate, S: StepBinding<G>> RouteParams<NoResourceIdentifier, QueryId, S, G> for (QueryId, G)
where
    Option<G>: From<S>,
{
    type Params = &'static str;

    fn resource_identifier(&self) -> NoResourceIdentifier {
        NoResourceIdentifier
    }

    fn query_id(&self) -> QueryId {
        self.0
    }

    fn step(&self) -> G {
        self.1.clone()
    }

    fn extra(&self) -> Self::Params {
        ""
    }
}

impl<G: Gate, S: StepBinding<G>> RouteParams<RouteId, QueryId, S, G> for (RouteId, QueryId, G)
where
    Option<G>: From<S>,
{
    type Params = &'static str;

    fn resource_identifier(&self) -> RouteId {
        self.0
    }

    fn query_id(&self) -> QueryId {
        self.1
    }

    fn step(&self) -> G {
        self.2.clone()
    }

    fn extra(&self) -> Self::Params {
        ""
    }
}

// impl<G: Gate> RouteParams<NoResourceIdentifier, QueryId, step::Descriptive, G>
//     for (QueryId, step::Descriptive)
// where
//     step::Descriptive: StepBinding<G>,
//     Option<G>: From<step::Descriptive>,
// {
//     type Params = &'static str;

//     fn resource_identifier(&self) -> NoResourceIdentifier {
//         NoResourceIdentifier
//     }

//     fn query_id(&self) -> QueryId {
//         self.0
//     }

//     fn step(&self) -> step::Descriptive {
//         self.1.clone()
//     }

//     fn extra(&self) -> Self::Params {
//         ""
//     }
// }

// impl<G: Gate> RouteParams<RouteId, QueryId, step::Descriptive, G>
//     for (RouteId, QueryId, step::Descriptive)
// where
//     step::Descriptive: StepBinding<G>,
//     Option<G>: From<step::Descriptive>,
// {
//     type Params = &'static str;

//     fn resource_identifier(&self) -> RouteId {
//         self.0
//     }

//     fn query_id(&self) -> QueryId {
//         self.1
//     }

//     fn step(&self) -> step::Descriptive {
//         self.2.clone()
//     }

//     fn extra(&self) -> Self::Params {
//         ""
//     }
// }

/// Transport that supports per-query,per-step channels
#[async_trait]
pub trait Transport<G: Gate>: Clone + Send + Sync + 'static {
    type RecordsStream: Stream<Item = Vec<u8>> + Send + Unpin;
    type Error: std::fmt::Debug;

    fn identity(&self) -> HelperIdentity;

    /// Sends a new request to the given destination helper party.
    /// Depending on the specific request, it may or may not require acknowledgment by the remote
    /// party
    async fn send<D, Q, S, R>(
        &self,
        dest: HelperIdentity,
        route: R,
        data: D,
    ) -> Result<(), Self::Error>
    where
        Option<QueryId>: From<Q>,
        Option<G>: From<S>,
        Q: QueryIdBinding,
        S: StepBinding<G>,
        R: RouteParams<RouteId, Q, S, G>,
        D: Stream<Item = Vec<u8>> + Send + 'static;

    /// Return the stream of records to be received from another helper for the specific query
    /// and step
    fn receive<R, S>(&self, from: HelperIdentity, route: R) -> Self::RecordsStream
    where
        R: RouteParams<NoResourceIdentifier, QueryId, S, G>,
        S: StepBinding<G>,
        Option<G>: From<S>;

    /// Alias for `Clone::clone`.
    ///
    /// `Transport` is implemented for `Weak<InMemoryTranport>` and `Arc<HttpTransport<G>>`. Clippy won't
    /// let us write `transport.clone()` since these are ref-counted pointer types, and neither
    /// `Arc::clone` or `Weak::clone` is universally correct. Thus `Transport::clone_ref`. Calling
    /// it `Transport::clone` would result in clashes anywhere both `Transport` and `Arc` are in-scope.
    #[must_use]
    fn clone_ref(&self) -> Self {
        <Self as Clone>::clone(self)
    }
}
