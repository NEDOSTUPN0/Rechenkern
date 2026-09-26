//! Time zones by place name, abbreviation or UTC offset.

use std::sync::LazyLock;

use jiff::Zoned;
use jiff::tz::{Offset, TimeZone};

use crate::hash::TableMap;

const PLACES: &str = include_str!("places.txt");
/// About 57k city names from GeoNames, sorted for binary search.
const CITIES: &str = include_str!("cities.txt");

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
static INDEX: LazyLock<TableMap<String, String>> = LazyLock::new(|| {
    let mut index = TableMap::default();
    for line in PLACES.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
        if let Some((name, zone)) = line.split_once('=') {
            index.insert(name.trim().to_string(), zone.trim().to_string());
        }
    }
    // Cities from the zone database itself: "Asia/Almaty" -> "almaty". The
    // bundled copy lists the same names as the system one without opening
    // every zone file; `find` checks that the system has the zone.
    for id in jiff::tz::TimeZoneDatabase::bundled().available() {
        let id = id.as_str();
        if !id.contains('/') || SKIPPED_PREFIXES.iter().any(|p| id.starts_with(p)) {
            continue;
        }
        let city = id.rsplit('/').next().unwrap_or(id).replace('_', " ").to_lowercase();
        index.entry(city).or_insert_with(|| id.to_string());
    }
    index
});

/// The `name\tzone` lines of `cities.txt`, without the header.
fn city_lines() -> &'static str {
    let mut data = CITIES;
    while data.starts_with('#') {
        data = data.split_once('\n').map_or("", |(_, rest)| rest);
    }
    data
}

/// All cities as (name, zone id).
fn cities() -> impl Iterator<Item = (&'static str, &'static str)> {
    // Scanning bytes is much faster here than `lines()` and `split_once()`.
    let mut rest = city_lines();
    std::iter::from_fn(move || {
        while !rest.is_empty() {
            let end = rest.bytes().position(|b| b == b'\n').unwrap_or(rest.len());
            let line = &rest[..end];
            rest = rest.get(end + 1..).unwrap_or_default();
            if let Some(tab) = line.bytes().position(|b| b == b'\t') {
                return Some((&line[..tab], &line[tab + 1..]));
            }
        }
        None
    })
}

/// Binary search right in the sorted text: no index to build at startup.
fn city(name: &str) -> Option<&'static str> {
    let data = city_lines();
    let bytes = data.as_bytes();
    // `lo` and `hi` always sit at line starts.
    let (mut lo, mut hi) = (0, data.len());
    while lo < hi {
        let mid = (lo + hi) / 2;
        let start = bytes[lo..mid].iter().rposition(|&b| b == b'\n').map_or(lo, |p| lo + p + 1);
        let end = bytes[start..hi].iter().position(|&b| b == b'\n').map_or(hi, |p| start + p);
        let (city, zone) = data[start..end].split_once('\t')?;
        match city.cmp(name) {
            std::cmp::Ordering::Equal => return Some(zone),
            std::cmp::Ordering::Less => lo = end + 1,
            std::cmp::Ordering::Greater => hi = start,
        }
    }
    None
}

/// Longest place name in words.
pub const MAX_WORDS: usize = 5;

/// Finds a zone by lowercase place name or abbreviation, like "new york" or "pst".
pub fn find(name: &str) -> Option<TimeZone> {
    if let Some(zone) = INDEX.get(name).and_then(|id| listed(id)) {
        return Some(zone);
    }
    // Full ids like "asia/tokyo" typed by the user.
    if name.contains('/') {
        return TimeZone::get(name).ok();
    }
    city(name).and_then(|id| TimeZone::get(id).ok())
}

/// A zone from `places.txt`: an IANA id or a fixed offset in POSIX form ("EDT4").
fn listed(id: &str) -> Option<TimeZone> {
    TimeZone::get(id).or_else(|_| TimeZone::posix(id)).ok()
}

/// The known place spelled most like `name`, for "did you mean" hints.
pub fn suggest(name: &str) -> Option<String> {
    if name.chars().count() < 4 {
        return None;
    }
    let limit = if name.chars().count() <= 5 { 1 } else { 2 };
    let target: Vec<char> = name.chars().collect();
    let mut row = Vec::new();
    let mut distance =
        |k: &str| (k.len().abs_diff(name.len()) <= limit).then(|| edit_distance(k, &target, limit, &mut row));
    // Listed places win ties over other cities, and ties among them go alphabetically.
    let listed = INDEX.keys().filter_map(|k| Some((distance(k)?, k.as_str()))).min();
    let city = cities().filter_map(|(city, _)| Some((distance(city)?, city))).min_by_key(|&(d, _)| d);
    let (distance, best) = match (listed, city) {
        (Some(listed), Some(city)) if city.0 < listed.0 => city,
        (listed, city) => listed.or(city)?,
    };
    (distance <= limit).then(|| title_case(best))
}

/// Levenshtein distance from `a` to `b`, or anything above `limit` once it
/// can't get back under it.
fn edit_distance(a: &str, b: &[char], limit: usize, row: &mut Vec<usize>) -> usize {
    row.clear();
    row.extend(0..=b.len());
    for (i, ca) in a.chars().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, &cb) in b.iter().enumerate() {
            let substitution = diagonal + usize::from(ca != cb);
            diagonal = row[j + 1];
            row[j + 1] = substitution.min(row[j] + 1).min(diagonal + 1);
        }
        if row.iter().all(|&d| d > limit) {
            return limit + 1;
        }
    }
    row[b.len()]
}

/// "new york" -> "New York".
fn title_case(name: &str) -> String {
    name.split(' ')
        .map(|w| {
            let mut chars = w.chars();
            chars.next().map_or(String::new(), |c| c.to_uppercase().chain(chars).collect())
        })
        .collect::<Vec<_>>()
        .join(" ")
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
        assert_eq!(find("brazil").unwrap().iana_name(), Some("America/Sao_Paulo"));
        assert_eq!(find("osaka").unwrap().iana_name(), Some("Asia/Tokyo"));
        assert_eq!(find("tokyo").unwrap().iana_name(), Some("Asia/Tokyo"));
        assert_eq!(find("new york").unwrap().iana_name(), Some("America/New_York"));
        assert_eq!(find("buenos aires").unwrap().iana_name(), Some("America/Argentina/Buenos_Aires"));
        assert!(find("qwertyville").is_none());
    }

    #[test]
    fn cities_come_from_geonames() {
        assert_eq!(find("marseille").unwrap().iana_name(), Some("Europe/Paris"));
        assert_eq!(find("sao jose dos campos").unwrap().iana_name(), Some("America/Sao_Paulo"));
        let names: Vec<&str> = cities().map(|(city, _)| city).collect();
        assert!(names.windows(2).all(|w| w[0] < w[1]), "cities.txt must be sorted");
        // Every name is found by the binary search.
        assert!(cities().all(|(name, zone)| city(name) == Some(zone)));
        assert_eq!(city("zzzzzz"), None);
        assert_eq!(city("aaa"), None);
    }

    #[test]
    fn suggests_close_spellings() {
        assert_eq!(suggest("marseile").as_deref(), Some("Marseille"));
        assert_eq!(suggest("xqzwv"), None);
    }

    #[test]
    fn city_zones_exist() {
        let mut ids: Vec<&str> = cities().map(|(_, id)| id).collect();
        ids.sort_unstable();
        ids.dedup();
        for id in ids {
            assert!(TimeZone::get(id).is_ok(), "{id}");
        }
    }

    #[test]
    fn all_listed_zones_exist() {
        for (name, id) in INDEX.iter() {
            assert!(listed(id).is_some(), "{name} = {id}");
        }
    }
}
