//! Images the app sends, into data/N/uploads where the agent reads them (src/lib/uploads.ts), and the files the agent
//! wrote, served (src/lib/files.ts).

use crate::web::{Res, Resp, bytes, err};
use std::path::{Path, PathBuf};

pub const MAX_UPLOAD: usize = 10_000_000;
const KEEP_MS: u128 = 86_400_000;

pub fn uploads_dir(slot: i64) -> PathBuf {
    crate::slotdb::slot_dir(slot).join("uploads")
}

/// The type of an image from its first bytes.
pub fn image_ext(head: &[u8]) -> Option<&'static str> {
    let at = |i: usize, b: &[u8]| head.get(i..i + b.len()) == Some(b);
    if at(0, &[0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]) {
        return Some("png");
    }
    if at(0, &[0xff, 0xd8, 0xff]) {
        return Some("jpg");
    }
    if at(0, &[0x47, 0x49, 0x46, 0x38]) {
        return Some("gif");
    }
    if at(0, &[0x52, 0x49, 0x46, 0x46]) && at(8, &[0x57, 0x45, 0x42, 0x50]) {
        return Some("webp");
    }
    None
}

pub fn image_type(ext: &str) -> &'static str {
    match ext {
        "png" => "image/png",
        "jpg" => "image/jpeg",
        "gif" => "image/gif",
        _ => "image/webp",
    }
}

/// An image, written to data/N/uploads; uploads older than a day go. Returns the file name.
pub fn save_image(slot: i64, data: &[u8]) -> Res<String> {
    if data.is_empty() {
        return Err(err(400, "Missing image"));
    }
    if data.len() > MAX_UPLOAD {
        return Err(err(413, "Image larger than 10 MB"));
    }
    let Some(ext) = image_ext(data) else {
        return Err(err(415, "Only PNG, JPEG, GIF or WebP images can be sent from here: send other files from Teams"));
    };
    let dir = uploads_dir(slot);
    std::fs::create_dir_all(&dir)?;
    let now = std::time::SystemTime::now();
    for e in std::fs::read_dir(&dir)?.flatten() {
        let old = e.metadata().ok().and_then(|m| m.modified().ok()).and_then(|t| now.duration_since(t).ok()).is_some_and(|d| d.as_millis() > KEEP_MS);
        if old {
            let _ = std::fs::remove_file(e.path());
        }
    }
    let name = format!("{}.{ext}", crate::auth::crypto::hex(&crate::auth::crypto::random(8)));
    std::fs::write(dir.join(&name), data)?;
    Ok(name)
}

/// A file the agent wrote, with its headers; 404 when it is not there.
pub fn file_response(file: &Path, headers: &[(&str, String)]) -> Res<Resp> {
    let Ok(data) = std::fs::read(file) else { return Err(err(404, "Not found")) };
    let mut h: Vec<(&str, String)> = vec![("content-length", data.len().to_string()), ("x-content-type-options", "nosniff".into())];
    h.extend(headers.iter().cloned());
    Ok(bytes(200, &h, data))
}

pub fn attachment_name(name: Option<&str>) -> String {
    let re = regex::Regex::new(r#"[\\/:*?"<>|\r\n]+"#).expect("fixed");
    let clean = re.replace_all(name.unwrap_or(""), "_").into_owned();
    let units: Vec<u16> = clean.encode_utf16().take(150).collect();
    let cut = String::from_utf16_lossy(&units);
    if cut.is_empty() { "file".into() } else { cut }
}

/// Every UTF-16 unit outside printable ASCII, and the quote, as "_".
pub fn content_disposition(name: &str) -> String {
    let ascii: String = name.encode_utf16().map(|u| if (0x20..=0x7e).contains(&u) && u != u16::from(b'"') { char::from(u as u8) } else { '_' }).collect();
    format!("attachment; filename=\"{ascii}\"; filename*=UTF-8''{}", crate::js::encode_uri_component(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(attachment_name(Some("a/b:c?.pdf")), "a_b_c_.pdf");
        assert_eq!(attachment_name(None), "file");
        assert_eq!(content_disposition("café \"x\".pdf"), "attachment; filename=\"caf_ _x_.pdf\"; filename*=UTF-8''caf%C3%A9%20%22x%22.pdf");
        assert_eq!(image_ext(b"GIF89a"), Some("gif"));
    }
}
