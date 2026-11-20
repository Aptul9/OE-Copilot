//! /mcp: the Teams tools for AI clients with MCP_TOKEN, acting as the administrator of .env (src/app/mcp/route.ts,
//! src/lib/mcp/server.ts, src/lib/mcp/tools.ts, src/lib/mcp/access.ts). The OAuth clients of the TypeScript release
//! and their relay browser tools are not served: only the MCP_TOKEN bearer.

mod schema;
pub(crate) mod tools;
mod wire;

use crate::route;
use crate::server::Route;
use crate::web::{Error, Req, Res, http};

pub fn routes() -> Vec<Route> {
    vec![route!("POST", "/mcp", post), route!("GET", "/mcp", not_allowed), route!("DELETE", "/mcp", not_allowed)]
}

/// Why the web app must not start with this MCP_TOKEN; None when it may.
pub fn config_error() -> Option<String> {
    let token = crate::config::mcp_token();
    if token.is_empty() {
        return None;
    }
    if token.len() < 32 {
        return Some("MCP_TOKEN must be at least 32 characters (openssl rand -hex 32), or empty to turn /mcp off".into());
    }
    if crate::config::env_admin_email().is_empty() {
        return Some("MCP_TOKEN needs ADMIN_EMAIL: /mcp reads the Teams accounts of that administrator".into());
    }
    None
}

/// Authorization: Bearer <MCP_TOKEN>
pub fn token_matches(authorization: Option<&str>) -> bool {
    let expected = crate::config::mcp_token();
    let token = crate::relay::bearer_token(authorization);
    !expected.is_empty() && !token.is_empty() && crate::relay::same_token(&token, &expected)
}

/// Id of the administrator of .env; None when there is none.
pub fn mcp_user_id() -> Res<Option<String>> {
    use rusqlite::OptionalExtension;
    let email = crate::config::env_admin_email();
    if email.is_empty() {
        return Ok(None);
    }
    crate::appdb::with(|c| c.query_row(r#"SELECT id FROM "user" WHERE lower(email)=?"#, [&email], |r| r.get(0)).optional())
}

/// MCP for AI clients (Streamable HTTP): the bearer of MCP_TOKEN acts as the administrator of .env and gets the Teams
/// tools (read, send, react, edit, delete, attachments). No session cookie counts here, and a request from a web page
/// (Origin) is refused.
async fn post(req: Req) -> Res {
    if req.headers.contains_key("origin") {
        return http(403, "Requests from web pages are not accepted");
    }
    if !crate::config::mcp_token().is_empty() && token_matches(req.header("authorization").as_deref()) {
        let Some(user_id) = mcp_user_id()? else { return http(503, "The administrator of .env does not exist") };
        let accept = req.header("accept").unwrap_or_default();
        return Ok(wire::post(user_id, &accept, req.header("content-type").as_deref(), req.header("mcp-protocol-version").as_deref(), &req.body));
    }
    if crate::config::mcp_token().is_empty() {
        return http(404, "Not found");
    }
    Err(Error::Http(401, "Missing or wrong token".into(), vec![("WWW-Authenticate".into(), "Bearer realm=\"teamsrelay\"".into())]))
}

/// GET and DELETE as the SDK's stateless transport answers them.
async fn not_allowed(_req: Req) -> Res {
    Ok(wire::not_allowed())
}

#[cfg(test)]
mod tests;
