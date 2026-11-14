//! Calendar reads over the same token and transport as mail. No writes, no RSVP.

use crate::js::encode_uri_component as q;
use crate::outlook::Outlook;
use crate::{Error, Result};
use regex::Regex;
use serde_json::Value;
use std::sync::LazyLock;

pub const MAX_RANGE_DAYS: i64 = 366;
pub const MAX_EVENTS: i64 = 1000;
const EVENT_FIELDS: &str = "Id,Subject,Start,End,IsAllDay,ShowAs,Location,Organizer,IsCancelled,Type,SeriesMasterId,OnlineMeetingUrl,WebLink";

pub async fn list_calendars(o: &Outlook, account: &str) -> Result<Vec<Value>> {
    let res = o.get(account, "me/calendars?$top=100&$select=Id,Name,Color,IsDefaultCalendar,CanEdit,Owner").await?;
    Ok(res.get("value").and_then(Value::as_array).cloned().unwrap_or_default())
}

/// An instant with an explicit zone (Z or +hh:mm), in milliseconds; naive times are refused, the mailbox zone would be
/// guessed.
fn instant(s: &str, name: &str) -> Result<i64> {
    static ZONE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)(Z|[+-]\d{2}:?\d{2})$").expect("fixed"));
    let t = s.trim();
    if !ZONE.is_match(t) {
        return Err(Error::provider(format!("{name} needs a time zone (Z or +hh:mm): {s}")));
    }
    let ts: jiff::Timestamp = t.parse().map_err(|_| Error::provider(format!("{name} is not an ISO date-time: {s}")))?;
    Ok(ts.as_millisecond())
}

/// `new Date(ms).toISOString()`.
fn iso(ms: i64) -> String {
    let ts = jiff::Timestamp::from_millisecond(ms).unwrap_or_default();
    format!("{}.{:03}Z", ts.strftime("%Y-%m-%dT%H:%M:%S"), ms.rem_euclid(1000))
}

pub async fn list_calendar_events(
    o: &Outlook,
    account: &str,
    start: &str,
    end: &str,
    calendar_id: Option<&str>,
    limit: Option<i64>,
) -> Result<(Vec<Value>, bool)> {
    let a = instant(start, "start")?;
    let b = instant(end, "end")?;
    if b <= a {
        return Err(Error::provider("end must be after start"));
    }
    if b - a > MAX_RANGE_DAYS * 86_400_000 {
        return Err(Error::provider(format!("Calendar range exceeds {MAX_RANGE_DAYS} days")));
    }
    let limit = limit.filter(|l| *l != 0).unwrap_or(100).clamp(1, MAX_EVENTS);
    let base = match calendar_id {
        Some(c) => format!("me/calendars/{}/calendarview", q(c)),
        None => "me/calendarview".into(),
    };
    let top = limit.to_string();
    let qs = crate::js::search_params(&[
        ("startDateTime", &iso(a)),
        ("endDateTime", &iso(b)),
        ("$top", &top),
        ("$select", EVENT_FIELDS),
        ("$orderby", "Start/DateTime"),
    ]);
    let res = o.get(account, &format!("{base}?{qs}")).await?;
    let events = res.get("value").and_then(Value::as_array).cloned().unwrap_or_default();
    Ok((events, crate::js::truthy_opt(res.get("@odata.nextLink"))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instants() {
        assert_eq!(iso(instant("2026-11-14T10:00:00+01:00", "start").ok().unwrap()), "2026-11-14T09:00:00.000Z");
        assert_eq!(iso(instant("2026-11-14T10:00:00.25Z", "start").ok().unwrap()), "2026-11-14T10:00:00.250Z");
        assert!(instant("2026-11-14T10:00:00", "start").is_err());
        assert!(instant("tomorrow Z", "start").is_err());
    }
}
