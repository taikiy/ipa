mod create;
mod input;
mod prepare;
mod results;
mod step;

use crate::{net::HttpTransport, protocol::step::Gate, sync::Arc};
use axum::Router;

/// Construct router for IPA query web service
///
/// In principle, this web service could be backed by either an HTTP-interconnected helper network or
/// an in-memory helper network. These are the APIs used by external callers (report collectors) to
/// examine attribution results.
pub fn query_router<G: Gate>(transport: Arc<HttpTransport<G>>) -> Router {
    Router::new()
        .merge(create::router(Arc::clone(&transport)))
        .merge(input::router(Arc::clone(&transport)))
        .merge(results::router(transport))
}

/// Construct router for helper-to-helper communications
///
/// This only makes sense in the context of an HTTP-interconnected helper network. These APIs are
/// called by peer helpers to exchange MPC step data, and by whichever helper is the leader for a
/// particular query, to coordinate servicing that query.
//
// It might make sense to split the query and h2h handlers into two modules.
pub fn h2h_router<G: Gate>(transport: Arc<HttpTransport<G>>) -> Router {
    Router::new()
        .merge(prepare::router(Arc::clone(&transport)))
        .merge(step::router(transport))
}

#[cfg(all(test, not(feature = "shuttle"), feature = "in-memory-infra"))]
pub mod test_helpers {
    use crate::net::test::TestServer;
    use futures_util::future::poll_immediate;
    use hyper::{service::Service, StatusCode};
    use tower::ServiceExt;

    /// types that implement `IntoFailingReq` are intended to induce some failure in the process of
    /// axum routing. Pair with `assert_req_fails_with` to detect specific [`StatusCode`] failures.
    pub trait IntoFailingReq {
        fn into_req(self, port: u16) -> hyper::Request<hyper::Body>;
    }

    /// Intended to be used for a request that will fail during axum routing. When passed a known
    /// bad request via `IntoFailingReq`, get a response from the server, and compare its
    /// [`StatusCode`] with what is expected.
    pub async fn assert_req_fails_with<I: IntoFailingReq>(req: I, expected_status: StatusCode) {
        let TestServer { server, .. } = TestServer::default().await;

        let mut router = server.router();
        let ready = poll_immediate(router.ready()).await.unwrap().unwrap();
        let resp = poll_immediate(ready.call(req.into_req(0)))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(resp.status(), expected_status);
    }
}
