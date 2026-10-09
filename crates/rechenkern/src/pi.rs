//! Pi to any number of decimal places: the Chudnovsky series, summed by binary splitting.

use std::sync::{Mutex, MutexGuard, PoisonError};
use std::thread;

use dashu_int::ops::SquareRoot;
use dashu_int::{IBig, UBig};

use crate::ast::Direction;

/// The most decimal places pi is written to; a million take about half a second.
pub const MAX_PLACES: u32 = 1_000_000;

/// The longest run of pi's leading digits worked out so far, without the point: "31415".
static KNOWN: Mutex<String> = Mutex::new(String::new());

/// Pi to `places` decimals, rounded in a direction: "3.1416".
pub fn text(places: u32, dir: Direction) -> String {
    let places = places as usize;
    let next = usize::from(dir == Direction::Nearest);
    let mut digits = leading_digits(1 + places + next).into_bytes();
    // Pi is irrational: it is never halfway between two decimals, nor on one.
    let up = match dir {
        Direction::Nearest => digits.pop().is_some_and(|d| d >= b'5'),
        Direction::Up => true,
        Direction::Down => false,
    };
    if up {
        increment(&mut digits);
    }
    if places > 0 {
        digits.insert(1, b'.');
    }
    String::from_utf8(digits).expect("ASCII digits")
}

/// Adds one to the last digit, carrying through nines.
fn increment(digits: &mut [u8]) {
    for d in digits.iter_mut().rev() {
        if *d == b'9' {
            *d = b'0';
        } else {
            *d += 1;
            return;
        }
    }
}

/// The first `count` digits of pi: "31415" for 5.
fn leading_digits(count: usize) -> String {
    if let Some(digits) = known().get(..count) {
        return digits.to_string();
    }
    let digits = exact_digits(count);
    let mut known = known();
    if digits.len() > known.len() {
        *known = digits.clone();
    }
    digits
}

fn known() -> MutexGuard<'static, String> {
    KNOWN.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The first `count` digits of pi, worked out with guard digits.
fn exact_digits(count: usize) -> String {
    let mut guard = 10;
    loop {
        let mut digits = approximate(count - 1 + guard).to_string();
        // The approximation is a few units off in its last place, so the digits
        // before the guard are exact unless the guard is all zeros or all nines.
        let settled = &digits[count..digits.len() - 2];
        if settled.bytes().any(|d| d != b'0') && settled.bytes().any(|d| d != b'9') {
            digits.truncate(count);
            return digits;
        }
        guard *= 2;
    }
}

/// Pi times `10^places`, give or take a few units.
fn approximate(places: usize) -> IBig {
    // Each term adds about 14.18 digits.
    let terms = places as u64 / 14 + 2;
    let threads = match places {
        0..20_000 => 1,
        _ => thread::available_parallelism().map_or(1, |n| n.get().min(8)),
    };
    // pi = 426880 √10005 Q / T
    let root = || IBig::from((UBig::from(10005u32) * UBig::from(10u8).pow(2 * places)).sqrt());
    let (root, sums) = if threads > 1 {
        thread::scope(|s| {
            let root = s.spawn(root);
            let sums = split(0, terms, threads);
            (root.join().expect("square root"), sums)
        })
    } else {
        (root(), split(0, terms, 1))
    };
    sums.q * IBig::from(426880u32) * root / sums.t
}

/// Binary splitting sums of the terms `a..b` of the series.
struct Sums {
    p: IBig,
    q: IBig,
    t: IBig,
}

/// 640320³ / 24
const C3_24: u128 = 10_939_058_860_032_000;

fn split(a: u64, b: u64, threads: usize) -> Sums {
    if b - a == 1 {
        return term(a);
    }
    let m = (a + b) / 2;
    let parallel = threads > 1 && b - a > 1000;
    let (left, right) = if parallel {
        thread::scope(|s| {
            let right = s.spawn(|| split(m, b, threads / 2));
            let left = split(a, m, threads - threads / 2);
            (left, right.join().expect("series"))
        })
    } else {
        (split(a, m, 1), split(m, b, 1))
    };
    if parallel { merge_parallel(left, right) } else { merge(left, right) }
}

fn term(a: u64) -> Sums {
    if a == 0 {
        return Sums { p: IBig::ONE, q: IBig::ONE, t: IBig::from(13_591_409u32) };
    }
    let a = a as u128;
    let p = IBig::from((6 * a - 5) * (2 * a - 1) * (6 * a - 1));
    let q = IBig::from(a * a * a * C3_24);
    let t = &p * IBig::from(13_591_409 + 545_140_134 * a);
    Sums { t: if a % 2 == 1 { -t } else { t }, p, q }
}

fn merge(l: Sums, r: Sums) -> Sums {
    Sums { p: &l.p * &r.p, q: &l.q * &r.q, t: &r.q * l.t + &l.p * r.t }
}

/// The same as `merge`, with each product on its own thread.
fn merge_parallel(l: Sums, r: Sums) -> Sums {
    thread::scope(|s| {
        let p = s.spawn(|| &l.p * &r.p);
        let q = s.spawn(|| &l.q * &r.q);
        let t = s.spawn(|| &r.q * &l.t);
        let t2 = &l.p * &r.t;
        Sums { p: p.join().expect("product"), q: q.join().expect("product"), t: t.join().expect("product") + t2 }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounds_in_each_direction() {
        assert_eq!(text(0, Direction::Nearest), "3");
        assert_eq!(text(4, Direction::Nearest), "3.1416");
        assert_eq!(text(4, Direction::Down), "3.1415");
        assert_eq!(text(4, Direction::Up), "3.1416");
        // Six nines from the 762nd decimal: the Feynman point.
        assert!(text(767, Direction::Nearest).ends_with("35000000"));
        assert!(text(767, Direction::Down).ends_with("34999999"));
    }

    #[test]
    fn known_digits() {
        // Decimals 99,991 to 100,000.
        assert!(text(100_000, Direction::Down).ends_with("5493624646"));
        assert_eq!(text(50, Direction::Down), "3.14159265358979323846264338327950288419716939937510");
    }
}
