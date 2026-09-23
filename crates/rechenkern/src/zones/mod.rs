//! Time zones by place name, abbreviation or UTC offset.

use std::collections::HashMap;
use std::sync::LazyLock;

use jiff::Zoned;
use jiff::tz::{Offset, TimeZone};

const PLACES: &str = include_str!("places.txt");

/// Zone ids whose last segment is not a useful place name.
const SKIPPED_PREFIXES: &[&str] = &[
    "Etc/",
    "US/",
    "Canada/",
    "Mexico/",
    "Brazil/",
    "Chile/",
    "Antarctica/",
    "America/Indiana/",
    "America/Kentucky/",
    "America/North_Dakota/",
    "posix/",
    "right/",
];

/// Lowercase place name -> IANA zone id.
static INDEX: LazyLock<HashMap<String, String>> = LazyLock::new(|| {
    let mut index = HashMap::new();
    for line in PLACES.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
        if let Some((name, zone)) = line.split_once('=') {
            index.insert(name.trim().to_string(), zone.trim().to_string());
        }
    }
    // Cities from the zone database itself: "Asia/Almaty" -> "almaty".
    for id in jiff::tz::db().available() {
        let id = id.as_str();
        if !id.contains('/') || SKIPPED_PREFIXES.iter().any(|p| id.starts_with(p)) {
            continue;
        }
        let city = id.rsplit('/').next().unwrap_or(id).replace('_', " ").to_lowercase();
        index.entry(city).or_insert_with(|| id.to_string());
    }
    index
});

/// Longest place name in words.
pub const MAX_WORDS: usize = 5;

/// Finds a zone by lowercase place name or abbreviation, like "new york" or "pst".
pub fn find(name: &str) -> Option<TimeZone> {
    if let Some(id) = INDEX.get(name) {
        return TimeZone::get(id).ok();
    }
    // Full ids like "europe/berlin" typed by the user.
    if name.contains('/') {
        return TimeZone::get(name).ok();
    }
    None
}

/// A zone with a fixed UTC offset in seconds.
pub fn fixed(seconds: i32) -> Option<TimeZone> {
    Offset::from_seconds(seconds).ok().map(Offset::to_time_zone)
}

/// A short label for the zone at that moment: "JST", "CEST" or "UTC+5".
pub fn label(time: &Zoned) -> String {
    let abbr = time.strftime("%Z").to_string();
    if !abbr.starts_with(['+', '-']) && !abbr.is_empty() {
        return abbr;
    }
    offset_label(time.offset())
}

/// "UTC", "UTC+5", "UTC-3:30".
pub fn offset_label(offset: Offset) -> String {
    let secs = offset.seconds();
    if secs == 0 {
        return "UTC".into();
    }
    let sign = if secs < 0 { '-' } else { '+' };
    let (h, m) = (secs.abs() / 3600, secs.abs() % 3600 / 60);
    if m == 0 { format!("UTC{sign}{h}") } else { format!("UTC{sign}{h}:{m:02}") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_places() {
        assert_eq!(find("kazakhstan").unwrap().iana_name(), Some("Asia/Almaty"));
        assert_eq!(find("tokyo").unwrap().iana_name(), Some("Asia/Tokyo"));
        assert_eq!(find("new york").unwrap().iana_name(), Some("America/New_York"));
        assert_eq!(find("buenos aires").unwrap().iana_name(), Some("America/Argentina/Buenos_Aires"));
        assert!(find("atlantis").is_none());
    }

    #[test]
    fn all_listed_zones_exist() {
        for (name, id) in INDEX.iter() {
            assert!(TimeZone::get(id).is_ok(), "{name} = {id}");
        }
    }
}
