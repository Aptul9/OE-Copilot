//! /mcp: the Teams tools for AI clients with MCP_TOKEN, acting as the administrator of .env (src/app/mcp/route.ts,
//! src/lib/mcp/server.ts, src/lib/mcp/tools.ts, src/lib/mcp/access.ts).
//!
//! STUB: implemented in full by the MCP port.

use crate::server::Route;

pub fn routes() -> Vec<Route> {
    vec![]
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
