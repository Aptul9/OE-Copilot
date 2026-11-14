//! The images of the messages of a light account (the part of src/agent/media.ts it uses): data/N/media, a file written
//! once and kept while a row names it. An image of AMS is fetched with the chatsvc token.

use super::http::{Http, send};
use super::{text_of, warn};
use sha1::Digest;
use std::cell::RefCell;
use std::collections::HashSet;
use std::path::PathBuf;

/// Maximum size of an image fetched from a message.
const MAX_IMAGE: usize = 8_000_000;

/// The file extensions of the images, by content type, in the order the folder is looked in.
const MEDIA_EXT: [(&str, &str); 4] = [("image/png", "png"), ("image/jpeg", "jpg"), ("image/gif", "gif"), ("image/webp", "webp")];

/// sha1 hex, cut to 16: the names of the files of the media folder.
pub fn sha16(s: &str) -> String {
    sha1::Sha1::digest(s.as_bytes()).iter().map(|b| format!("{b:02x}")).collect::<String>()[..16].to_string()
}

/// Image number `index` of message `mid`, without the extension.
pub fn image_key(chat: &str, mid: &str, index: usize) -> String {
    sha16(&format!("{chat}|{mid}|{index}"))
}

/// The 1x1 GIF Teams draws while it loads an image.
pub fn is_placeholder_image(d: &[u8]) -> bool {
    d.len() >= 10 && &d[..4] == b"GIF8" && u16::from_le_bytes([d[6], d[7]]) <= 1 && u16::from_le_bytes([d[8], d[9]]) <= 1
}

pub struct Media {
    dir: PathBuf,
    http: Http,
    /// addresses that could not be fetched, per image: no new attempt
    failed: RefCell<HashSet<String>>,
}

impl Media {
    pub fn new(dir: PathBuf, http: Http) -> Self {
        Media { dir, http, failed: RefCell::new(HashSet::new()) }
    }

    /// File of the image `key` of a message, already saved or fetched now from AMS with the chatsvc token; None when it
    /// cannot be read.
    pub async fn image(&self, key: &str, src: &str, token: &str) -> Option<String> {
        for (_, ext) in MEDIA_EXT {
            let file = self.dir.join(format!("{key}.{ext}"));
            let Ok(st) = std::fs::metadata(&file) else { continue };
            if st.len() > 1000 || !std::fs::read(&file).is_ok_and(|d| is_placeholder_image(&d)) {
                return Some(format!("{key}.{ext}"));
            }
            let _ = std::fs::remove_file(&file);
            super::info("media", "placeholder removed", &[("file", format!("{key}.{ext}").into())]);
        }
        let attempt = format!("{key} {src}");
        if src.is_empty() || self.failed.borrow().contains(&attempt) {
            return None;
        }
        let ams = regex::Regex::new(r"^https://[a-z0-9-]+\.asm\.skype\.com/").expect("fixed");
        let mut got = None;
        if !token.is_empty() && ams.is_match(src) {
            match send(&self.http, "GET", src, vec![("Authorization", format!("Bearer {token}"))], vec![], 30_000).await {
                Ok(r) if r.ok() && r.body.len() <= MAX_IMAGE => {
                    let kind = r.header("content-type").unwrap_or("").split(';').next().unwrap_or("").trim().to_string();
                    got = Some((kind, r.body));
                }
                Ok(_) => {}
                Err(e) => warn("media", &text_of(&e), &[("src", super::cut(src, 60).into())]),
            }
        }
        let ext = got.as_ref().and_then(|(kind, _)| MEDIA_EXT.iter().find(|(t, _)| t == kind).map(|(_, e)| *e));
        let (Some((_, data)), Some(ext)) = (got, ext) else {
            self.failed.borrow_mut().insert(attempt);
            return None;
        };
        // still loading in Teams: the next read gets the image
        if is_placeholder_image(&data) {
            return None;
        }
        let _ = std::fs::create_dir_all(&self.dir);
        std::fs::write(self.dir.join(format!("{key}.{ext}")), &data).ok()?;
        Some(format!("{key}.{ext}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_placeholders() {
        assert_eq!(sha16("a"), "86f7e437faa5a7fc");
        assert!(is_placeholder_image(b"GIF89a\x01\x00\x01\x00rest"));
        assert!(!is_placeholder_image(b"GIF89a\x10\x00\x01\x00rest"));
    }
}
