//! The public status pages of the status challenge (src/lib/status-pages.ts, src/lib/status-stats.ts,
//! src/shared/status-db.ts, app/status-challenge-spec.md).
//!
//! STUB: implemented in full by the status port.

use crate::server::Route;
use crate::web::Res;
use serde_json::Value;

/// The routes: /api/admin/status-pages (GET, POST, PATCH, DELETE) and /api/status/[slug] (GET).
pub fn routes() -> Vec<Route> {
    vec![]
}

/// The answer of a public status page for /[slug] (statusAnswer), None when the slug is no page.
pub fn page_answer(_slug: &str) -> Res<Option<Value>> {
    Ok(None)
}
