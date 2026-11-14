//! Stats of the status challenge (src/lib/status-stats.ts, app/status-challenge-spec.md, Stats): the API computes them,
//! the page sums them. Times are Unix seconds, days YYYY-MM-DD in the zone of the challenge.

use jiff::tz::TimeZone;
use jiff::{Timestamp, ToSpan, civil};
use serde_json::{Map, Value, json};

/// The statuses of the challenge (src/shared/status.ts); a stretch may also be "unknown".
pub const STATUSES: [&str; 6] = ["available", "busy", "dnd", "brb", "away", "offline"];

const ONLINE: [&str; 5] = ["available", "busy", "dnd", "brb", "away"];

/// Seconds after the last poll that saw a status within which the same status extends its stretch.
pub const STATUS_GAP: i64 = 180;

/// since, until, status
pub type Stretch = (i64, i64, String);

/// The zone of a name; a name the time zone database does not know is UTC.
pub fn zone(name: &str) -> TimeZone {
    TimeZone::get(name).unwrap_or(TimeZone::UTC)
}

fn zoned(t: i64, tz: &TimeZone) -> jiff::Zoned {
    Timestamp::from_second(t).unwrap_or(Timestamp::UNIX_EPOCH).to_zoned(tz.clone())
}

pub fn day_of(t: i64, tz: &TimeZone) -> String {
    let z = zoned(t, tz);
    format!("{}-{:02}-{:02}", z.year(), z.month(), z.day())
}

pub fn clock(t: i64, tz: &TimeZone) -> String {
    let z = zoned(t, tz);
    format!("{:02}:{:02}", z.hour(), z.minute())
}

/// Seconds the zone is ahead of UTC at t.
fn offset(t: i64, tz: &TimeZone) -> i64 {
    i64::from(tz.to_offset(Timestamp::from_second(t).unwrap_or(Timestamp::UNIX_EPOCH)).seconds())
}

/// Date.UTC(y, m - 1, d) of a YYYY-MM-DD, in days: a month or day out of range carries over, as in JavaScript.
fn civil_of(day: &str, extra_days: i64) -> Option<civil::Date> {
    let mut p = day.split('-').map(|x| x.parse::<i64>().ok());
    let (y, m, d) = (p.next()??, p.next()??, p.next()??);
    let first = civil::Date::new(i16::try_from(y).ok()?, 1, 1).ok()?;
    first.checked_add((m - 1).months()).ok()?.checked_add((d - 1 + extra_days).days()).ok()
}

fn utc_seconds(d: civil::Date) -> i64 {
    d.to_zoned(TimeZone::UTC).map(|z| z.timestamp().as_second()).unwrap_or(0)
}

/// Unix s of 00:00 of the day in the zone.
pub fn day_start(day: &str, tz: &TimeZone) -> i64 {
    let Some(d) = civil_of(day, 0) else { return 0 };
    let local = utc_seconds(d);
    let t = local - offset(local, tz);
    local - offset(t, tz)
}

pub fn next_day(day: &str) -> String {
    civil_of(day, 1).map(|d| d.to_string()).unwrap_or_default()
}

/// The start of each hour of the day and the end of the day: 25 values, 24 or 26 on a DST change.
pub fn hour_bounds(day: &str, tz: &TimeZone) -> Vec<i64> {
    let start = day_start(day, tz);
    let end = day_start(&next_day(day), tz);
    let mut out: Vec<i64> = (0..).map(|i| start + i * 3600).take_while(|t| *t < end).collect();
    out.push(end);
    out
}

/// A stretch runs until the next one when that came at most `gap` after its last poll, else until its last poll, the
/// rest being no data; the newest one runs until now while it is fresh. Stretches of one status that touch are joined.
pub fn join_stretches(stretches: &[Stretch], gap: i64, now: i64) -> Vec<Stretch> {
    let mut s = stretches.to_vec();
    s.sort_by_key(|x| x.0);
    let mut out: Vec<Stretch> = vec![];
    for (i, (since, until, status)) in s.iter().enumerate() {
        let mut end = *until;
        match s.get(i + 1) {
            Some(next) if next.0 - until <= gap => end = end.max(next.0),
            None if now - until <= gap => end = end.max(now),
            _ => {}
        }
        match out.last_mut() {
            Some(last) if last.2 == *status && last.1 >= *since => last.1 = last.1.max(end),
            _ => out.push((*since, end, status.clone())),
        }
    }
    out
}

/// Math.round(n * 100) / 100
fn round2(n: f64) -> f64 {
    (n * 100.0 + 0.5).floor() / 100.0
}

fn known(s: &str) -> Option<usize> {
    STATUSES.iter().position(|k| *k == s)
}

/// The stats of a day, keys in the order of DayStats; `with_stretches`: the timeline draws them (today, yesterday).
pub fn day_stats(stretches: &[Stretch], day: &str, tz: &TimeZone, now: i64, gap: i64, with_stretches: bool) -> Value {
    let bounds = hour_bounds(day, tz);
    let start = bounds[0];
    let end = *bounds.last().expect("bounds");
    let limit = start.max(end.min(now));
    let segs: Vec<Stretch> =
        join_stretches(stretches, gap, now).into_iter().map(|(a, b, s)| (a.max(start), b.min(limit), s)).filter(|(a, b, _)| b > a).collect();

    let mut seconds = [0i64; 6];
    for (a, b, s) in &segs {
        if let Some(i) = known(s) {
            seconds[i] += b - a;
        }
    }
    let nodata = limit - start - seconds.iter().sum::<i64>();
    let mut hours = Map::new();
    for (k, v) in STATUSES.iter().zip(seconds) {
        hours.insert((*k).into(), crate::web::num(round2(v as f64 / 3600.0)));
    }
    hours.insert("nodata".into(), crate::web::num(round2(nodata as f64 / 3600.0)));

    let online: Vec<&Stretch> = segs.iter().filter(|(_, _, s)| ONLINE.contains(&s.as_str())).collect();
    let clock_of = |t: i64| if t >= end { "24:00".to_string() } else { clock(t, tz) };
    let online_seconds: i64 = seconds[..5].iter().sum();

    // the status that held most of each hour; no data (gaps, unknown, the hours still to come today) when that is most
    let by_hour: Vec<&str> = bounds
        .windows(2)
        .map(|w| {
            let (h0, h1) = (w[0], w[1]);
            let mut cover = [0i64; 6];
            let mut nodata = h1 - h0;
            for (a, b, s) in &segs {
                let x = b.min(&h1) - a.max(&h0);
                let Some(i) = known(s) else { continue };
                if x <= 0 {
                    continue;
                }
                cover[i] += x;
                nodata -= x;
            }
            let mut best = ("nodata", nodata);
            for (k, c) in STATUSES.iter().zip(cover) {
                if c > best.1 {
                    best = (k, c);
                }
            }
            best.0
        })
        .collect();

    let longest = segs.iter().filter(|(_, _, s)| s == "available").map(|(a, b, _)| b - a).max().unwrap_or(0).max(0);
    let mut m = Map::new();
    m.insert("day".into(), json!(day));
    m.insert("hours".into(), Value::Object(hours));
    m.insert("firstOnline".into(), json!(online.first().map_or(String::new(), |s| clock_of(s.0))));
    m.insert("lastOnline".into(), json!(online.last().map_or(String::new(), |s| clock_of(s.1))));
    m.insert("longestAvailable".into(), json!((longest as f64 / 60.0 + 0.5).floor() as i64));
    m.insert("green".into(), if online_seconds > 0 { crate::web::num(round2(seconds[0] as f64 / online_seconds as f64)) } else { Value::Null });
    m.insert("byHour".into(), json!(by_hour));
    if with_stretches {
        m.insert("stretches".into(), Value::Array(segs.iter().map(|(a, b, s)| json!([a, b, s])).collect()));
    }
    Value::Object(m)
}
