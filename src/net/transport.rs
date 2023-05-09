use crate::{
    helpers::{
        query::{PrepareQuery, QueryConfig, QueryInput},
        CompleteQueryResult, HelperIdentity, LogErrors, NoResourceIdentifier, PrepareQueryResult,
        QueryIdBinding, QueryInputResult, ReceiveQueryResult, ReceiveRecords, RouteId, RouteParams,
        StepBinding, StreamCollection, Transport, TransportCallbacks,
    },
    net::{client::MpcHelperClient, error::Error, MpcHelperServer},
    protocol::{step::GateImpl, QueryId},
    sync::Arc,
};
use async_trait::async_trait;
use axum::{body::Bytes, extract::BodyStream};
use futures::{Stream, TryFutureExt};
use std::borrow::Borrow;

type LogHttpErrors = LogErrors<BodyStream, Bytes, axum::Error>;

/// HTTP transport for IPA helper service.
pub struct HttpTransport {
    identity: HelperIdentity,
    callbacks: TransportCallbacks<Arc<HttpTransport>>,
    clients: [MpcHelperClient; 3],
    record_streams: StreamCollection<LogHttpErrors>,
}

impl HttpTransport {
    #[must_use]
    pub fn new(
        identity: HelperIdentity,
        clients: [MpcHelperClient; 3],
        callbacks: TransportCallbacks<Arc<HttpTransport>>,
    ) -> (Arc<Self>, MpcHelperServer) {
        let transport = Self::new_internal(identity, clients, callbacks);
        let server = MpcHelperServer::new(Arc::clone(&transport));
        (transport, server)
    }

    fn new_internal(
        identity: HelperIdentity,
        clients: [MpcHelperClient; 3],
        callbacks: TransportCallbacks<Arc<HttpTransport>>,
    ) -> Arc<Self> {
        Arc::new(Self {
            identity,
            callbacks,
            clients,
            record_streams: StreamCollection::default(),
        })
    }

    pub fn receive_query(self: Arc<Self>, req: QueryConfig) -> ReceiveQueryResult {
        (Arc::clone(&self).callbacks.receive_query)(self, req)
    }

    pub fn prepare_query(self: Arc<Self>, req: PrepareQuery) -> PrepareQueryResult {
        (Arc::clone(&self).callbacks.prepare_query)(self, req)
    }

    pub fn query_input(self: Arc<Self>, req: QueryInput) -> QueryInputResult {
        (Arc::clone(&self).callbacks.query_input)(self, req)
    }

    pub fn complete_query(self: Arc<Self>, query_id: QueryId) -> CompleteQueryResult {
        (Arc::clone(&self).callbacks.complete_query)(self, query_id)
    }

    /// Connect an inbound stream of MPC record data.
    ///
    /// This is called by peer helpers via the HTTP server.
    pub fn receive_stream(
        self: Arc<Self>,
        query_id: QueryId,
        step: GateImpl,
        from: HelperIdentity,
        stream: BodyStream,
    ) {
        self.record_streams
            .add_stream((query_id, from, step), LogErrors::new(stream));
    }
}

#[async_trait]
impl Transport for Arc<HttpTransport> {
    type RecordsStream = ReceiveRecords<LogHttpErrors>;
    type Error = Error;

    fn identity(&self) -> HelperIdentity {
        self.identity
    }

    async fn send<
        D: Stream<Item = Vec<u8>> + Send + 'static,
        Q: QueryIdBinding,
        S: StepBinding,
        R: RouteParams<RouteId, Q, S>,
    >(
        &self,
        dest: HelperIdentity,
        route: R,
        data: D,
    ) -> Result<(), Error>
    where
        Option<QueryId>: From<Q>,
        Option<GateImpl>: From<S>,
    {
        let route_id = route.resource_identifier();
        match route_id {
            RouteId::Records => {
                // TODO(600): These fallible extractions aren't really necessary.
                let query_id = <Option<QueryId>>::from(route.query_id())
                    .expect("query_id required when sending records");
                let step = <Option<GateImpl>>::from(route.step())
                    .expect("step required when sending records");
                let resp_future = self.clients[dest].step(self.identity, query_id, &step, data)?;
                tokio::spawn(async move {
                    resp_future
                        .map_err(Into::into)
                        .and_then(MpcHelperClient::resp_ok)
                        .await
                        .expect("failed to stream records");
                });
                // TODO(600): We need to do something better than panic if there is an error sending the
                // data. Note, also, that the caller of this function (`GatewayBase::get_sender`)
                // currently panics on errors.
                Ok(())
            }
            RouteId::PrepareQuery => {
                let req = serde_json::from_str(route.extra().borrow()).unwrap();
                self.clients[dest].prepare_query(self.identity, req).await
            }
            RouteId::ReceiveQuery => {
                unimplemented!("attempting to send ReceiveQuery to another helper")
            }
        }
    }

    fn receive<R: RouteParams<NoResourceIdentifier, QueryId, GateImpl>>(
        &self,
        from: HelperIdentity,
        route: R,
    ) -> Self::RecordsStream {
        ReceiveRecords::new(
            (route.query_id(), from, route.step()),
            self.record_streams.clone(),
        )
    }
}

#[cfg(all(test, not(feature = "shuttle"), feature = "real-world-infra"))]
mod e2e_tests {
    use super::*;
    use crate::{
        config::{NetworkConfig, PeerConfig, ServerConfig},
        ff::{FieldType, Fp31, Serializable},
        helpers::{query::QueryType, ByteArrStream},
        net::test::{body_stream, TestClients, TestServer},
        protocol::step,
        secret_sharing::{replicated::semi_honest::AdditiveShare, IntoShares},
        test_fixture::{config::TestConfigBuilder, Reconstruct},
        AppSetup, HelperApp,
    };
    use futures::stream::{poll_immediate, StreamExt};
    use futures_util::future::{join_all, try_join_all};
    use generic_array::GenericArray;
    use once_cell::sync::Lazy;
    use std::{iter::zip, net::TcpListener, task::Poll};
    use tokio::sync::mpsc::channel;
    use tokio_stream::wrappers::ReceiverStream;
    use typenum::Unsigned;

    static STEP: Lazy<GateImpl> = Lazy::new(|| GateImpl::from("http-transport"));

    #[tokio::test]
    async fn receive_stream() {
        let (tx, rx) = channel::<Result<Bytes, Box<dyn std::error::Error + Send + Sync>>>(1);
        let expected_chunk1 = vec![0u8, 1, 2, 3];
        let expected_chunk2 = vec![255u8, 254, 253, 252];

        let TestServer { transport, .. } = TestServer::default().await;

        let body = body_stream(Box::new(ReceiverStream::new(rx))).await;

        // Register the stream with the transport (normally called by step data HTTP API handler)
        Arc::clone(&transport).receive_stream(QueryId, STEP.clone(), HelperIdentity::TWO, body);

        // Request step data reception (normally called by protocol)
        let mut stream =
            Arc::clone(&transport).receive(HelperIdentity::TWO, (QueryId, STEP.clone()));

        // make sure it is not ready as it hasn't received any data yet.
        assert!(matches!(
            poll_immediate(&mut stream).next().await,
            Some(Poll::Pending)
        ));

        // send and verify first chunk
        tx.send(Ok(expected_chunk1.clone().into())).await.unwrap();

        assert_eq!(
            poll_immediate(&mut stream).next().await,
            Some(Poll::Ready(expected_chunk1))
        );

        // send and verify second chunk
        tx.send(Ok(expected_chunk2.clone().into())).await.unwrap();

        assert_eq!(
            poll_immediate(&mut stream).next().await,
            Some(Poll::Ready(expected_chunk2))
        );
    }

    // TODO: write a test for an error while reading the body (after error handling is finalized)

    async fn make_helpers(
        ids: [HelperIdentity; 3],
        sockets: [TcpListener; 3],
        server_config: [ServerConfig; 3],
        network_config: &NetworkConfig,
    ) -> [HelperApp; 3] {
        use crate::net::BindTarget;

        join_all(zip(ids, zip(sockets, server_config)).map(
            |(id, (socket, _server_conf))| async move {
                let (setup, callbacks) = AppSetup::new();
                let client_config = network_config.clone();
                let clients = TestClients::builder()
                    .with_network_config(client_config)
                    .build();
                let (transport, server) = HttpTransport::new(id, clients.0, callbacks);
                server.bind(BindTarget::HttpListener(socket), ()).await;
                let app = setup.connect(transport);
                app
            },
        ))
        .await
        .try_into()
        .ok()
        .unwrap()
    }

    fn make_clients(confs: &[PeerConfig; 3]) -> [MpcHelperClient; 3] {
        confs
            .iter()
            .map(|conf| MpcHelperClient::new(conf.origin.clone()))
            .collect::<Vec<_>>()
            .try_into()
            .unwrap()
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn happy_case() {
        const SZ: usize = <AdditiveShare<Fp31> as Serializable>::Size::USIZE;
        let mut conf = TestConfigBuilder::with_open_ports().build();
        let ids = HelperIdentity::make_three();
        let clients = make_clients(conf.network.peers());
        let _helpers = make_helpers(
            ids,
            conf.sockets.take().unwrap(),
            conf.servers,
            &conf.network,
        )
        .await;

        // send a create query command
        let leader_client = &clients[0];
        let create_data = QueryConfig {
            field_type: FieldType::Fp31,
            query_type: QueryType::TestMultiply,
        };

        // create query
        let query_id = leader_client.create_query(create_data).await.unwrap();

        // send input
        let a = Fp31::try_from(4u128).unwrap();
        let b = Fp31::try_from(5u128).unwrap();

        let helper_shares = (a, b).share().map(|(a, b)| {
            let mut vec = vec![0u8; 2 * SZ];
            a.serialize(GenericArray::from_mut_slice(&mut vec[..SZ]));
            b.serialize(GenericArray::from_mut_slice(&mut vec[SZ..]));
            ByteArrStream::from(vec)
        });

        let mut handle_resps = Vec::with_capacity(helper_shares.len());
        for (i, input_stream) in helper_shares.into_iter().enumerate() {
            let data = QueryInput {
                query_id,
                input_stream,
            };
            handle_resps.push(clients[i].query_input(data));
        }
        try_join_all(handle_resps).await.unwrap();

        let result: [_; 3] = join_all(clients.map(|client| async move {
            let r = client.query_results(query_id).await.unwrap();
            AdditiveShare::<Fp31>::from_byte_slice(&r).collect::<Vec<_>>()
        }))
        .await
        .try_into()
        .unwrap();
        let res = result.reconstruct();
        assert_eq!(Fp31::try_from(20u128).unwrap(), res[0]);
    }
}
