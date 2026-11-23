//! The strangler's other half: with `-fallback http://127.0.0.1:18097`, every request beatscript-serve does not own (a route
//! group not moved yet) is passed to the Go server as it came, and its answer comes back as Go wrote it: status,
//! headers, body, streamed both ways (SSE and render streams included). beatscript-serve adds nothing to a proxied answer.
//!
//! The Go server trusts `X-Real-IP` from a loopback peer (`api.ResolveClientIP`), which beatscript-serve is: nginx's header
//! is passed on, and a request that reached beatscript-serve directly gets its peer's address.
//!
//! Which paths are Go's while a group has not moved is `go_owned` (the Go mux's patterns that beatscript-serve does not
//! answer). The static site is the Go mux's catch-all, so it is served here only for paths no Go pattern takes.

use std::net::SocketAddr;

use axum::body::Body;
use axum::http::{header, HeaderValue, Request, Response, Uri};
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;

pub struct Fallback {
    base: String,
    client: Client<HttpConnector, Body>,
}

/// Marks a response as the Go server's: the header layer leaves it alone.
#[derive(Clone, Copy)]
pub struct Proxied;

/// The Go server's routes that beatscript-serve does not serve (yet). Exact paths and prefixes, from `server.routes`,
/// `pairRoutes`, `auth.Mount`, `wire` (compose, admin, MCP), `card.Mount` and `ld.Mount`. Everything under `/api/`
/// beatscript-serve does not own is Go's too, so a route added to Go later is never served as a static 404 here.
pub fn go_owned(path: &str) -> bool {
    // Every route group has moved (P1-P7): no path is the Go server's. `-fallback` is kept as a flag so a route found
    // missing after the cutover can be sent back to Go by listing it here, without an nginx change.
    const EXACT: &[&str] = &[];
    const PREFIX: &[&str] = &[];
    EXACT.contains(&path) || PREFIX.iter().any(|p| path.starts_with(p))
}

impl Fallback {
    pub fn new(base: &str) -> Fallback {
        let mut c = HttpConnector::new();
        c.set_nodelay(true);
        let client = Client::builder(TokioExecutor::new()).http1_title_case_headers(true).build(c);
        Fallback { base: base.trim_end_matches('/').to_string(), client }
    }

    pub async fn forward(&self, mut req: Request<Body>, peer: SocketAddr) -> Response<Body> {
        let pq = req.uri().path_and_query().map_or("/", |p| p.as_str()).to_string();
        let Ok(uri) = format!("{}{pq}", self.base).parse::<Uri>() else {
            return crate::ApiError::new(502, "bad_gateway", "bad upstream URL").response();
        };
        *req.uri_mut() = uri;
        let h = req.headers_mut();
        if !h.contains_key("x-real-ip") || !peer.ip().is_loopback() {
            // a direct client must not choose its own address
            h.insert("x-real-ip", HeaderValue::from_str(&peer.ip().to_string()).unwrap());
        }
        // hop-by-hop
        for n in [header::CONNECTION, header::UPGRADE, header::TE, header::TRAILER, header::PROXY_AUTHORIZATION] {
            h.remove(n);
        }
        h.remove("keep-alive");
        match self.client.request(req).await {
            Ok(r) => {
                let mut r = r.map(Body::new);
                r.headers_mut().remove(header::CONNECTION);
                r.headers_mut().remove("keep-alive");
                r.extensions_mut().insert(Proxied);
                r
            }
            Err(e) => {
                eprintln!("beatscript-serve: fallback {}: {e}", self.base);
                let mut r = crate::ApiError::new(502, "bad_gateway", "the Go server did not answer").response();
                r.extensions_mut().insert(Proxied);
                r
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::go_owned;

    #[test]
    fn owned() {
        for p in ["/", "/mcp", "/pair", "/api/pair", "/api/session/x/events", "/api/devices/", "/api/compose", "/api/admin/stats", "/p/zabc", "/c/zabc", "/api/check", "/api/nope"] {
            assert!(!go_owned(p), "{p}");
        }
    }
}
