//! The sun and the moon: sunrise, sunset, the phases of the moon, moonrise and moonset.
//!
//! The sun follows NOAA's solar calculator, the moon's phases chapter 49 of
//! Meeus' "Astronomical Algorithms". Both are good to about a minute, away
//! from the poles for the sun. Moonrise and moonset take the moon from
//! the largest terms of Meeus' chapter 47, good to a couple of minutes.

use jiff::Timestamp;

/// Julian day of the Unix epoch.
const UNIX_EPOCH: f64 = 2440587.5;
/// Julian day of J2000.0.
const J2000: f64 = 2451545.0;
/// Mean length of a lunar month in days.
const SYNODIC_MONTH: f64 = 29.530588861;
/// Terrestrial time runs ahead of UT by about this many seconds in the 2020s;
/// a few seconds either way change nothing at a minute's precision.
const DELTA_T: f64 = 69.0;

fn julian_day(t: Timestamp) -> f64 {
    UNIX_EPOCH + t.as_millisecond() as f64 / 86_400_000.0
}

fn timestamp(jd: f64) -> Option<Timestamp> {
    Timestamp::from_millisecond(((jd - UNIX_EPOCH) * 86_400_000.0).round() as i64).ok()
}

fn sin(degrees: f64) -> f64 {
    degrees.to_radians().sin()
}

fn cos(degrees: f64) -> f64 {
    degrees.to_radians().cos()
}

/// The sun on one day at one place.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SunDay {
    Rises {
        rise: Timestamp,
        set: Timestamp,
    },
    /// Polar day: the sun never sets.
    AlwaysUp,
    /// Polar night: the sun never rises.
    AlwaysDown,
}

/// Sunrise and sunset on the solar day around `noon` at a place, in degrees
/// north and east. The sun's upper edge meets the horizon, with refraction.
pub fn sun_day(noon: Timestamp, latitude: f64, longitude: f64) -> Option<SunDay> {
    // Midnight UT of the day whose solar noon at this longitude is nearest `noon`.
    let midnight = (julian_day(noon) - 1.0 + longitude / 360.0).round() + 0.5;
    let transit = |jd: f64| midnight + (720.0 - 4.0 * longitude - sun(jd).equation_of_time) / 1440.0;
    // Hour angle of sunrise in degrees, or how the sun stays when it has none.
    let hour_angle = |jd: f64| -> Result<f64, SunDay> {
        let declination = sun(jd).declination;
        let cos_h = (cos(90.833) - sin(latitude) * sin(declination)) / (cos(latitude) * cos(declination));
        match cos_h {
            c if c < -1.0 => Err(SunDay::AlwaysUp),
            c if c > 1.0 => Err(SunDay::AlwaysDown),
            c => Ok(c.acos().to_degrees()),
        }
    };
    let noon = transit(transit(midnight + 0.5));
    let mut events = [0.0; 2];
    for (event, sign) in events.iter_mut().zip([-1.0, 1.0]) {
        let mut jd = match hour_angle(noon) {
            Ok(h) => noon + sign * h / 360.0,
            Err(polar) => return Some(polar),
        };
        // The sun moves during the morning: look again at the time found.
        for _ in 0..3 {
            match hour_angle(jd) {
                Ok(h) => jd = transit(jd) + sign * h / 360.0,
                Err(_) => break,
            }
        }
        *event = jd;
    }
    Some(SunDay::Rises { rise: timestamp(events[0])?, set: timestamp(events[1])? })
}

/// Where the sun is at a moment: declination in degrees, and the equation of
/// time in minutes (how far the sundial runs ahead of the clock).
struct Sun {
    declination: f64,
    equation_of_time: f64,
}

fn sun(jd: f64) -> Sun {
    let t = (jd - J2000) / 36525.0;
    let mean_longitude = (280.46646 + t * (36000.76983 + t * 0.0003032)).rem_euclid(360.0);
    let anomaly = 357.52911 + t * (35999.05029 - 0.0001537 * t);
    let eccentricity = 0.016708634 - t * (0.000042037 + 0.0000001267 * t);
    let center = sin(anomaly) * (1.914602 - t * (0.004817 + 0.000014 * t))
        + sin(2.0 * anomaly) * (0.019993 - 0.000101 * t)
        + sin(3.0 * anomaly) * 0.000289;
    let node = 125.04 - 1934.136 * t;
    let longitude = mean_longitude + center - 0.00569 - 0.00478 * sin(node);
    let mean_obliquity = 23.0 + (26.0 + (21.448 - t * (46.815 + t * (0.00059 - t * 0.001813))) / 60.0) / 60.0;
    let obliquity = mean_obliquity + 0.00256 * cos(node);
    let declination = (sin(obliquity) * sin(longitude)).asin().to_degrees();
    let y = (obliquity / 2.0).to_radians().tan().powi(2);
    let (l, m, e) = (mean_longitude, anomaly, eccentricity);
    let equation = y * sin(2.0 * l) - 2.0 * e * sin(m) + 4.0 * e * y * sin(m) * cos(2.0 * l)
        - 0.5 * y * y * sin(4.0 * l)
        - 1.25 * e * e * sin(2.0 * m);
    Sun { declination, equation_of_time: 4.0 * equation.to_degrees() }
}

/// One of the eight phases of the moon.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    New,
    WaxingCrescent,
    FirstQuarter,
    WaxingGibbous,
    Full,
    WaningGibbous,
    LastQuarter,
    WaningCrescent,
}

impl Phase {
    pub fn name(self) -> &'static str {
        match self {
            Phase::New => "New Moon",
            Phase::WaxingCrescent => "Waxing Crescent",
            Phase::FirstQuarter => "First Quarter",
            Phase::WaxingGibbous => "Waxing Gibbous",
            Phase::Full => "Full Moon",
            Phase::WaningGibbous => "Waning Gibbous",
            Phase::LastQuarter => "Last Quarter",
            Phase::WaningCrescent => "Waning Crescent",
        }
    }
}

/// The moon at a moment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Moon {
    /// A main phase lasts an eighth of the month, centered on its exact time.
    pub phase: Phase,
    /// Days since the last new moon.
    pub age: f64,
}

/// When the moon rises and sets between two moments at a place, in degrees north and east;
/// `None` where it doesn't. Its upper edge meets the horizon, with refraction and parallax.
pub fn moon_rise_set(
    from: Timestamp,
    to: Timestamp,
    latitude: f64,
    longitude: f64,
) -> (Option<Timestamp>, Option<Timestamp>) {
    let (start, end) = (julian_day(from), julian_day(to));
    let above = |jd: f64| moon_altitude(jd, latitude, longitude);
    let (mut rise, mut set) = (None, None);
    // Ten minutes at a time, then halving the step where the moon crosses the horizon.
    const STEP: f64 = 10.0 / 1440.0;
    let mut a = start;
    let mut alt_a = above(a);
    while a < end {
        let b = (a + STEP).min(end);
        let alt_b = above(b);
        if (alt_a < 0.0) != (alt_b < 0.0) {
            let (mut lo, mut hi) = (a, b);
            for _ in 0..20 {
                let mid = (lo + hi) / 2.0;
                if (above(mid) < 0.0) == (alt_a < 0.0) { lo = mid } else { hi = mid }
            }
            let slot = if alt_a < 0.0 { &mut rise } else { &mut set };
            if slot.is_none() {
                *slot = timestamp((lo + hi) / 2.0);
            }
        }
        (a, alt_a) = (b, alt_b);
    }
    (rise, set)
}

/// The moon's altitude above the height where its upper edge touches the horizon, in degrees.
fn moon_altitude(jd: f64, latitude: f64, longitude: f64) -> f64 {
    let (lambda, beta, distance) = moon_position(jd + DELTA_T / 86_400.0);
    let parallax = (6378.14 / distance).asin().to_degrees();
    let t = (jd - J2000) / 36525.0;
    let obliquity = 23.439 - 0.013 * t;
    let ra = (sin(lambda) * cos(obliquity) - beta.to_radians().tan() * sin(obliquity)).atan2(cos(lambda)).to_degrees();
    let dec = (sin(beta) * cos(obliquity) + cos(beta) * sin(obliquity) * sin(lambda)).asin().to_degrees();
    let sidereal = 280.46061837 + 360.98564736629 * (jd - J2000);
    let hour_angle = sidereal + longitude - ra;
    let altitude = (sin(latitude) * sin(dec) + cos(latitude) * cos(dec) * cos(hour_angle)).asin().to_degrees();
    altitude - (0.7275 * parallax - 0.5667)
}

/// The moon's ecliptic longitude and latitude in degrees and its distance in km at `jde`, from
/// the largest terms of Meeus' chapter 47: good to about a hundredth of a degree.
fn moon_position(jde: f64) -> (f64, f64, f64) {
    // Multiples of D, M, M', F, then the terms of longitude (1e-6 °) and distance (m).
    #[rustfmt::skip]
    const LR: [(f64, f64, f64, f64, f64, f64); 25] = [
        (0.0, 0.0, 1.0, 0.0, 6288774.0, -20905355.0), (2.0, 0.0, -1.0, 0.0, 1274027.0, -3699111.0),
        (2.0, 0.0, 0.0, 0.0, 658314.0, -2955968.0), (0.0, 0.0, 2.0, 0.0, 213618.0, -569925.0),
        (0.0, 1.0, 0.0, 0.0, -185116.0, 48888.0), (0.0, 0.0, 0.0, 2.0, -114332.0, -3149.0),
        (2.0, 0.0, -2.0, 0.0, 58793.0, 246158.0), (2.0, -1.0, -1.0, 0.0, 57066.0, -152138.0),
        (2.0, 0.0, 1.0, 0.0, 53322.0, -170733.0), (2.0, -1.0, 0.0, 0.0, 45758.0, -204586.0),
        (0.0, 1.0, -1.0, 0.0, -40923.0, -129620.0), (1.0, 0.0, 0.0, 0.0, -34720.0, 108743.0),
        (0.0, 1.0, 1.0, 0.0, -30383.0, 104755.0), (2.0, 0.0, 0.0, -2.0, 15327.0, 10321.0),
        (0.0, 0.0, 1.0, 2.0, -12528.0, 0.0), (0.0, 0.0, 1.0, -2.0, 10980.0, 79661.0),
        (4.0, 0.0, -1.0, 0.0, 10675.0, -34782.0), (0.0, 0.0, 3.0, 0.0, 10034.0, -23210.0),
        (4.0, 0.0, -2.0, 0.0, 8548.0, -21636.0), (2.0, 1.0, -1.0, 0.0, -7888.0, 24208.0),
        (2.0, 1.0, 0.0, 0.0, -6766.0, 30824.0), (1.0, 0.0, -1.0, 0.0, -5163.0, -8379.0),
        (1.0, 1.0, 0.0, 0.0, 4987.0, -16675.0), (2.0, -1.0, 1.0, 0.0, 4036.0, -12831.0),
        (2.0, 0.0, 2.0, 0.0, 3994.0, -10445.0),
    ];
    #[rustfmt::skip]
    const B: [(f64, f64, f64, f64, f64); 13] = [
        (0.0, 0.0, 0.0, 1.0, 5128122.0), (0.0, 0.0, 1.0, 1.0, 280602.0), (0.0, 0.0, 1.0, -1.0, 277693.0),
        (2.0, 0.0, 0.0, -1.0, 173237.0), (2.0, 0.0, -1.0, 1.0, 55413.0), (2.0, 0.0, -1.0, -1.0, 46271.0),
        (2.0, 0.0, 0.0, 1.0, 32573.0), (0.0, 0.0, 2.0, 1.0, 17198.0), (2.0, 0.0, 1.0, -1.0, 9266.0),
        (0.0, 0.0, 2.0, -1.0, 8822.0), (2.0, -1.0, 0.0, -1.0, 8216.0), (2.0, 0.0, -2.0, -1.0, 4324.0),
        (2.0, 0.0, 1.0, 1.0, 4200.0),
    ];
    let t = (jde - J2000) / 36525.0;
    let l = 218.3164477 + 481267.88123421 * t;
    let d = 297.8501921 + 445267.1114034 * t;
    let m = 357.5291092 + 35999.0502909 * t;
    let mp = 134.9633964 + 477198.8675055 * t;
    let f = 93.2720950 + 483202.0175233 * t;
    let e = 1.0 - 0.002516 * t;
    let eccentricity = |k: f64| e.powi(k.abs() as i32);
    let (mut sl, mut sr, mut sb) = (0.0, 0.0, 0.0);
    for (kd, km, kmp, kf, cl, cr) in LR {
        let arg = kd * d + km * m + kmp * mp + kf * f;
        sl += cl * eccentricity(km) * sin(arg);
        sr += cr * eccentricity(km) * cos(arg);
    }
    for (kd, km, kmp, kf, cb) in B {
        sb += cb * eccentricity(km) * sin(kd * d + km * m + kmp * mp + kf * f);
    }
    (l + sl / 1e6, sb / 1e6, 385000.56 + sr / 1000.0)
}

pub fn moon(t: Timestamp) -> Moon {
    let jde = julian_day(t) + DELTA_T / 86_400.0;
    // Main phases are counted in quarters of a month from the new moon of January 2000.
    let mut quarter = ((jde - 2451550.09766) / SYNODIC_MONTH * 4.0).floor();
    while main_phase(quarter) > jde {
        quarter -= 1.0;
    }
    while main_phase(quarter + 1.0) <= jde {
        quarter += 1.0;
    }
    let (last, next) = (main_phase(quarter), main_phase(quarter + 1.0));
    let after = [Phase::New, Phase::FirstQuarter, Phase::Full, Phase::LastQuarter];
    let between = [Phase::WaxingCrescent, Phase::WaxingGibbous, Phase::WaningGibbous, Phase::WaningCrescent];
    let index = quarter.rem_euclid(4.0) as usize;
    let half = SYNODIC_MONTH / 16.0;
    let phase = if jde - last < half {
        after[index]
    } else if next - jde < half {
        after[(index + 1) % 4]
    } else {
        between[index]
    };
    let age = jde - main_phase(quarter - index as f64);
    Moon { phase, age }
}

/// Time (JDE) of a main phase: `quarter` counts quarters of a month from the
/// new moon of 6 January 2000, so 0 is new, 1 first quarter, 2 full moon...
fn main_phase(quarter: f64) -> f64 {
    let k = quarter / 4.0;
    let t = k / 1236.85;
    let (t2, t3, t4) = (t * t, t * t * t, t * t * t * t);
    let mean = 2451550.09766 + SYNODIC_MONTH * k + 0.00015437 * t2 - 0.000000150 * t3 + 0.00000000073 * t4;
    let e = 1.0 - 0.002516 * t - 0.0000074 * t2;
    let m = 2.5534 + 29.10535670 * k - 0.0000014 * t2 - 0.00000011 * t3;
    let mp = 201.5643 + 385.81693528 * k + 0.0107582 * t2 + 0.00001238 * t3 - 0.000000058 * t4;
    let f = 160.7108 + 390.67050284 * k - 0.0016118 * t2 - 0.00000227 * t3 + 0.000000011 * t4;
    let omega = 124.7746 - 1.56375588 * k + 0.0020672 * t2 + 0.00000215 * t3;

    let correction = match quarter.rem_euclid(4.0) as u8 {
        q @ (0 | 2) => {
            // New and full moons differ in their largest terms only.
            let c = if q == 0 {
                [-0.40720, 0.17241, 0.01608, 0.01039, 0.00739, -0.00514, 0.00208]
            } else {
                [-0.40614, 0.17302, 0.01614, 0.01043, 0.00734, -0.00515, 0.00209]
            };
            c[0] * sin(mp)
                + c[1] * e * sin(m)
                + c[2] * sin(2.0 * mp)
                + c[3] * sin(2.0 * f)
                + c[4] * e * sin(mp - m)
                + c[5] * e * sin(mp + m)
                + c[6] * e * e * sin(2.0 * m)
                - 0.00111 * sin(mp - 2.0 * f)
                - 0.00057 * sin(mp + 2.0 * f)
                + 0.00056 * e * sin(2.0 * mp + m)
                - 0.00042 * sin(3.0 * mp)
                + 0.00042 * e * sin(m + 2.0 * f)
                + 0.00038 * e * sin(m - 2.0 * f)
                - 0.00024 * e * sin(2.0 * mp - m)
                - 0.00017 * sin(omega)
                - 0.00007 * sin(mp + 2.0 * m)
                + 0.00004 * sin(2.0 * mp - 2.0 * f)
                + 0.00004 * sin(3.0 * m)
                + 0.00003 * sin(mp + m - 2.0 * f)
                + 0.00003 * sin(2.0 * mp + 2.0 * f)
                - 0.00003 * sin(mp + m + 2.0 * f)
                + 0.00003 * sin(mp - m + 2.0 * f)
                - 0.00002 * sin(mp - m - 2.0 * f)
                - 0.00002 * sin(3.0 * mp + m)
                + 0.00002 * sin(4.0 * mp)
        }
        q => {
            let w = 0.00306 - 0.00038 * e * cos(m) + 0.00026 * cos(mp) - 0.00002 * cos(mp - m)
                + 0.00002 * cos(mp + m)
                + 0.00002 * cos(2.0 * f);
            let w = if q == 1 { w } else { -w };
            -0.62801 * sin(mp) + 0.17172 * e * sin(m) - 0.01183 * e * sin(mp + m)
                + 0.00862 * sin(2.0 * mp)
                + 0.00804 * sin(2.0 * f)
                + 0.00454 * e * sin(mp - m)
                + 0.00204 * e * e * sin(2.0 * m)
                - 0.00180 * sin(mp - 2.0 * f)
                - 0.00070 * sin(mp + 2.0 * f)
                - 0.00040 * sin(3.0 * mp)
                - 0.00034 * e * sin(2.0 * mp - m)
                + 0.00032 * e * sin(m + 2.0 * f)
                + 0.00032 * e * sin(m - 2.0 * f)
                - 0.00028 * e * e * sin(mp + 2.0 * m)
                + 0.00027 * e * sin(2.0 * mp + m)
                - 0.00017 * sin(omega)
                - 0.00005 * sin(mp - m - 2.0 * f)
                + 0.00004 * sin(2.0 * mp + 2.0 * f)
                - 0.00004 * sin(mp + m + 2.0 * f)
                + 0.00004 * sin(mp - 2.0 * m)
                + 0.00003 * sin(mp + m - 2.0 * f)
                + 0.00003 * sin(3.0 * m)
                + 0.00002 * sin(2.0 * mp - 2.0 * f)
                + 0.00002 * sin(mp - m + 2.0 * f)
                - 0.00002 * sin(3.0 * mp + m)
                + w
        }
    };
    // Corrections from the planets, the same for every phase.
    #[rustfmt::skip]
    let planets = [
        (299.77, 0.107408, 0.000325), (251.88, 0.016321, 0.000165), (251.83, 26.651886, 0.000164),
        (349.42, 36.412478, 0.000126), (84.66, 18.206239, 0.000110), (141.74, 53.303771, 0.000062),
        (207.14, 2.453732, 0.000060), (154.84, 7.306860, 0.000056), (34.52, 27.261239, 0.000047),
        (207.19, 0.121824, 0.000042), (291.34, 1.844379, 0.000040), (161.72, 24.198154, 0.000037),
        (239.56, 25.513099, 0.000035), (331.55, 3.592518, 0.000023),
    ];
    let a1_square = -0.009173 * t2;
    let additional: f64 = planets
        .iter()
        .enumerate()
        .map(|(i, &(a, b, size))| size * sin(a + b * k + if i == 0 { a1_square } else { 0.0 }))
        .sum();
    mean + correction + additional
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> Timestamp {
        text.parse().unwrap()
    }

    /// Seconds between two moments.
    fn apart(a: Timestamp, b: Timestamp) -> i64 {
        (a.as_second() - b.as_second()).abs()
    }

    // Reference times are from PyEphem, with the sun's upper edge 34' below the horizon.
    #[test]
    fn sunrise_and_sunset() {
        let cases = [
            ("2026-09-24T03:00Z", 35.69, 139.69, "2026-09-23T20:30:21Z", "2026-09-24T08:35:53Z"),
            ("2026-09-23T11:00Z", 51.51, -0.13, "2026-09-23T05:48:07Z", "2026-09-23T17:56:41Z"),
            ("2026-12-21T01:00Z", -33.87, 151.21, "2026-12-20T18:40:37Z", "2026-12-21T09:05:26Z"),
        ];
        for (noon, latitude, longitude, rise, set) in cases {
            let Some(SunDay::Rises { rise: r, set: s }) = sun_day(at(noon), latitude, longitude) else {
                panic!("{noon}")
            };
            assert!(apart(r, at(rise)) < 60 && apart(s, at(set)) < 60, "{noon}: {r} {s}");
        }
    }

    #[test]
    fn polar_day_and_night() {
        assert_eq!(sun_day(at("2026-06-21T11:00Z"), 69.65, 18.96), Some(SunDay::AlwaysUp));
        assert_eq!(sun_day(at("2026-12-21T11:00Z"), 69.65, 18.96), Some(SunDay::AlwaysDown));
    }

    #[test]
    fn main_phases() {
        // The new moon, first quarter, full moon and last quarter around September 2026.
        let times = ["2026-09-11T03:26:55Z", "2026-09-18T20:43:42Z", "2026-09-26T16:48:58Z", "2026-10-03T13:24:58Z"];
        let first = ((julian_day(at(times[0])) - 2451550.09766) / SYNODIC_MONTH * 4.0).round();
        for (k, time) in times.iter().enumerate() {
            let found = timestamp(main_phase(first + k as f64) - DELTA_T / 86_400.0).unwrap();
            assert!(apart(found, at(time)) < 120, "{time}: {found}");
        }
    }

    #[test]
    fn phases_and_age() {
        let moon = |text| moon(at(text));
        assert_eq!(moon("2026-09-23T18:30Z").phase, Phase::WaxingGibbous);
        assert_eq!(moon("2026-09-23T18:30Z").age.floor(), 12.0);
        // A main phase lasts from 1.8 days before its time to 1.8 days after.
        assert_eq!(moon("2026-09-25T00:00Z").phase, Phase::Full);
        assert_eq!(moon("2026-09-28T12:00Z").phase, Phase::Full);
        assert_eq!(moon("2026-09-28T20:00Z").phase, Phase::WaningGibbous);
        assert_eq!(moon("2026-09-11T00:00Z").phase, Phase::New);
        assert!(moon("2026-09-11T04:00Z").age < 0.1);
        assert!(moon("2026-09-11T03:00Z").age > 29.0);
    }
}
