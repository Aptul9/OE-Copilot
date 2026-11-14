//! The shared Microsoft accounts of the OE calendar agent (src/lib/shared-accounts.ts, src/lib/shared-signin.ts,
//! src/app/api/shared-accounts/route.ts): a slot runs for each account with Teams on, owned by the owner of the shared
//! accounts.
//!
//! STUB: implemented in full by the shared accounts port.

use crate::server::Route;
use crate::web::Res;

/// /api/shared-accounts (GET, POST).
pub fn routes() -> Vec<Route> {
    vec![]
}

/// The id of the user the shared accounts belong to (SHARED_ACCOUNTS_OWNER, else ADMIN_EMAIL); None when none.
pub fn shared_owner_id() -> Res<Option<String>> {
    Ok(None)
}

/// Teams on or off for a shared account, in the directory of the agent; 404 when the account is not there.
pub async fn set_shared_chat(_account: &str, _on: bool) -> Res<()> {
    Ok(())
}

/// One pass of the sync, now (syncSharedAccounts with the options of the web app).
pub async fn sync_now() -> Res<()> {
    Ok(())
}

/// The loop of the sync, started at boot (keepSharedAccountsInSync).
pub fn keep_in_sync() {}
