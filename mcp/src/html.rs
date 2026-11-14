//! HTML mail bodies to Markdown and back.

use regex::{Captures, Regex};
use std::sync::LazyLock;

macro_rules! re {
    ($name:ident, $pat:expr) => {
        static $name: LazyLock<Regex> = LazyLock::new(|| Regex::new($pat).expect("fixed"));
    };
}

pub fn html_unescape(s: &str) -> String {
    re!(ENTITY, r"(?i)&(#x[0-9a-f]+|#\d+|[a-z]+);");
    ENTITY
        .replace_all(s, |c: &Captures| {
            let e = &c[1];
            if let Some(num) = e.strip_prefix('#') {
                let n = if let Some(h) = num.strip_prefix(['x', 'X']) { u32::from_str_radix(h, 16).ok() } else { num.parse::<u32>().ok() };
                return match n.filter(|n| *n <= 0x10ffff) {
                    Some(n) => char::from_u32(n).unwrap_or('\u{fffd}').to_string(),
                    None => c[0].to_string(),
                };
            }
            match e.to_lowercase().as_str() {
                "amp" => "&".into(),
                "lt" => "<".into(),
                "gt" => ">".into(),
                "quot" => "\"".into(),
                "apos" => "'".into(),
                "nbsp" => "\u{a0}".into(),
                _ => c[0].to_string(),
            }
        })
        .into_owned()
}

pub fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&#x27;")
}

fn strip_tags(s: &str) -> String {
    re!(TAG, r"<[^>]+>");
    TAG.replace_all(s, "").into_owned()
}

/// Defender SafeLinks wrap every URL: the target is in the url parameter.
pub fn unwrap_safelink(url: &str) -> String {
    if url.is_empty() {
        return url.into();
    }
    if let Ok(u) = url::Url::parse(url)
        && u.host_str().unwrap_or("").to_lowercase().contains("safelinks.protection.outlook.com")
        && let Some((_, v)) = u.query_pairs().find(|(k, _)| k == "url")
        && !v.is_empty()
    {
        return v.into_owned();
    }
    url.into()
}

fn table(inner: &str) -> String {
    re!(TR, r"(?i)<tr[^>]*>([\s\S]*?)</tr>");
    re!(CELL, r"(?i)<(?:td|th)[^>]*>([\s\S]*?)</(?:td|th)>");
    re!(BR, r"(?i)<br\s*/?>");
    re!(ANYTAG, r"<[^>]+>");
    re!(WS, r"\s+");
    let mut rows: Vec<String> = vec![];
    for r in TR.captures_iter(inner) {
        let cells: Vec<String> = CELL
            .captures_iter(&r[1])
            .map(|c| {
                let t = strip_tags(&BR.replace_all(&c[1], " "));
                let t = html_unescape(&ANYTAG.replace_all(&t, " ")).replace('\u{a0}', " ");
                WS.replace_all(&t, " ").trim().replace('|', "\\|")
            })
            .collect();
        if !cells.iter().any(|c| !c.is_empty()) {
            continue;
        }
        rows.push(format!("| {} |", cells.join(" | ")));
        if rows.len() == 1 {
            rows.push(format!("| {} |", cells.iter().map(|_| "---").collect::<Vec<_>>().join(" | ")));
        }
    }
    if rows.is_empty() { String::new() } else { format!("\n\n{}\n\n", rows.join("\n")) }
}

pub fn html_to_markdown(raw: &str) -> String {
    if raw.is_empty() {
        return String::new();
    }
    static DROP: LazyLock<Vec<Regex>> = LazyLock::new(|| {
        ["style", "script", "head", "title", "xml"].iter().map(|t| Regex::new(&format!(r"(?i)<{t}[^>]*>[\s\S]*?</{t}>")).expect("fixed")).collect()
    });
    static HEADINGS: LazyLock<Vec<Regex>> =
        LazyLock::new(|| (1..=6).map(|l| Regex::new(&format!(r"(?i)<h{l}[^>]*>([\s\S]*?)</h{l}>")).expect("fixed")).collect());
    static BOLD: LazyLock<Vec<Regex>> =
        LazyLock::new(|| ["b", "strong"].iter().map(|t| Regex::new(&format!(r"(?i)<{t}(?:\s[^>]*)?>([\s\S]*?)</{t}>")).expect("fixed")).collect());
    static ITALIC: LazyLock<Vec<Regex>> =
        LazyLock::new(|| ["i", "em"].iter().map(|t| Regex::new(&format!(r"(?i)<{t}(?:\s[^>]*)?>([\s\S]*?)</{t}>")).expect("fixed")).collect());
    re!(IFS, r"(?i)<!\[if[^\]]*\]>[\s\S]*?<!\[endif\]>");
    re!(COMMENT, r"<!--[\s\S]*?-->");
    re!(PRE, r"(?i)<pre[^>]*>([\s\S]*?)</pre>");
    re!(CODE, r"(?i)<code[^>]*>([\s\S]*?)</code>");
    re!(LINK, r#"(?i)<a\s+[^>]*href=["']([^"']+)["'][^>]*>([\s\S]*?)</a>"#);
    re!(TABLE, r"(?i)<table[^>]*>([\s\S]*?)</table>");
    re!(QUOTE, r"(?i)<blockquote[^>]*>([\s\S]*?)</blockquote>");
    re!(LI_OPEN, r"(?i)<li[^>]*>");
    re!(LI_CLOSE, r"(?i)</li>");
    re!(BR, r"(?i)<br\s*/?>");
    re!(BLOCK_END, r"(?i)</(p|div|tr|ul|ol|table)>");
    re!(HR, r"(?i)<hr\s*[^>]*/?>");
    re!(SPACES, r"[ \t]+");
    re!(AROUND_NL, r" *\n *");
    re!(MANY_NL, r"\n{3,}");

    let mut s = raw.to_string();
    for r in DROP.iter() {
        s = r.replace_all(&s, "").into_owned();
    }
    s = IFS.replace_all(&s, "").into_owned();
    s = COMMENT.replace_all(&s, "").into_owned();
    for (i, r) in HEADINGS.iter().enumerate() {
        s = r.replace_all(&s, |c: &Captures| format!("\n\n{} {}\n\n", "#".repeat(i + 1), strip_tags(&c[1]).trim())).into_owned();
    }
    s = PRE.replace_all(&s, |c: &Captures| format!("\n```\n{}\n```\n", html_unescape(strip_tags(&c[1]).trim()))).into_owned();
    s = CODE.replace_all(&s, |c: &Captures| format!("`{}`", html_unescape(strip_tags(&c[1]).trim()))).into_owned();
    for r in BOLD.iter() {
        s = r.replace_all(&s, |c: &Captures| if c[1].trim().is_empty() { String::new() } else { format!("**{}**", c[1].trim()) }).into_owned();
    }
    for r in ITALIC.iter() {
        s = r.replace_all(&s, |c: &Captures| if c[1].trim().is_empty() { String::new() } else { format!("*{}*", c[1].trim()) }).into_owned();
    }
    s = LINK
        .replace_all(&s, |c: &Captures| {
            let url = unwrap_safelink(&html_unescape(c[1].trim()));
            let text = unwrap_safelink(&html_unescape(strip_tags(&c[2]).trim()));
            if text.is_empty() || text == url { url } else { format!("[{text}]({url})") }
        })
        .into_owned();
    s = TABLE.replace_all(&s, |c: &Captures| table(&c[1])).into_owned();
    s = QUOTE
        .replace_all(&s, |c: &Captures| {
            let inner = html_to_markdown(&c[1]);
            format!("\n{}\n", inner.split('\n').filter(|l| !l.trim().is_empty()).map(|l| format!("> {l}")).collect::<Vec<_>>().join("\n"))
        })
        .into_owned();
    s = LI_OPEN.replace_all(&s, "\n* ").into_owned();
    s = LI_CLOSE.replace_all(&s, "").into_owned();
    s = BR.replace_all(&s, "\n").into_owned();
    s = BLOCK_END.replace_all(&s, "\n\n").into_owned();
    s = HR.replace_all(&s, "\n---\n").into_owned();
    s = html_unescape(&strip_tags(&s)).replace('\u{a0}', " ").replace('\u{200b}', "").replace("\r\n", "\n");
    s = SPACES.replace_all(&s, " ").into_owned();
    s = AROUND_NL.replace_all(&s, "\n").into_owned();
    s = MANY_NL.replace_all(&s, "\n\n").into_owned();
    s.trim().to_string()
}

pub fn looks_like_html(text: &str) -> bool {
    re!(HTML_TAG, r"(?i)<(?:p|div|span|b|strong|i|em|u|mark|br|hr|ul|ol|li|table|thead|tbody|tr|th|td|h[1-6]|a|blockquote|pre|code|font|img)\b");
    !text.is_empty() && HTML_TAG.is_match(text)
}

/// Body for an HTML reply or forward comment: HTML kept, plain text escaped with line breaks kept.
pub fn format_body_for_html(body: &str) -> String {
    if body.is_empty() {
        return String::new();
    }
    if looks_like_html(body) {
        return body.into();
    }
    let escaped = html_escape(body).replace("\r\n", "\n").replace('\n', "<br>\n");
    format!("<div style=\"font-family:Aptos,Calibri,sans-serif;font-size:11pt;\">{escaped}</div>")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown() {
        let html = "<html><head><style>x{}</style></head><body><h2>Title</h2><p>Hi <b>there</b> &amp; <a href=\"https://eur01.safelinks.protection.outlook.com/?url=https%3A%2F%2Fx.it%2F&data=1\">link</a></p>\
<table><tr><th>A</th><th>B|C</th></tr><tr><td>1</td><td>2</td></tr></table><blockquote><p>quoted</p><p>two</p></blockquote><ul><li>one</li><li>two</li></ul></body></html>";
        assert_eq!(
            html_to_markdown(html),
            "## Title\n\nHi **there** & [link](https://x.it/)\n\n| A | B\\|C |\n| --- | --- |\n| 1 | 2 |\n\n> quoted\n> two\n\n* one\n* two"
        );
        assert_eq!(html_unescape("&#x41;&#66;&nbsp;&bogus;"), "AB\u{a0}&bogus;");
    }

    #[test]
    fn detection_and_bodies() {
        assert!(looks_like_html("<p>x</p>"));
        assert!(!looks_like_html("a < b and c > d"));
        assert!(!looks_like_html("< b>"));
        assert_eq!(format_body_for_html("a <x>\nb"), "<div style=\"font-family:Aptos,Calibri,sans-serif;font-size:11pt;\">a &lt;x&gt;<br>\nb</div>");
        assert_eq!(format_body_for_html("<b>x</b>"), "<b>x</b>");
    }
}
