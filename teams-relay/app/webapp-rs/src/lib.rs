//! The web app of TeamsRelay in Rust: the API, users and sessions of src/app (route.ts files) and src/lib, the boot of
//! src/server, and the pages of the UI exported as static files. Same database, same cookies, same answers.

pub mod accounts;
pub mod agent;
pub mod appdb;
pub mod auth;
pub mod checks;
pub mod commands;
pub mod config;
pub mod control;
pub mod hubs;
pub mod js;
pub mod light;
pub mod mcp;
pub mod pages;
pub mod push;
pub mod relay;
pub mod routes;
pub mod server;
pub mod shared_accounts;
pub mod slotdb;
pub mod slots;
pub mod state;
pub mod status;
pub mod uploads;
pub mod web;
pub mod zod;

/// The tests that set the process environment (APP_DB, TZ, UI_DIR) take this, one at a time: the tests of one binary
/// run on parallel threads of one process.
#[cfg(test)]
pub static TEST_ENV: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Shared HTTP client: no redirects, rustls with the webpki roots.
pub fn http_client() -> &'static reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).connect_timeout(std::time::Duration::from_secs(15)).build().expect("HTTP client")
    })
}

/// The administrator of .env exists, is an administrator, is not banned and signs in with ADMIN_PASSWORD; a new password
/// in .env signs it out everywhere (src/server/env-admin.ts). What was done, for the log.
pub async fn sync_env_admin() -> Result<String, String> {
    use auth::rows;
    let fail = |e: web::Error| e.to_string();
    let users = rows::count_users().map_err(fail)?;
    let email = config::env_admin_email();
    let password = std::env::var("ADMIN_PASSWORD").unwrap_or_default();
    if email.is_empty() || password.is_empty() {
        if users == 0 {
            return Err("No users yet: set ADMIN_EMAIL and ADMIN_PASSWORD in .env to create the first administrator".into());
        }
        return Ok("no administrator in .env".into());
    }
    if password.encode_utf16().count() < auth::PASSWORD_MIN {
        return Err(format!("ADMIN_PASSWORD must be at least {} characters", auth::PASSWORD_MIN));
    }
    let Some(u) = rows::user_by_email(&email).map_err(fail)? else {
        let name = std::env::var("ADMIN_NAME").ok().filter(|n| !n.is_empty()).unwrap_or_else(|| "Administrator".into());
        let id = routes::admin::create_user(&email, &name, &password, "admin").await.map_err(fail)?;
        if users != 0 {
            return Ok(format!("administrator {email} created"));
        }
        let (slots, devices) = appdb::with(|c| appdb::adopt_legacy_data(c, &id)).map_err(fail)?;
        return Ok(format!("administrator {email} created, {slots} existing accounts and {devices} devices assigned"));
    };
    let mut done = vec![];
    let now = web::now_ms();
    if u.role.as_deref() != Some("admin") || u.banned == Some(true) {
        rows::update_user_admin(&u.id, now).map_err(fail)?;
        done.push("administrator rights restored");
    }
    let hash = rows::password_of(&u.id).map_err(fail)?;
    let good = match hash.clone().flatten() {
        Some(h) => auth::crypto::verify_password_async(h, password.clone()).await.unwrap_or(false),
        None => false,
    };
    if !good {
        let new = auth::crypto::hash_password_async(password).await;
        if hash.is_some() {
            rows::update_password(&u.id, &new, now).map_err(fail)?;
        } else {
            rows::link_account(&u.id, &new, now).map_err(fail)?;
        }
        rows::delete_user_sessions(&u.id).map_err(fail)?;
        done.push("password set from ADMIN_PASSWORD, signed out everywhere");
    }
    Ok(format!("administrator {email}: {}", if done.is_empty() { "in line with .env".into() } else { done.join(", ") }))
}
