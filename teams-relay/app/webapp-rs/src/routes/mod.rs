//! The route table: one entry per handler of a route.ts file of src/app.

pub mod accounts;
pub mod admin;
pub mod calls;
pub mod events;
pub mod everflow;
pub mod messages;
pub mod relay;

use crate::server::Route;
use std::sync::OnceLock;

pub fn table() -> &'static [Route] {
    static TABLE: OnceLock<Vec<Route>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut t = vec![];
        t.extend(accounts::routes());
        t.extend(admin::routes());
        t.extend(calls::routes());
        t.extend(events::routes());
        t.extend(everflow::routes());
        t.extend(messages::routes());
        t.extend(relay::routes());
        t.extend(crate::mcp::routes());
        t.extend(crate::status::routes());
        t.extend(crate::shared_accounts::routes());
        t
    })
}
