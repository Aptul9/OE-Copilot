//! Images, profile pictures, the logo of the company of the account (data/N/media) and attachments (data/N/files) of the
//! browser agent (src/agent/media.ts, logic/files.ts): a file is written once and kept while a row names it.

use crate::agent::browser::Page;
use crate::agent::http::{Http, send};
use crate::agent::media::{is_placeholder_image, sha16};
use crate::agent::{info, now_ms, warn};
use base64::Engine;
use serde_json::{Value, json};
use std::cell::RefCell;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::LazyLock;

const MAX_IMAGE: usize = 8_000_000;
const MAX_LOGO: usize = 256_000;
const LOGO_EVERY_MS: i64 = 86_400_000;
pub const MAX_DOWNLOAD: usize = 100_000_000;
const MEDIA_EXT: [(&str, &str); 4] = [("image/png", "png"), ("image/jpeg", "jpg"), ("image/gif", "gif"), ("image/webp", "webp")];
static BRANDING_HOST: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r"^https://[a-z0-9-]+\.(msauthimages|msftauthimages)\.net/").expect("fixed"));
static AMS_HOST: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r"^https://[a-z0-9-]+\.asm\.skype\.com/").expect("fixed"));
/// The names the web app serves: 16 hex characters and an extension (MEDIA_NAME)
static MEDIA_NAME: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r"^[0-9a-f]{16}\.(png|jpg|gif|webp)$").expect("fixed"));

pub fn avatar_file(src: &str) -> String {
    format!("{}.png", sha16(src))
}

/// An attachment's file: the hash of its address and its extension when it has a plain one.
pub fn download_file(url: &str, name: &str) -> String {
    let ext = std::path::Path::new(name).extension().and_then(|e| e.to_str()).map(|e| e.to_lowercase()).unwrap_or_default();
    let plain = !ext.is_empty() && ext.len() <= 8 && ext.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit());
    format!("{}{}", sha16(url), if plain { format!(".{ext}") } else { String::new() })
}

/// Attachments are downloaded only from SharePoint and OneDrive for work, over HTTPS.
pub fn is_share_point_url(url: &str) -> bool {
    url.starts_with("https://") && url::Url::parse(url).ok().and_then(|u| u.host_str().map(|h| h.ends_with(".sharepoint.com"))).unwrap_or(false)
}

pub fn behind_mcas(page_url: &str) -> bool {
    url::Url::parse(page_url).ok().and_then(|u| u.host_str().map(|h| h.ends_with(".mcas.ms"))).unwrap_or(false)
}

fn logo_ext(d: &[u8]) -> Option<&'static str> {
    if d.starts_with(&[0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]) {
        Some("png")
    } else if d.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("jpg")
    } else {
        None
    }
}

pub struct Media {
    media_dir: PathBuf,
    files_dir: PathBuf,
    http: Http,
    /// addresses that could not be fetched, per image: no new attempt
    failed: RefCell<HashSet<String>>,
    logo_asked: RefCell<(String, i64)>,
}

impl Media {
    pub fn new(media_dir: PathBuf, files_dir: PathBuf, http: Http) -> Media {
        Media { media_dir, files_dir, http, failed: RefCell::default(), logo_asked: RefCell::new((String::new(), 0)) }
    }

    fn write(&self, name: &str, data: &[u8]) -> bool {
        let _ = std::fs::create_dir_all(&self.media_dir);
        std::fs::write(self.media_dir.join(name), data).is_ok()
    }

    /// File of the image `key` of a message: saved already, or fetched by the page, or for an AMS image with a chatsvc
    /// token by the agent itself; None when it cannot be read.
    pub async fn image(&self, page: Option<&Page>, key: &str, src: &str, token: &str) -> Option<String> {
        for (_, ext) in MEDIA_EXT {
            let file = self.media_dir.join(format!("{key}.{ext}"));
            let Ok(st) = std::fs::metadata(&file) else { continue };
            if st.len() > 1000 || !std::fs::read(&file).is_ok_and(|d| is_placeholder_image(&d)) {
                return Some(format!("{key}.{ext}"));
            }
            let _ = std::fs::remove_file(&file);
            info("media", "placeholder removed", &[("file", format!("{key}.{ext}").into())]);
        }
        let attempt = format!("{key} {src}");
        if src.is_empty() || self.failed.borrow().contains(&attempt) {
            return None;
        }
        let mut got: Option<(String, Vec<u8>)> = None;
        if !token.is_empty() && AMS_HOST.is_match(src) {
            match send(&self.http, "GET", src, vec![("Authorization", format!("Bearer {token}"))], vec![], 30_000).await {
                Ok(r) if r.ok() && r.body.len() <= MAX_IMAGE => {
                    got = Some((r.header("content-type").unwrap_or("").split(';').next().unwrap_or("").trim().to_string(), r.body))
                }
                Ok(_) => {}
                Err(e) => warn("media", &crate::agent::text_of(&e), &[("src", crate::agent::cut(src, 60).into())]),
            }
        } else if let Some(page) = page {
            match page.evaluate("media.fetchImage", Some(&json!({ "src": src, "max": MAX_IMAGE }))).await {
                Ok(v) if v.is_object() => {
                    let data = base64::engine::general_purpose::STANDARD.decode(v["data"].as_str().unwrap_or("")).unwrap_or_default();
                    got = Some((v["type"].as_str().unwrap_or("").to_string(), data));
                }
                Ok(_) => {}
                Err(e) => warn("media", &e, &[("src", crate::agent::cut(src, 60).into())]),
            }
        }
        let ext = got.as_ref().and_then(|(t, _)| MEDIA_EXT.iter().find(|(m, _)| m == t).map(|(_, e)| *e));
        let (Some((_, data)), Some(ext)) = (got, ext) else {
            self.failed.borrow_mut().insert(attempt);
            return None;
        };
        // still loading in Teams: the next read gets the image
        if is_placeholder_image(&data) {
            return None;
        }
        self.write(&format!("{key}.{ext}"), &data).then(|| format!("{key}.{ext}"))
    }

    /// File of a profile picture drawn in the page, copied once; `budget` caps the copies of one round.
    pub async fn avatar(&self, page: &Page, src: &str, budget: &mut usize) -> String {
        if src.is_empty() {
            return String::new();
        }
        let file = avatar_file(src);
        if self.media_dir.join(&file).exists() {
            return file;
        }
        if *budget == 0 {
            return String::new();
        }
        *budget -= 1;
        let Ok(Value::String(data)) = page.evaluate("media.copyImage", Some(&json!(src))).await else { return String::new() };
        let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(data) else { return String::new() };
        if self.write(&file, &bytes) { file } else { String::new() }
    }

    /// The square logo of the Microsoft sign-in branding of the account's organization, else its banner made square by
    /// the page. File name; "" when there is none to take, None when Microsoft or the page could not be reached.
    pub async fn logo(&self, email: &str, page: Option<&Page>) -> Option<String> {
        let run = async {
            let body = crate::js::stringify(&json!({ "username": email })).into_bytes();
            let r = send(
                &self.http,
                "POST",
                "https://login.microsoftonline.com/common/GetCredentialType",
                vec![("Content-Type", "application/json".into())],
                body,
                15_000,
            )
            .await
            .map_err(|e| e.to_string())?;
            if !r.ok() {
                return Ok(None);
            }
            let answer = crate::agent::http::json_or_empty(&r);
            let branding = &answer["EstsProperties"]["UserTenantBranding"][0];
            // String(x ?? ""): missing or null is ""
            let text = |k: &str| branding.get(k).filter(|v| !v.is_null()).map(|v| crate::js::string(Some(v))).unwrap_or_default();
            let tile = text("TileLogo");
            let url = if tile.is_empty() { text("BannerLogo") } else { tile.clone() };
            if !BRANDING_HOST.is_match(&url) || (tile.is_empty() && page.is_none()) {
                return Ok(Some(String::new()));
            }
            let img = send(&self.http, "GET", &url, vec![], vec![], 15_000).await.map_err(|e| e.to_string())?;
            if !img.ok() {
                return Ok(None);
            }
            let mut data = img.body;
            let mut ext = if data.len() <= MAX_LOGO { logo_ext(&data) } else { None };
            if ext.is_none() {
                return Ok(Some(String::new()));
            }
            if tile.is_empty() {
                let b64 = base64::engine::general_purpose::STANDARD.encode(&data);
                let square = page.ok_or("no page")?.evaluate("media.squareLogo", Some(&json!(b64))).await?;
                data = base64::engine::general_purpose::STANDARD.decode(square.as_str().unwrap_or("")).unwrap_or_default();
                ext = (!data.is_empty() && data.len() <= MAX_LOGO).then_some("png");
                if ext.is_none() {
                    return Ok(Some(String::new()));
                }
            }
            let file = format!("{}.{}", &avatar_file(&url)[..16], ext.unwrap_or("png"));
            if !self.write(&file, &data) {
                return Ok(None);
            }
            Ok::<Option<String>, String>(Some(file))
        };
        match run.await {
            Ok(v) => v,
            Err(e) => {
                warn("media", &format!("logo: {e}"), &[]);
                None
            }
        }
    }

    /// The logo in the identity of the account, asked for once a day and at once for another organization: the one known
    /// stays while Microsoft or the page cannot be reached.
    pub async fn company_logo(&self, email: &str, tenant: &str, saved: &str, page: Option<&Page>) -> String {
        let (asked, at) = self.logo_asked.borrow().clone();
        if email.is_empty() || (asked == tenant && now_ms() - at < LOGO_EVERY_MS) {
            return saved.to_string();
        }
        *self.logo_asked.borrow_mut() = (tenant.to_string(), now_ms());
        match self.logo(email, page).await {
            Some(l) => l,
            None => {
                *self.logo_asked.borrow_mut() = (String::new(), 0);
                saved.to_string()
            }
        }
    }

    /// Removes the files of the media folder no row names any more. Number of files removed.
    pub fn prune(&self, named: &HashSet<String>) -> usize {
        let Ok(entries) = std::fs::read_dir(&self.media_dir) else { return 0 };
        let mut n = 0;
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if !MEDIA_NAME.is_match(&name) || named.contains(&name) {
                continue;
            }
            match std::fs::remove_file(e.path()) {
                Ok(()) => n += 1,
                Err(err) => warn("media", &err.to_string(), &[("file", name.as_str().into())]),
            }
        }
        n
    }

    /// A SharePoint or OneDrive attachment downloaded with the browser's session, redirects followed with its cookies
    /// (the request context of Playwright); behind the proxy of the tenant, through the proxy first. File name, or None.
    pub async fn download(&self, page: &Page, url: &str, name: &str) -> Option<String> {
        if !is_share_point_url(url) {
            return None;
        }
        let file = download_file(url, name);
        let target = self.files_dir.join(&file);
        if target.exists() {
            return Some(file);
        }
        let _ = std::fs::create_dir_all(&self.files_dir);
        let browser = page.browser()?;
        let mcas = |u: &str| {
            let mut x = url::Url::parse(u).ok()?;
            let host = format!("{}.mcas.ms", x.host_str()?);
            x.set_host(Some(&host)).ok()?;
            Some(x.to_string())
        };
        let sources: Vec<String> =
            if behind_mcas(&page.url()) { [mcas(url), Some(url.to_string())].into_iter().flatten().collect() } else { vec![url.to_string()] };
        for source in sources {
            let mut at = url::Url::parse(&format!("{source}{}download=1", if source.contains('?') { "&" } else { "?" })).ok()?;
            let mut answer = None;
            for _ in 0..=10 {
                let cookies = browser.cookies().await.unwrap_or_default();
                let cookie = crate::cdp::cookie_header(&cookies, &at);
                let headers = if cookie.is_empty() { vec![] } else { vec![("Cookie", cookie)] };
                let r = match send(&self.http, "GET", at.as_str(), headers, vec![], 60_000).await {
                    Ok(r) => r,
                    Err(e) => {
                        warn("download", &crate::agent::text_of(&e), &[]);
                        return None;
                    }
                };
                let _ = browser.set_cookies(crate::cdp::set_cookies(&at, &r)).await;
                match r.header("location").filter(|_| (300..400).contains(&r.status)) {
                    Some(l) => at = at.join(l).ok()?,
                    None => {
                        answer = Some(r);
                        break;
                    }
                }
            }
            let Some(r) = answer else { continue };
            let kind = r.header("content-type").unwrap_or("").to_string();
            if r.status != 200 || kind.starts_with("text/html") {
                warn("download", "refused", &[("status", r.status.into()), ("type", kind.as_str().into()), ("mcas", (source != url).into())]);
                continue;
            }
            if r.body.len() > MAX_DOWNLOAD {
                return None;
            }
            return std::fs::write(&target, &r.body).is_ok().then_some(file);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_of_files() {
        assert_eq!(download_file("https://x.sharepoint.com/a", "Report.PDF"), format!("{}.pdf", sha16("https://x.sharepoint.com/a")));
        assert_eq!(download_file("https://x.sharepoint.com/a", "noext"), sha16("https://x.sharepoint.com/a"));
        assert!(
            is_share_point_url("https://contoso.sharepoint.com/x")
                && !is_share_point_url("http://contoso.sharepoint.com/x")
                && !is_share_point_url("https://evil.test/x")
        );
        assert!(behind_mcas("https://teams.cloud.microsoft.mcas.ms/") && !behind_mcas("https://teams.cloud.microsoft/"));
        assert_eq!(logo_ext(&[0xff, 0xd8, 0xff, 0]), Some("jpg"));
    }
}
