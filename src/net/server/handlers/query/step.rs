use crate::{
    helpers::Transport,
    net::{http_serde, server::Error, HttpTransport},
    protocol::step::Gate,
    sync::Arc,
};
use axum::{extract::BodyStream, routing::post, Extension, Router};

#[allow(clippy::unused_async)] // axum doesn't like synchronous handler
async fn handler<G: Gate>(
    transport: Extension<Arc<HttpTransport<G>>>,
    req: http_serde::query::step::Request<BodyStream, G>,
) -> Result<(), Error> {
    let transport = Transport::clone_ref(&*transport);
    transport.receive_stream(req.query_id, req.step, req.origin, req.body);
    Ok(())
}

pub fn router<G: Gate>(transport: Arc<HttpTransport<G>>) -> Router {
    Router::new()
        .route(http_serde::query::step::AXUM_PATH, post(handler))
        .layer(Extension(transport))
}

#[cfg(all(test, not(feature = "shuttle"), feature = "in-memory-infra"))]
mod tests {
    use std::{future::ready, task::Poll};

    use super::*;
    use crate::{
        helpers::{HelperIdentity, MESSAGE_PAYLOAD_SIZE_BYTES},
        net::{
            server::handlers::query::test_helpers::{assert_req_fails_with, IntoFailingReq},
            test::{body_stream, TestServer},
        },
        protocol::{
            step::{self, Gate, StepNarrow},
            QueryId,
        },
    };
    use axum::http::Request;
    use futures::{
        stream::{once, poll_immediate},
        StreamExt,
    };
    use hyper::{Body, StatusCode};

    const DATA_LEN: usize = 3;

    #[tokio::test]
    async fn step() {
        let TestServer { transport, .. } = TestServer::builder().build().await;

        let step = step::Descriptive::default().narrow("test");
        let payload = vec![213; DATA_LEN * MESSAGE_PAYLOAD_SIZE_BYTES];
        let req = http_serde::query::step::Request::new(
            HelperIdentity::TWO,
            QueryId,
            step.clone(),
            body_stream(Box::new(once(ready(Ok(payload.clone().into()))))).await,
        );

        handler(Extension(Arc::clone(&transport)), req)
            .await
            .unwrap();

        let mut stream = Arc::clone(&transport).receive(HelperIdentity::TWO, (QueryId, step));

        assert_eq!(
            poll_immediate(&mut stream).next().await,
            Some(Poll::Ready(payload))
        );
    }

    struct OverrideReq<G: Gate> {
        origin: u8,
        query_id: String,
        step: G,
        payload: Vec<u8>,
    }

    impl<G: Gate> IntoFailingReq for OverrideReq<G> {
        fn into_req(self, port: u16) -> Request<Body> {
            let uri = format!(
                "http://localhost:{}{}/{}/step/{}",
                port,
                http_serde::query::BASE_AXUM_PATH,
                self.query_id,
                self.step.as_ref()
            );
            hyper::Request::post(uri)
                .header("origin", u32::from(self.origin))
                .body(hyper::Body::from(self.payload))
                .unwrap()
        }
    }

    impl<G: Gate> Default for OverrideReq<G> {
        fn default() -> Self {
            Self {
                origin: 1,
                query_id: QueryId.as_ref().to_string(),
                step: G::default().narrow("test"),
                payload: vec![1; DATA_LEN * MESSAGE_PAYLOAD_SIZE_BYTES],
            }
        }
    }

    #[tokio::test]
    async fn malformed_origin_fails() {
        let req = OverrideReq {
            origin: 4,
            ..Default::default()
        };
        assert_req_fails_with(req, StatusCode::BAD_REQUEST).await;
    }

    #[tokio::test]
    async fn malformed_query_id_fails() {
        let req = OverrideReq {
            query_id: "not-a-query-id".into(),
            ..Default::default()
        };
        assert_req_fails_with(req, StatusCode::UNPROCESSABLE_ENTITY).await;
    }
}
