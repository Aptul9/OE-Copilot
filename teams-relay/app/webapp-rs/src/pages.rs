//! The pages and public files of the UI: the static export of the app (`next build` with TR_EXPORT=1, UI_DIR). The pages
//! of src/app/*/page.tsx check the session and compute their props here, as their server components did, and hand them
//! to the page in `window.__TR__`, written in the head before any script. The RSC payloads (.txt) of the export are not
//! served: a client navigation that asks for one falls back to loading the page, which gets its props from here.

use crate::auth::{SessionUser, current_user, login_for, login_url, safe_next};
use crate::web::Req;
use bytes::Bytes;
use http_body_util::{BodyExt, Empty, Full};
use hyper::header::HeaderMap;
use serde_json::{Value, json};
use std::path::{Component, Path, PathBuf};

const NO_STORE: &str = "private, no-cache, no-store, max-age=0, must-revalidate";

fn root() -> PathBuf {
    crate::config::ui_dir()
}

fn kind(p: &Path) -> &'static str {
    match p.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
        Some("html") => "text/html; charset=utf-8",
        Some("js" | "mjs") => "application/javascript; charset=UTF-8",
        Some("css") => "text/css; charset=UTF-8",
        Some("json") => "application/json",
        Some("webmanifest") => "application/manifest+json",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("svg") => "image/svg+xml",
        Some("ico") => "image/x-icon",
        Some("woff2") => "font/woff2",
        Some("woff") => "font/woff",
        Some("txt") => "text/plain; charset=utf-8",
        Some("map") => "application/json",
        Some("mp3") => "audio/mpeg",
        Some("ogg") => "audio/ogg",
        Some("wav") => "audio/wav",
        _ => "application/octet-stream",
    }
}

/// The file of the export for a path: no .. or odd segments; no pages (.html) and no page payloads (.txt) outside
/// /_next/.
pub fn static_file(path: &str) -> Option<PathBuf> {
    let decoded = crate::js::decode_uri_component(path)?;
    let rel = decoded.strip_prefix('/')?;
    if rel.is_empty() || rel.contains('\\') || rel.contains('\0') {
        return None;
    }
    let p = Path::new(rel);
    if !p.components().all(|c| matches!(c, Component::Normal(_))) {
        return None;
    }
    let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    if !rel.starts_with("_next/") && (ext == "html" || ext == "txt") {
        return None;
    }
    let file = root().join(p);
    std::fs::metadata(&file).ok().filter(|m| m.is_file()).map(|_| file)
}

fn serve_file(file: &Path, path: &str, head: bool) -> crate::web::Resp {
    let Ok(data) = std::fs::read(file) else { return crate::web::empty(404) };
    let cache = if path.starts_with("/_next/static/") { "public, max-age=31536000, immutable" } else { "public, max-age=0" };
    let mut b = hyper::Response::builder()
        .status(200)
        .header("content-type", kind(file))
        .header("content-length", data.len().to_string())
        .header("cache-control", cache);
    if path == "/sw.js" {
        b = b.header("service-worker-allowed", "/");
    }
    let body = if head { Empty::new().boxed_unsync() } else { Full::new(Bytes::from(data)).boxed_unsync() };
    b.body(body).expect("response")
}

/// The JSON inside a <script>: nothing that closes the tag or opens an HTML comment.
fn in_script(v: &Value) -> String {
    crate::js::stringify(v)
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('&', "\\u0026")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029")
}

/// A page of the export with its props from the server; `title` replaces its title, `noindex` adds the robots meta.
fn page(name: &str, props: &Value, status: u16, head: bool, title: Option<&str>, noindex: bool) -> crate::web::Resp {
    let Ok(mut html) = std::fs::read_to_string(root().join(name)) else {
        eprintln!("pages: {name} not found in {}", root().display());
        return crate::web::bytes(
            500,
            &[("content-type", "text/plain; charset=utf-8".into()), ("cache-control", "no-store".into())],
            b"UI not installed\n".to_vec(),
        );
    };
    if let Some(t) = title
        && let (Some(a), Some(b)) = (html.find("<title>"), html.find("</title>"))
    {
        html = format!("{}<title>{}{}", &html[..a], crate::web::html_escape(t), &html[b..]);
    }
    let mut extra = format!("<script>window.__TR__={}</script>", in_script(props));
    if noindex {
        extra.insert_str(0, r#"<meta name="robots" content="noindex, nofollow"/>"#);
    }
    let html = match html.find("<head>") {
        Some(i) => format!("{}{extra}{}", &html[..i + 6], &html[i + 6..]),
        None => format!("{extra}{html}"),
    };
    let len = html.len();
    let body = if head { Empty::new().boxed_unsync() } else { Full::new(Bytes::from(html)).boxed_unsync() };
    hyper::Response::builder()
        .status(status)
        .header("content-type", "text/html; charset=utf-8")
        .header("content-length", len.to_string())
        .header("cache-control", NO_STORE)
        .body(body)
        .expect("response")
}

/// redirect() of Next.js in a page: 307.
fn to(location: &str) -> crate::web::Resp {
    hyper::Response::builder().status(307).header("location", location).header("cache-control", NO_STORE).body(Empty::new().boxed_unsync()).expect("response")
}

pub fn not_found(head: bool) -> crate::web::Resp {
    page("404.html", &json!({}), 404, head, None, false)
}

fn user_props(u: &SessionUser) -> Value {
    json!({ "name": u.name, "email": u.email, "role": u.role_or_user() })
}

pub async fn serve(method: &str, path: &str, query: &str, headers: &HeaderMap) -> crate::web::Resp {
    let head = method == "HEAD";
    if method != "GET" && !head {
        return if static_file(path).is_some() || matches!(path, "/" | "/admin" | "/login" | "/remote" | "/settings") {
            crate::web::empty(405)
        } else {
            not_found(false)
        };
    }
    if let Some(f) = static_file(path) {
        return serve_file(&f, path, head);
    }
    let req =
        Req { method: "GET".into(), path: path.into(), query: query.into(), headers: headers.clone(), body: Bytes::new(), params: vec![], stream_id: None };
    match render(&req, head) {
        Ok(r) => r,
        Err(e) => crate::web::error_response(e),
    }
}

fn render(req: &Req, head: bool) -> crate::web::Res {
    let q = req.query_pairs();
    Ok(match req.path.as_str() {
        "/" => match current_user(req)? {
            None => to(&login_for(&q)),
            Some(u) => page("index.html", &json!({ "user": user_props(&u), "desktopUrl": crate::config::desktop_url() }), 200, head, None, false),
        },
        "/admin" => match current_user(req)? {
            None => to(&login_url("/admin")),
            Some(u) if !u.is_admin() => to("/"),
            Some(u) => page("admin.html", &json!({ "selfId": u.id }), 200, head, None, false),
        },
        "/login" => {
            let next = safe_next(req.query_param("next").as_deref());
            match current_user(req)? {
                Some(_) => to(&next),
                None => page("login.html", &json!({ "next": next }), 200, head, None, false),
            }
        }
        "/remote" => {
            let account = q.iter().find(|(k, _)| k == "account").map(|(_, v)| v.clone()).unwrap_or_default();
            let asked = if !account.is_empty() && account.bytes().all(|b| b.is_ascii_digit()) { account } else { String::new() };
            match current_user(req)? {
                None => to(&login_url(&if asked.is_empty() { "/remote".to_string() } else { format!("/remote?account={asked}") })),
                Some(u) => {
                    let accounts = crate::accounts::accounts_of(&u.id)?;
                    let initial = crate::js::number(Some(&Value::String(asked)));
                    page("remote.html", &json!({ "accounts": accounts["accounts"], "initial": crate::web::num(initial) }), 200, head, None, false)
                }
            }
        }
        "/settings" => match current_user(req)? {
            None => to(&login_url("/settings")),
            Some(u) => page(
                "settings.html",
                &json!({ "user": user_props(&u), "passwordManaged": crate::config::is_env_admin(Some(&u.email)) }),
                200,
                head,
                None,
                false,
            ),
        },
        p => {
            let slug = p.strip_prefix('/').unwrap_or("");
            if slug.is_empty() || slug.contains('/') {
                return Ok(not_found(head));
            }
            let slug = crate::js::decode_uri_component(slug).unwrap_or_else(|| slug.to_string());
            match crate::status::page_answer(&slug)? {
                None => not_found(head),
                Some(answer) => {
                    let title = format!("{} · Status challenge", answer.get("name").and_then(Value::as_str).unwrap_or(""));
                    let props = json!({ "slug": slug, "initial": answer, "serverNow": crate::web::now_s() });
                    page("status.html", &props, 200, head, Some(&title), true)
                }
            }
        }
    })
}
