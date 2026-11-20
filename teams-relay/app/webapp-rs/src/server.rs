//! The HTTP server: the route table of src/app (route.ts files), the pages and public files of the UI for every other
//! path, and the websockets of the call sound. One thread, every task local to it (LocalSet), as Node had.

use crate::web::{Body, Error, Req, Res, Resp, empty, error_response};
use bytes::Bytes;
use http_body_util::BodyExt;
use hyper::body::Incoming;
use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

pub type Fut = Pin<Box<dyn Future<Output = Res>>>;
pub type Handler = fn(Req) -> Fut;

/// One handler of a route file: method, path pattern ([name] segments), whether the handler reads the body itself.
pub struct Route {
    pub method: &'static str,
    pub pattern: &'static str,
    pub handler: Handler,
    pub stream: bool,
}

#[macro_export]
macro_rules! route {
    ($m:expr, $p:expr, $f:path) => {
        $crate::server::Route { method: $m, pattern: $p, handler: |r| Box::pin($f(r)), stream: false }
    };
    ($m:expr, $p:expr, $f:path, stream) => {
        $crate::server::Route { method: $m, pattern: $p, handler: |r| Box::pin($f(r)), stream: true }
    };
}

/// The parameters of a path matching a pattern, None when it does not match.
pub fn matches(pattern: &str, path: &str) -> Option<Vec<(String, String)>> {
    let p: Vec<&str> = pattern.trim_start_matches('/').split('/').collect();
    let s: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    if p.len() != s.len() {
        return None;
    }
    let mut params = vec![];
    for (a, b) in p.iter().zip(&s) {
        if let Some(name) = a.strip_prefix('[').and_then(|x| x.strip_suffix(']')) {
            if b.is_empty() {
                return None;
            }
            params.push((name.to_string(), crate::js::decode_uri_component(b).unwrap_or_else(|| b.to_string())));
        } else if a != b {
            return None;
        }
    }
    Some(params)
}

/// The largest body read for a route that does not stream its own.
const MAX_BODY: usize = 80 * 1024 * 1024;

async fn read_body(body: Incoming) -> Result<Bytes, Error> {
    let mut out: Vec<u8> = vec![];
    let mut body = body;
    while let Some(frame) = body.frame().await {
        let frame = frame.map_err(|e| Error::Internal(format!("body: {e}")))?;
        if let Some(d) = frame.data_ref() {
            out.extend_from_slice(d);
            if out.len() > MAX_BODY {
                return Err(crate::web::err(413, "Body too large"));
            }
        }
    }
    Ok(Bytes::from(out))
}

thread_local! {
    /// The body of a request whose route streams it, taken by the handler (stream_body).
    static STREAMS: std::cell::RefCell<Vec<(u64, Incoming)>> = const { std::cell::RefCell::new(Vec::new()) };
    static NEXT: std::cell::Cell<u64> = const { std::cell::Cell::new(1) };
}

/// The body of a streaming route, as the request carried it (once).
pub fn take_stream(req: &Req) -> Option<Incoming> {
    let id = req.stream_id?;
    STREAMS.with(|s| {
        let mut s = s.borrow_mut();
        let i = s.iter().position(|(x, _)| *x == id)?;
        Some(s.remove(i).1)
    })
}

fn put_stream(b: Incoming) -> u64 {
    let id = NEXT.with(|n| {
        let v = n.get();
        n.set(v + 1);
        v
    });
    STREAMS.with(|s| s.borrow_mut().push((id, b)));
    id
}

fn drop_stream(id: Option<u64>) {
    if let Some(id) = id {
        STREAMS.with(|s| s.borrow_mut().retain(|(x, _)| *x != id));
    }
}

pub async fn handle(req: hyper::Request<Incoming>, peer: Option<std::net::IpAddr>) -> Resp {
    let (mut parts, body) = req.into_parts();
    // X-Authentik-* count only when they come from the Caddy of this compose (everid::screen)
    crate::everid::screen(&mut parts.headers, peer, &crate::config::everid_proxy_host()).await;
    let path = parts.uri.path().to_string();
    let query = parts.uri.query().unwrap_or("").to_string();
    let method = parts.method.as_str().to_string();
    // a trailing slash is redirected to the path without it (Next.js trailingSlash: false)
    if path.len() > 1 && path.ends_with('/') {
        let to = format!("{}{}{}", path.trim_end_matches('/'), if query.is_empty() { "" } else { "?" }, query);
        let to = if to.is_empty() || to.starts_with('?') { format!("/{to}") } else { to };
        return hyper::Response::builder().status(308).header("location", to).body(http_body_util::Empty::new().boxed_unsync()).expect("response");
    }
    if let Some(r) = crate::hubs::upgrade(&mut parts, &path, &query).await {
        return r;
    }
    // app/api/auth/[...all]: GET and POST to Better Auth
    if path.starts_with("/api/auth/") && path.len() > "/api/auth/".len() {
        if method != "GET" && method != "POST" && method != "HEAD" {
            return empty(405);
        }
        let body = match read_body(body).await {
            Ok(b) => b,
            Err(e) => return error_response(e),
        };
        let req = Req {
            method: if method == "HEAD" { "GET".into() } else { method.clone() },
            path,
            query,
            headers: parts.headers,
            body,
            params: vec![],
            stream_id: None,
        };
        return crate::auth::routes::handle(&req).await;
    }
    let table = crate::routes::table();
    let mut found_path = false;
    let mut allowed: Vec<&str> = vec![];
    for r in table.iter() {
        let Some(params) = matches(r.pattern, &path) else { continue };
        found_path = true;
        allowed.push(r.method);
        let head = method == "HEAD" && r.method == "GET";
        if r.method != method && !head {
            continue;
        }
        let mut req = Req {
            method: if head { "GET".into() } else { method.clone() },
            path: path.clone(),
            query: query.clone(),
            headers: parts.headers.clone(),
            body: Bytes::new(),
            params,
            stream_id: None,
        };
        if r.stream {
            req.stream_id = Some(put_stream(body));
        } else {
            match read_body(body).await {
                Ok(b) => req.body = b,
                Err(e) => return error_response(e),
            }
        }
        let id = req.stream_id;
        let out = match (r.handler)(req).await {
            Ok(resp) => resp,
            Err(e) => error_response(e),
        };
        drop_stream(id);
        if head {
            let (p, _) = out.into_parts();
            return hyper::Response::from_parts(p, http_body_util::Empty::new().boxed_unsync());
        }
        return out;
    }
    if found_path {
        if method == "OPTIONS" {
            let mut all = allowed.clone();
            all.push("OPTIONS");
            if all.contains(&"GET") {
                all.push("HEAD");
            }
            return hyper::Response::builder().status(204).header("allow", all.join(", ")).body(http_body_util::Empty::new().boxed_unsync()).expect("response");
        }
        return empty(405);
    }
    crate::pages::serve(&method, &path, &query, &parts.headers).await
}

/// Serves until a stop signal, then lets the open requests finish for up to 10 s.
pub async fn serve(listener: tokio::net::TcpListener, stop: impl Future<Output = ()>) {
    const SLOTS: u32 = 1 << 20;
    let (shutdown, _) = tokio::sync::watch::channel(false);
    let open = std::sync::Arc::new(tokio::sync::Semaphore::new(SLOTS as usize));
    tokio::pin!(stop);
    loop {
        let (stream, addr) = tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok(a) => a,
                Err(_) => continue,
            },
            _ = &mut stop => break,
        };
        let peer = Some(addr.ip());
        let mut down = shutdown.subscribe();
        let guard = open.clone().try_acquire_owned().ok();
        tokio::task::spawn_local(async move {
            let _guard = guard;
            let service = hyper::service::service_fn(move |req| async move { Ok::<Resp, Infallible>(handle(req, peer).await) });
            let conn = hyper::server::conn::http1::Builder::new()
                .timer(hyper_util::rt::TokioTimer::new())
                .header_read_timeout(Duration::from_secs(30))
                .serve_connection(hyper_util::rt::TokioIo::new(stream), service)
                .with_upgrades();
            tokio::pin!(conn);
            tokio::select! {
                _ = conn.as_mut() => {}
                _ = down.changed() => {
                    conn.as_mut().graceful_shutdown();
                    let _ = conn.await;
                }
            }
        });
    }
    let _ = shutdown.send(true);
    let _ = tokio::time::timeout(Duration::from_secs(10), open.acquire_many(SLOTS)).await;
}

pub fn boxed(b: Body) -> Body {
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patterns() {
        assert_eq!(matches("/api/accounts/[slot]/check", "/api/accounts/3/check"), Some(vec![("slot".into(), "3".into())]));
        assert_eq!(matches("/api/accounts/[slot]", "/api/accounts/3/check"), None);
        assert_eq!(matches("/files/[file]", "/files/a%20b"), Some(vec![("file".into(), "a b".into())]));
        assert_eq!(matches("/api/chats", "/api/chats"), Some(vec![]));
    }
}
