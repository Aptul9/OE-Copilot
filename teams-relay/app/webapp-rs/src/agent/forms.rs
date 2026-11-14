//! The pages of a Microsoft sign-in behind Defender for Cloud Apps, read without running them (scripts/
//! mcas-auth-forms.mjs): each parser accepts only the exact shape observed and fails on anything else.

use regex::Regex;
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use url::Url;

pub type Res<T> = Result<T, String>;

fn ensure(ok: bool, why: &str) -> Res<()> {
    if ok { Ok(()) } else { Err(why.to_string()) }
}

fn re(p: &str) -> Regex {
    Regex::new(p).expect("fixed pattern")
}

/// The named and numeric entities of an attribute value.
fn decode(s: &str) -> Res<String> {
    let mut out = String::new();
    let mut last = 0;
    for c in re(r"(?i)&(?:#x([\da-f]+)|#(\d+)|(amp|quot|apos|lt|gt));").captures_iter(s) {
        let m = c.get(0).expect("match");
        out.push_str(&s[last..m.start()]);
        if let Some(name) = c.get(3) {
            out.push_str(match name.as_str().to_lowercase().as_str() {
                "amp" => "&",
                "quot" => "\"",
                "apos" => "'",
                "lt" => "<",
                _ => ">",
            });
        } else {
            let (digits, radix) = c.get(1).map(|h| (h.as_str(), 16)).unwrap_or_else(|| (c.get(2).map_or("", |d| d.as_str()), 10));
            let cp = u32::from_str_radix(digits, radix).ok().and_then(char::from_u32).ok_or("Invalid code point")?;
            out.push(cp);
        }
        last = m.end();
    }
    out.push_str(&s[last..]);
    Ok(out)
}

/// The quoted attributes of a tag, names in lower case; a later one replaces an earlier one.
fn attributes(tag: &str) -> Res<BTreeMap<String, String>> {
    let mut out = BTreeMap::new();
    for c in re(r#"\b([\w-]+)\s*=\s*(?:"([^"]*)"|'([^']*)')"#).captures_iter(tag) {
        let v = c.get(2).or(c.get(3)).map_or("", |m| m.as_str());
        out.insert(c[1].to_lowercase(), decode(v)?);
    }
    Ok(out)
}

fn hidden_inputs(html: &str) -> Res<Vec<BTreeMap<String, String>>> {
    let mut out = vec![];
    for m in re(r"(?i)<input\b[^>]*>").find_iter(html) {
        let a = attributes(m.as_str())?;
        if a.get("type").is_some_and(|t| t.to_lowercase() == "hidden") {
            out.push(a);
        }
    }
    Ok(out)
}

fn plain_https(u: &Url) -> bool {
    u.scheme() == "https" && u.username().is_empty() && u.password().is_none() && u.port().is_none()
}

fn gateway_action(value: Option<&str>) -> Res<String> {
    let action = Url::parse(value.unwrap_or("undefined")).map_err(|_| "Invalid URL")?;
    let host = action.host_str().unwrap_or("");
    ensure(
        plain_https(&action) && re(r"(?i)^[a-z0-9-]+\.access\.mcas\.ms$").is_match(host) && action.path() == "/aad_login",
        "unexpected MCAS form destination",
    )?;
    Ok(action.to_string())
}

/// The one form of the sign-in that posts the id_token to the gateway of the tenant.
pub fn mcas_form(html: &str) -> Res<(String, String)> {
    let forms: Vec<regex::Captures> = re(r"(?i)<form\b[^>]*>([\s\S]*?)</form>").captures_iter(html).collect();
    ensure(forms.len() == 1, "one MCAS form required")?;
    let whole = forms[0].get(0).expect("match").as_str();
    let form = attributes(&whole[..whole.find('>').map_or(0, |i| i + 1)])?;
    ensure(form.get("method").is_some_and(|m| m.to_lowercase() == "post"), "MCAS form must use POST")?;
    let action = gateway_action(form.get("action").map(String::as_str))?;
    let tokens: Vec<_> = hidden_inputs(&forms[0][1])?.into_iter().filter(|i| i.get("name").map(String::as_str) == Some("id_token")).collect();
    let token = tokens.first().and_then(|t| t.get("value")).cloned().unwrap_or_default();
    ensure(tokens.len() == 1 && !token.is_empty() && token.encode_utf16().count() < 30000, "one MCAS id_token required")?;
    Ok((action, token))
}

/// The monitored-access page of the proxy: where it continues, with its hidden fields.
pub fn mcas_continuation(html: &str) -> Res<(String, Vec<(String, String)>)> {
    let raw = re(r"var context\s*=\s*(\{[\s\S]*?\});").captures(html).map(|c| c[1].to_string()).unwrap_or_default();
    let c: Map<String, Value> = match serde_json::from_str(&raw) {
        Ok(Value::Object(m)) => m,
        _ => return Err("invalid MCAS continuation context".into()),
    };
    let t = |k: &str| c.get(k).is_some_and(crate::js::truthy);
    ensure(
        c.get("TITLE_ID") == Some(&Value::from("PROXYWEB_MONITORED_ACCESS_MESSAGE"))
            && c.get("DESCRIPTION_ID") == Some(&Value::from("PROXYWEB_MONITORED_ACCESS_DESCRIPTION"))
            && c.get("DISPLAY_DISCLAIMER") == Some(&Value::Bool(true))
            && !t("CUSTOMIZED_TITLE")
            && !t("CUSTOMIZED_BODY")
            && !t("WAIT_FOR_APP_ONBOARDING")
            && !t("APP_ONBOARDING_SHOW_CONTINUE")
            && !t("BYPASS_NON_SUFFIX_SUPPORTED_APP_FOR_ADMIN")
            && !t("SECOND_LOGIN_INFORMATION_PAGE_ENABLED"),
        "unsupported MCAS interstitial",
    )?;
    ensure(c.get("REDIRECT_TO_SUFFIX_METHOD") == Some(&Value::from("POST")), "MCAS continuation must use POST")?;
    let hidden = match c.get("ADDITIONAL_HIDDEN_FIELDS") {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(_) => return Err("invalid MCAS continuation fields".into()),
    };
    let inputs = hidden_inputs(&hidden)?;
    let allowed = ["id_token", "appLoginCookies", "contextData"];
    ensure(!inputs.is_empty() && inputs.iter().all(|i| i.get("name").is_some_and(|n| allowed.contains(&n.as_str()))), "unsupported MCAS continuation field")?;
    let mut fields: Vec<(String, String)> = vec![];
    for i in &inputs {
        let (n, v) = (i.get("name").cloned().unwrap_or_default(), i.get("value").cloned().unwrap_or_default());
        match fields.iter_mut().find(|(k, _)| *k == n) {
            Some(f) => f.1 = v,
            None => fields.push((n, v)),
        }
    }
    let token = fields.iter().find(|(k, _)| k == "id_token").map(|(_, v)| v.as_str()).unwrap_or("");
    ensure(!token.is_empty() && fields.len() == inputs.len(), "one MCAS id_token required")?;
    let action = gateway_action(c.get("REDIRECT_TO_SUFFIX_URL").and_then(Value::as_str))?;
    for (k, v) in [("continueToSessionControl", "true"), ("isTopFrame", "true"), ("jsNativeBrokerMainWindowDomain", "")] {
        fields.push((k.into(), v.into()));
    }
    Ok((action, fields))
}

/// The context-restorer page of the proxy CDN: the post back to the gateway.
pub fn mcas_restore(value: &str, gateway: &str) -> Res<(String, Vec<(String, String)>)> {
    let url = Url::parse(value).map_err(|_| "Invalid URL")?;
    ensure(
        plain_https(&url) && url.host_str() == Some("inline.cdn.mcas.ms") && url.path() == "/proxyweb/1.67.98/html/session-context-restore.html",
        "unexpected MCAS context-restorer",
    )?;
    let params: Vec<(String, String)> = form_urlencoded::parse(url.fragment().unwrap_or("").as_bytes()).into_owned().collect();
    let values = |k: &str| params.iter().filter(|(n, _)| n == k).map(|(_, v)| v.clone()).collect::<Vec<_>>();
    ensure(values("action") == ["post"] && values("redirect_uri").len() == 1, "unsupported MCAS context-restorer action")?;
    let target = Url::parse(&values("redirect_uri")[0]).map_err(|_| "Invalid URL")?;
    ensure(plain_https(&target) && target.host_str() == Some(gateway) && target.path() == "/finalize_login", "unexpected MCAS context-restorer target")?;
    Ok((target.to_string(), vec![("contextData".into(), String::new())]))
}

/// The "Object Moved" page of the gateway: the one literal redirect it makes.
pub fn mcas_moved(html: &str, app_origin: &str) -> Res<String> {
    ensure(re(r"(?i)<title>\s*Object Moved\s*</title>").is_match(html), "unexpected MCAS moved response")?;
    let found: Vec<String> = re(r#"window\.location\.replace\(\s*("(?:\\.|[^"\\])*")\s*\)"#).captures_iter(html).map(|c| c[1].to_string()).collect();
    ensure(found.len() == 1, "one literal MCAS redirect required")?;
    let target: String = serde_json::from_str(&found[0]).map_err(|_| "invalid MCAS redirect")?;
    let url = Url::parse(&target).map_err(|_| "Invalid URL")?;
    let outlook = app_origin == "https://outlook.cloud.microsoft";
    let hosts: &[&str] = if outlook { &["outlook.cloud.microsoft.mcas.ms"] } else { &["teams.cloud.microsoft.mcas.ms", "teams.microsoft.com.mcas.ms"] };
    let host = url.host_str().unwrap_or("");
    ensure(plain_https(&url) && hosts.contains(&host) && (url.path() == "/" || (outlook && url.path() == "/mail/")), "unexpected MCAS moved destination")?;
    Ok(url.to_string())
}

/// The code of the callback of the sign-in, in its query or its fragment.
pub fn authorization_code(value: &str, expected_state: &str) -> Res<String> {
    let url = Url::parse(value).map_err(|_| "Invalid URL")?;
    let host = url.host_str().unwrap_or("");
    ensure(
        plain_https(&url)
            && ["teams.cloud.microsoft", "teams.cloud.microsoft.mcas.ms", "teams.microsoft.com.mcas.ms"].contains(&host)
            && url.path() == "/v2/authv2",
        "unexpected authorization callback",
    )?;
    let params: Vec<(String, String)> = if url.query_pairs().any(|(k, _)| k == "code") {
        url.query_pairs().into_owned().collect()
    } else {
        form_urlencoded::parse(url.fragment().unwrap_or("").as_bytes()).into_owned().collect()
    };
    let get = |k: &str| params.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
    ensure(get("state").as_deref() == Some(expected_state), "authorization state mismatch")?;
    get("code").filter(|c| !c.is_empty()).ok_or_else(|| "authorization code missing".to_string())
}

const WORD: fn(char) -> bool = |c| c.is_ascii_alphanumeric() || c == '_' || c == '$' || c == '.';

/// The one literal JSON object assigned to `$Config` or passed to `__JSWRAPPER.initWrapperFixer(`, never executed.
pub fn configuration(source: &str, marker: &str) -> Res<Map<String, Value>> {
    ensure(source.len() <= 2 * 1024 * 1024, "configuration source exceeds two MiB")?;
    let (pattern, assignment) = match marker {
        "$Config" => (r"\$Config\s*=", true),
        "__JSWRAPPER.initWrapperFixer" => (r"__JSWRAPPER\.initWrapperFixer\s*\(", false),
        _ => return Err("unsupported configuration marker".into()),
    };
    // (?<![\w$.]) before the marker, and after "=" no "=" or ">" (a comparison, an arrow)
    let found: Vec<usize> = re(pattern)
        .find_iter(source)
        .filter(|m| !source[..m.start()].chars().next_back().is_some_and(WORD))
        .filter(|m| !assignment || !source[m.end()..].starts_with(['=', '>']))
        .map(|m| m.end())
        .collect();
    ensure(found.len() == 1, "one configuration marker required")?;
    let rest = &source[found[0]..];
    let body = rest.trim_start_matches(|c: char| c.is_whitespace() || c == '\u{feff}');
    ensure(body.starts_with('{'), "configuration must be a JSON object")?;
    let (mut depth, mut quoted, mut escaped) = (0i32, false, false);
    for (i, ch) in body.char_indices() {
        if quoted {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                quoted = false;
            }
        } else if ch == '"' {
            quoted = true;
        } else if ch == '{' {
            depth += 1;
        } else if ch == '}' {
            depth -= 1;
            if depth == 0 {
                let tail = body[i + 1..].trim_start_matches(|c: char| c.is_whitespace() || c == '\u{feff}');
                let closed =
                    if assignment { tail.is_empty() || tail.starts_with(';') || re(r"(?i)^</script\s*>").is_match(tail) } else { tail.starts_with(')') };
                ensure(closed, "configuration must be one JSON object")?;
                return match serde_json::from_str(&body[..=i]) {
                    Ok(Value::Object(m)) => Ok(m),
                    _ => Err("invalid configuration JSON object".into()),
                };
            }
        }
    }
    Err("invalid configuration JSON object".into())
}

/// The bootstrap script of the proxy a Teams page behind MCAS loads, None when it loads none.
pub fn bootstrap_url(html: &str, origin: &str) -> Res<Option<String>> {
    let base = Url::parse(origin).map_err(|_| "Invalid URL")?;
    let mut urls: Vec<String> = vec![];
    let tags = re(r#"(?i)<!--[\s\S]*?-->|(<script\b(?:[^"'<>]|"[^"]*"|'[^']*')*>)(?:[\s\S]*?</script\s*>|$)"#);
    let attr = re(r#"\s+([\w-]+)\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s"'`=<>]+))"#);
    for c in tags.captures_iter(html) {
        let Some(tag) = c.get(1) else { continue };
        let src = match attr.captures_iter(tag.as_str()).find(|a| a[1].to_lowercase() == "src") {
            Some(a) => decode(a.get(2).or(a.get(3)).or(a.get(4)).map_or("", |m| m.as_str()))?,
            None => String::new(),
        };
        if src.is_empty() {
            continue;
        }
        let url = base.join(&src).map_err(|_| "Invalid URL")?;
        let host = url.host_str().unwrap_or("");
        if !url.path().ends_with("/js-bootstrap.js") && host != "mcas-proxyweb.mcas.ms" {
            continue;
        }
        ensure(plain_https(&url) && host == "mcas-proxyweb.mcas.ms" && url.path() == "/js-bootstrap.js", "unexpected MCAS bootstrap destination")?;
        if !urls.contains(&url.to_string()) {
            urls.push(url.to_string());
        }
    }
    ensure(urls.len() <= 1, "one MCAS bootstrap URL required")?;
    Ok(urls.into_iter().next())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const ORIGIN: &str = "https://teams.cloud.microsoft.mcas.ms";
    const BOOTSTRAP: &str = "https://mcas-proxyweb.mcas.ms/js-bootstrap.js";

    fn script(src: &str) -> String {
        format!(r#"<script defer src="{src}"></script>"#)
    }

    #[test]
    fn literal_configuration() {
        let c = configuration(
            "<script>var $Config \n = \t {\"url\":\"https://login.microsoftonline.com\",\"nested\":{\"items\":[1,true,null]}};</script>",
            "$Config",
        )
        .expect("config");
        assert_eq!(Value::Object(c), json!({"url": "https://login.microsoftonline.com", "nested": {"items": [1, true, null]}}));
        assert_eq!(
            Value::Object(configuration("__JSWRAPPER.initWrapperFixer \n ( {\"enabled\":true} );", "__JSWRAPPER.initWrapperFixer").expect("config")),
            json!({"enabled": true})
        );
        let value = json!({"nested": {"text": "escaped quote: \" and backslash: \\ and braces: } {", "items": [{"key": "}"}]}});
        assert_eq!(Value::Object(configuration(&format!("$Config={value};"), "$Config").expect("config")), value);
        for source in [
            "",
            r#"$Config = {"incomplete":true;"#,
            r#"$Config = {"unterminated":"};"#,
            "$Config = {'singleQuoted':true};",
            "$Config = {unquoted:true};",
            r#"$Config = {"trailing":true,};"#,
            r#"$Config = {"value":undefined};"#,
            r#"$Config = {"value":(()=>{throw new Error("injection")})()};"#,
            r#"$Config = JSON.parse("{}");"#,
            r#"$Config = {"value":1} || globalThis.injected();"#,
            r#"$Config = [{"array":true}];"#,
            "$Config = null;",
            "$Config = true;",
            r#"$Config = "text";"#,
            r#"$Config == {"comparison":true};"#,
            r#"$ConfigOther = {"prefix":true};"#,
            "$Config={\"one\":1};\n$Config = {\"two\":2};",
            "$Config=invalid;\n$Config={\"valid\":true};",
        ] {
            assert!(configuration(source, "$Config").is_err(), "{source}");
        }
        for source in [
            r#"__JSWRAPPER.initWrapperFixer({"value":1}, {"extra":true});"#,
            r#"__JSWRAPPER.initWrapperFixer({"value":1} || injected());"#,
            r#"__JSWRAPPER.initWrapperFixer({"value":1};"#,
            "__JSWRAPPER.initWrapperFixer({\"one\":1});\n__JSWRAPPER.initWrapperFixer ({\"two\":2});",
        ] {
            assert!(configuration(source, "__JSWRAPPER.initWrapperFixer").is_err(), "{source}");
        }
        assert!(configuration(r#"other={"value":1};"#, "other").is_err());
        let prefix = "$Config={};";
        assert!(configuration(&format!("{prefix}{}", " ".repeat(2 * 1024 * 1024 - prefix.len())), "$Config").is_ok());
        assert!(configuration(&format!("{prefix}{}", " ".repeat(2 * 1024 * 1024)), "$Config").is_err());
        assert!(configuration(&format!("{prefix}{}", "\u{e9}".repeat(1024 * 1024)), "$Config").is_err());
    }

    #[test]
    fn bootstrap_script_url() {
        let none = |html: &str, origin: &str| bootstrap_url(html, origin).expect("parsed");
        assert_eq!(none("<html></html>", ORIGIN), None);
        assert_eq!(none(r#"<script>const src="https://mcas-proxyweb.mcas.ms/js-bootstrap.js";</script>"#, ORIGIN), None);
        assert_eq!(none(&script("https://other.test/application.js"), ORIGIN), None);
        assert_eq!(none(r#"<script data-src="https://mcas-proxyweb.mcas.ms/js-bootstrap.js"></script>"#, ORIGIN), None);
        assert_eq!(none(&format!("<html>{}{}</html>", script("https://other.test/application.js"), script(BOOTSTRAP)), ORIGIN).as_deref(), Some(BOOTSTRAP));
        assert_eq!(none(&script("/js-bootstrap.js?version=1"), "https://mcas-proxyweb.mcas.ms").as_deref(), Some(&*format!("{BOOTSTRAP}?version=1")));
        assert_eq!(none(&script("//mcas-proxyweb.mcas.ms/js-bootstrap.js"), ORIGIN).as_deref(), Some(BOOTSTRAP));
        assert_eq!(
            none("<SCRIPT SRC='https&#58;//mcas-proxyweb.mcas.ms/js-bootstrap.js?a=1&amp;b=&#x32;'></SCRIPT>", ORIGIN).as_deref(),
            Some(&*format!("{BOOTSTRAP}?a=1&b=2"))
        );
        assert_eq!(none(&format!("<script defer src={BOOTSTRAP}></script>"), ORIGIN).as_deref(), Some(BOOTSTRAP));
        assert_eq!(none(&format!(r#"<script data-note="src='{BOOTSTRAP}'"></script>"#), ORIGIN), None);
        assert_eq!(none(&format!("<!-- {} -->", script(BOOTSTRAP)), ORIGIN), None);
        assert_eq!(none(&format!("<script>const text = {};</script>", json!(script(BOOTSTRAP))), ORIGIN), None);
        assert_eq!(none(&(script(BOOTSTRAP) + &script(&BOOTSTRAP.replace(".ms/", ".ms:443/"))), ORIGIN).as_deref(), Some(BOOTSTRAP));
        for url in [
            "https://other.test/js-bootstrap.js",
            "https://mcas-proxyweb.mcas.ms.other.test/js-bootstrap.js",
            "http://mcas-proxyweb.mcas.ms/js-bootstrap.js",
            "https://user@mcas-proxyweb.mcas.ms/js-bootstrap.js",
            "https://user:password@mcas-proxyweb.mcas.ms/js-bootstrap.js",
            "https://mcas-proxyweb.mcas.ms:444/js-bootstrap.js",
            "https://mcas-proxyweb.mcas.ms/other.js",
            "https://mcas-proxyweb.mcas.ms/nested/js-bootstrap.js",
        ] {
            assert!(bootstrap_url(&script(url), ORIGIN).is_err(), "{url}");
        }
        assert!(bootstrap_url(&(script(&format!("{BOOTSTRAP}?version=1")) + &script(&format!("{BOOTSTRAP}?version=2"))), ORIGIN).is_err());
    }

    #[test]
    fn forms_and_redirects() {
        let html = r#"<form method="post" action="https://tenant.access.mcas.ms/aad_login"><input type="hidden" name="id_token" value="tok&amp;en"></form>"#;
        assert_eq!(mcas_form(html).expect("form"), ("https://tenant.access.mcas.ms/aad_login".into(), "tok&en".into()));
        assert!(
            mcas_form(r#"<form method="post" action="https://login.microsoftonline.com/common/login"><input type="password" name="passwd"></form>"#).is_err()
        );
        assert!(mcas_form(r#"<form method="post" action="https://tenant.access.mcas.ms/aad_login"><input name="id_token" value="private"></form>"#).is_err());
        let moved = r#"<title>Object Moved</title><script>window.location.replace("https://teams.microsoft.com.mcas.ms/")</script>"#;
        assert_eq!(mcas_moved(moved, "").expect("moved"), "https://teams.microsoft.com.mcas.ms/");
        assert!(mcas_moved(&moved.replace("teams.microsoft.com.mcas.ms", "evil.test"), "").is_err());
        let restore = "https://inline.cdn.mcas.ms/proxyweb/1.67.98/html/session-context-restore.html#action=post&redirect_uri=https%3A%2F%2Ftenant.access.mcas.ms%2Ffinalize_login";
        assert_eq!(mcas_restore(restore, "tenant.access.mcas.ms").expect("restore").0, "https://tenant.access.mcas.ms/finalize_login");
        assert!(mcas_restore(restore, "other.access.mcas.ms").is_err());
        let context = json!({
            "TITLE_ID": "PROXYWEB_MONITORED_ACCESS_MESSAGE", "DESCRIPTION_ID": "PROXYWEB_MONITORED_ACCESS_DESCRIPTION", "DISPLAY_DISCLAIMER": true,
            "REDIRECT_TO_SUFFIX_METHOD": "POST", "REDIRECT_TO_SUFFIX_URL": "https://tenant.access.mcas.ms/aad_login",
            "ADDITIONAL_HIDDEN_FIELDS": r#"<input type="hidden" name="id_token" value="t"><input type="hidden" name="contextData" value="c">"#,
        });
        let (action, fields) = mcas_continuation(&format!("<script>var context = {context};</script>")).expect("continuation");
        assert_eq!(action, "https://tenant.access.mcas.ms/aad_login");
        assert_eq!(
            fields.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(),
            ["id_token", "contextData", "continueToSessionControl", "isTopFrame", "jsNativeBrokerMainWindowDomain"]
        );
        assert_eq!(authorization_code("https://teams.microsoft.com.mcas.ms/v2/authv2#code=c&state=s", "s").expect("code"), "c");
        assert!(authorization_code("https://teams.microsoft.com/v2/authv2#code=c&state=s", "s").is_err());
        assert!(authorization_code("https://teams.cloud.microsoft/v2/authv2?code=c&state=x", "s").is_err());
    }
}
