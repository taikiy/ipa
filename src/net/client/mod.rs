use crate::{
    config::NetworkConfig,
    helpers::{
        query::{PrepareQuery, QueryConfig, QueryInput},
        HelperIdentity,
    },
    net::{http_serde, Error},
    protocol::{step::GateImpl, QueryId},
};
use axum::http::uri;
use futures::{Stream, StreamExt};
use hyper::{
    body,
    client::{HttpConnector, ResponseFuture},
    Body, Client, Response, StatusCode, Uri,
};
use hyper_tls::HttpsConnector;
use std::collections::HashMap;

/// TODO: we need a client that can be used by any system that is not aware of the internals
///       of the helper network. That means that create query and send inputs API need to be
///       separated from prepare/step data etc.
/// TODO: It probably isn't necessary to always use `[MpcHelperClient; 3]`. Instead, a single
///       client can be configured to talk to all three helpers.
#[derive(Debug, Clone)]
pub struct MpcHelperClient {
    client: Client<HttpsConnector<HttpConnector>, Body>,
    scheme: uri::Scheme,
    authority: uri::Authority,
}

impl MpcHelperClient {
    #[must_use]
    #[allow(clippy::missing_panics_doc)]
    pub fn from_conf(conf: &NetworkConfig) -> [MpcHelperClient; 3] {
        conf.peers()
            .iter()
            .map(|conf| Self::new(conf.origin.clone()))
            .collect::<Vec<_>>()
            .try_into()
            .unwrap()
    }

    /// addr must have a valid scheme and authority
    /// # Panics
    /// if addr does not have scheme and authority
    #[must_use]
    pub fn new(addr: Uri) -> Self {
        // HttpsConnector works for both http and https
        Self::new_with_connector(addr, HttpsConnector::new())
    }

    /// addr must have a valid scheme and authority
    /// # Panics
    /// if addr does not have scheme and authority
    #[must_use]
    pub fn new_with_connector(addr: Uri, connector: HttpsConnector<HttpConnector>) -> Self {
        let client = Client::builder().build(connector);
        let parts = addr.into_parts();
        Self {
            client,
            scheme: parts.scheme.unwrap(),
            authority: parts.authority.unwrap(),
        }
    }

    /// same as new, but first parses the addr from a [&str]
    /// # Errors
    /// if addr is an invalid [Uri], this will fail
    pub fn with_str_addr(addr: &str) -> Result<Self, Error> {
        Ok(Self::new(addr.parse()?))
    }

    /// Responds with whatever input is passed to it
    /// # Errors
    /// If the request has illegal arguments, or fails to deliver to helper
    pub async fn echo(&self, s: &str) -> Result<String, Error> {
        const FOO: &str = "foo";

        let req =
            http_serde::echo::Request::new(HashMap::from([(FOO.into(), s.into())]), HashMap::new());
        let req = req.try_into_http_request(self.scheme.clone(), self.authority.clone())?;
        let resp = self.client.request(req).await?;
        let status = resp.status();
        if status.is_success() {
            let result = hyper::body::to_bytes(resp.into_body()).await?;
            let http_serde::echo::Request {
                mut query_params, ..
            } = serde_json::from_slice(&result)?;
            // It is potentially confusing to synthesize a 500 error here, but
            // it doesn't seem worth creating an error variant just for this.
            query_params.remove(FOO).ok_or(Error::FailedHttpRequest {
                status: StatusCode::INTERNAL_SERVER_ERROR,
                reason: "did not receive mirrored echo response".into(),
            })
        } else {
            Err(Error::from_failed_resp(resp).await)
        }
    }

    /// Helper to read a possible error response to a request that returns nothing on success
    ///
    /// # Errors
    /// If there was an error reading the response body or if the request itself failed.
    pub async fn resp_ok(resp: Response<Body>) -> Result<(), Error> {
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(Error::from_failed_resp(resp).await)
        }
    }

    /// Intended to be called externally, by the report collector. Informs the MPC ring that
    /// the external party wants to start a new query.
    /// # Errors
    /// If the request has illegal arguments, or fails to deliver to helper
    pub async fn create_query(&self, data: QueryConfig) -> Result<QueryId, Error> {
        let req = http_serde::query::create::Request::new(data);
        let req = req.try_into_http_request(self.scheme.clone(), self.authority.clone())?;
        let resp = self.client.request(req).await?;
        if resp.status().is_success() {
            let body_bytes = body::to_bytes(resp.into_body()).await?;
            let http_serde::query::create::ResponseBody { query_id } =
                serde_json::from_slice(&body_bytes)?;
            Ok(query_id)
        } else {
            Err(Error::from_failed_resp(resp).await)
        }
    }

    /// Used to communicate from one helper to another. Specifically, the helper that receives a
    /// "create query" from an external party must communicate the intent to start a query to the
    /// other helpers, which this prepare query does.
    /// # Errors
    /// If the request has illegal arguments, or fails to deliver to helper
    pub async fn prepare_query(
        &self,
        origin: HelperIdentity,
        data: PrepareQuery,
    ) -> Result<(), Error> {
        let req = http_serde::query::prepare::Request::new(origin, data);
        let req = req.try_into_http_request(self.scheme.clone(), self.authority.clone())?;
        let resp = self.client.request(req).await?;
        Self::resp_ok(resp).await
    }

    /// Intended to be called externally, e.g. by the report collector. After the report collector
    /// calls "create query", it must then send the data for the query to each of the clients. This
    /// query input contains the data intended for a helper.
    /// # Errors
    /// If the request has illegal arguments, or fails to deliver to helper
    pub async fn query_input(&self, data: QueryInput) -> Result<(), Error> {
        let req = http_serde::query::input::Request::new(data);
        let req = req.try_into_http_request(self.scheme.clone(), self.authority.clone())?;
        let resp = self.client.request(req).await?;
        Self::resp_ok(resp).await
    }

    /// Sends a batch of messages associated with a query's step to another helper. Messages are a
    /// contiguous block of records. Also includes [`crate::protocol::RecordId`] information and
    /// [`crate::helpers::network::ChannelId`].
    /// # Errors
    /// If the request has illegal arguments, or fails to deliver to helper
    /// # Panics
    /// If messages size > max u32 (unlikely)
    pub fn step<S: Stream<Item = Vec<u8>> + Send + 'static>(
        &self,
        origin: HelperIdentity,
        query_id: QueryId,
        step: &GateImpl,
        data: S,
    ) -> Result<ResponseFuture, Error> {
        let body = hyper::Body::wrap_stream::<_, _, Error>(data.map(Ok));
        let req = http_serde::query::step::Request::new(origin, query_id, step.clone(), body);
        let req = req.try_into_http_request(self.scheme.clone(), self.authority.clone())?;
        Ok(self.client.request(req))
    }

    /// Wait for completion of the query and pull the results of this query. This is a blocking
    /// API so it is not supposed to be used outside of CLI context.
    ///
    /// ## Errors
    /// If the request has illegal arguments, or fails to deliver to helper
    /// # Panics
    /// if there is a problem reading the response body
    #[cfg(any(all(test, not(feature = "shuttle")), feature = "cli"))]
    pub async fn query_results(&self, query_id: QueryId) -> Result<body::Bytes, Error> {
        let req = http_serde::query::results::Request::new(query_id);
        let req = req.try_into_http_request(self.scheme.clone(), self.authority.clone())?;

        let resp = self.client.request(req).await?;
        if resp.status().is_success() {
            Ok(body::to_bytes(resp.into_body()).await.unwrap())
        } else {
            Err(Error::from_failed_resp(resp).await)
        }
    }
}

#[cfg(all(test, not(feature = "shuttle"), feature = "real-world-infra"))]
pub(crate) mod tests {
    use super::*;
    use crate::{
        ff::{FieldType, Fp31},
        helpers::{
            query::QueryType, RoleAssignment, Transport, TransportCallbacks,
            MESSAGE_PAYLOAD_SIZE_BYTES,
        },
        net::{test::TestServer, HttpTransport},
        protocol::step::{GateImpl, StepNarrow},
        query::ProtocolResult,
        secret_sharing::replicated::semi_honest::AdditiveShare as Replicated,
        sync::Arc,
    };
    use futures::stream::{once, poll_immediate};
    use std::{
        fmt::Debug,
        future::{ready, Future},
        task::Poll,
    };

    // This is a kludgy way of working around `TransportCallbacks` not being `Clone`, so
    // that tests can run against both HTTP and HTTPS servers with one set.
    //
    // If the use grows beyond that, it's probably worth doing something more elegant, on the
    // TransportCallbacks type itself (references and lifetime parameters, dyn_clone, or make it a
    // trait and implement it on an `Arc` type).
    fn clone_callbacks<T: 'static>(
        cb: TransportCallbacks<T>,
    ) -> (TransportCallbacks<T>, TransportCallbacks<T>) {
        fn wrap<T: 'static>(inner: &Arc<TransportCallbacks<T>>) -> TransportCallbacks<T> {
            let ri = Arc::clone(inner);
            let pi = Arc::clone(inner);
            let qi = Arc::clone(inner);
            let ci = Arc::clone(inner);
            TransportCallbacks {
                receive_query: Box::new(move |t, req| (ri.receive_query)(t, req)),
                prepare_query: Box::new(move |t, req| (pi.prepare_query)(t, req)),
                query_input: Box::new(move |t, req| (qi.query_input)(t, req)),
                complete_query: Box::new(move |t, req| (ci.complete_query)(t, req)),
            }
        }

        let arc_cb = Arc::new(cb);
        (wrap(&arc_cb), wrap(&arc_cb))
    }

    /// tests that a query command runs as expected. Since query commands require the server to
    /// actively respond to a client request, the test must handle both ends of the request
    /// simultaneously. That means taking the client behavior (`clientf`) and the server behavior
    /// (`serverf`), and executing them simultaneously (via a `join!`). Finally, return the results
    /// of `clientf` for final checks.
    ///
    /// Also tests that the same functionality works for both `http` and `https`. In order to ensure
    /// this, the return type of `clientf` must be `Eq + Debug` so that the results of `http` and
    /// `https` can be compared.
    async fn test_query_command<ClientOut, ClientFut, ClientF>(
        clientf: ClientF,
        server_cb: TransportCallbacks<Arc<HttpTransport>>,
    ) -> ClientOut
    where
        ClientOut: Eq + Debug,
        ClientFut: Future<Output = ClientOut>,
        ClientF: Fn(MpcHelperClient) -> ClientFut,
    {
        let (http_cb, https_cb) = clone_callbacks(server_cb);

        let TestServer {
            client: http_client,
            ..
        } = TestServer::builder().with_callbacks(http_cb).build().await;

        let clientf_res_http = clientf(http_client).await;

        let TestServer {
            client: https_client,
            ..
        } = TestServer::builder()
            .https()
            .with_callbacks(https_cb)
            .build()
            .await;

        let clientf_res_https = clientf(https_client).await;

        assert_eq!(clientf_res_http, clientf_res_https);
        clientf_res_http
    }

    #[tokio::test]
    async fn echo() {
        let expected_output = "asdf";

        let output = test_query_command(
            |client| async move { client.echo(expected_output).await.unwrap() },
            TransportCallbacks::default(),
        )
        .await;
        assert_eq!(expected_output, &output);
    }

    #[tokio::test]
    async fn create() {
        let expected_query_id = QueryId;
        let expected_query_config = QueryConfig {
            field_type: FieldType::Fp31,
            query_type: QueryType::TestMultiply,
        };
        let cb = TransportCallbacks {
            receive_query: Box::new(move |_transport, query_config| {
                assert_eq!(query_config, expected_query_config);
                Box::pin(ready(Ok(expected_query_id)))
            }),
            ..Default::default()
        };
        let query_id = test_query_command(
            |client| async move { client.create_query(expected_query_config).await.unwrap() },
            cb,
        )
        .await;
        assert_eq!(query_id, expected_query_id);
    }

    #[tokio::test]
    async fn prepare() {
        let input = PrepareQuery {
            query_id: QueryId,
            config: QueryConfig {
                field_type: FieldType::Fp31,
                query_type: QueryType::TestMultiply,
            },
            roles: RoleAssignment::new(HelperIdentity::make_three()),
        };
        let expected_data = input.clone();
        let origin = HelperIdentity::ONE;
        let cb = TransportCallbacks {
            prepare_query: Box::new(move |_transport, prepare_query| {
                assert_eq!(prepare_query, expected_data);
                Box::pin(ready(Ok(())))
            }),
            ..Default::default()
        };
        test_query_command(
            |client| {
                let req = input.clone();
                async move { client.prepare_query(origin, req).await.unwrap() }
            },
            cb,
        )
        .await;
    }

    #[tokio::test]
    async fn input() {
        let expected_query_id = QueryId;
        let expected_input = &[8u8; 25];
        let cb = TransportCallbacks {
            query_input: Box::new(move |_transport, query_input| {
                Box::pin(async move {
                    assert_eq!(query_input.query_id, expected_query_id);
                    assert_eq!(&query_input.input_stream.to_vec().await, expected_input);
                    Ok(())
                })
            }),
            ..Default::default()
        };
        test_query_command(
            |client| {
                let data = QueryInput {
                    query_id: expected_query_id,
                    input_stream: expected_input.to_vec().into(),
                };
                async move { client.query_input(data).await.unwrap() }
            },
            cb,
        )
        .await;
    }

    #[tokio::test]
    async fn step() {
        let TestServer {
            client, transport, ..
        } = TestServer::builder().build().await;
        let origin = HelperIdentity::ONE;
        let expected_query_id = QueryId;
        let expected_step = GateImpl::default().narrow("test-step");
        let expected_payload = vec![7u8; MESSAGE_PAYLOAD_SIZE_BYTES];

        let resp = client
            .step(
                origin,
                expected_query_id,
                &expected_step,
                once(ready(expected_payload.clone())),
            )
            .unwrap()
            .await
            .unwrap();

        MpcHelperClient::resp_ok(resp).await.unwrap();

        let mut stream =
            Arc::clone(&transport).receive(HelperIdentity::ONE, (QueryId, expected_step.clone()));

        assert_eq!(
            poll_immediate(&mut stream).next().await,
            Some(Poll::Ready(expected_payload))
        );
    }

    #[tokio::test]
    async fn results() {
        let expected_results = Box::new(vec![Replicated::from((
            Fp31::try_from(1u128).unwrap(),
            Fp31::try_from(2u128).unwrap(),
        ))]);
        let expected_query_id = QueryId;
        let raw_results = expected_results.to_vec();
        let cb = TransportCallbacks {
            complete_query: Box::new(move |_transport, query_id| {
                let results: Box<dyn ProtocolResult> = Box::new(raw_results.clone());
                assert_eq!(query_id, expected_query_id);
                Box::pin(ready(Ok(results)))
            }),
            ..Default::default()
        };
        let results = test_query_command(
            |client| async move { client.query_results(expected_query_id).await.unwrap() },
            cb,
        )
        .await;
        assert_eq!(results.to_vec(), expected_results.into_bytes());
    }
}
