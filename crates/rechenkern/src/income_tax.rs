//! Income tax by country: `income tax on $100,000 in US`, `€80,000 after tax in Germany`.
//! The schedules are in `data/income_tax.txt`.

use std::sync::LazyLock;

use crate::number::Number;

const DATA: &str = include_str!("data/income_tax.txt");

/// One country's income tax for a single resident employee.
#[derive(Debug)]
pub struct Country {
    pub currency: &'static str,
    pub names: Vec<&'static str>,
    allowance: Allowance,
    contribution: Number,
    schedules: Vec<(Base, Vec<(Number, Number)>)>,
    levy: Option<(Number, Number)>,
    credit: Credit,
    surtax: Number,
    cap: Option<Number>,
    germany: bool,
}

#[derive(Debug, Default)]
enum Allowance {
    #[default]
    None,
    Fixed(Number),
    /// Less `rate` per unit of income above `from`.
    Taper {
        amount: Number,
        from: Number,
        rate: Number,
    },
    /// Whole up to `full`, nothing from `zero`.
    Phase {
        amount: Number,
        full: Number,
        zero: Number,
    },
    /// By income: the first tier the income is within, then `rest`.
    Tiers {
        tiers: Vec<(Number, Number)>,
        rest: Number,
    },
}

#[derive(Debug, Default)]
enum Credit {
    #[default]
    None,
    Fixed(Number),
    /// An amount taxed at the lowest rate.
    AtFirstRate(Number),
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Base {
    Gross,
    Net,
    Taxable,
}

static COUNTRIES: LazyLock<Vec<Country>> = LazyLock::new(|| parse(DATA));

/// The country called `name` (lowercase): "germany", "us".
pub fn country(name: &str) -> Option<&'static Country> {
    COUNTRIES.iter().find(|c| c.names.contains(&name))
}

impl Country {
    /// The tax on a year's income, in the country's currency.
    pub fn tax(&self, income: Number) -> Number {
        let zero = Number::ZERO;
        let income = max(income, zero);
        let contribution = income * self.contribution;
        let net = income - contribution;
        let taxable = max(net - self.allowance(income), zero);
        let mut tax = if self.germany { germany(taxable) } else { zero };
        let mut first_rate = None;
        for (base, rates) in &self.schedules {
            let amount = match base {
                Base::Gross => income,
                Base::Net => net,
                Base::Taxable => taxable,
            };
            first_rate = first_rate.or(rates.first().map(|&(_, rate)| rate));
            tax = tax + progressive(amount, rates);
        }
        if let Some((rate, above)) = self.levy
            && income > above
        {
            tax = tax + income * rate;
        }
        tax = max(
            match self.credit {
                Credit::None => tax,
                Credit::Fixed(n) => tax - n,
                Credit::AtFirstRate(n) => tax - n * first_rate.unwrap_or(zero),
            },
            zero,
        );
        tax = tax * (Number::ONE + self.surtax);
        if let Some(cap) = self.cap {
            tax = min(tax, income * cap);
        }
        (tax + contribution).round_dp(2)
    }

    fn allowance(&self, income: Number) -> Number {
        match &self.allowance {
            Allowance::None => Number::ZERO,
            Allowance::Fixed(n) => *n,
            Allowance::Taper { amount, from, rate } => {
                max(*amount - max(income - *from, Number::ZERO) * *rate, Number::ZERO)
            }
            Allowance::Phase { amount, full, zero } => {
                if income <= *full {
                    *amount
                } else if income >= *zero {
                    Number::ZERO
                } else {
                    *amount * (*zero - income) / (*zero - *full)
                }
            }
            Allowance::Tiers { tiers, rest } => {
                tiers.iter().find(|(upto, _)| income <= *upto).map_or(*rest, |&(_, n)| n)
            }
        }
    }
}

fn max(a: Number, b: Number) -> Number {
    if a > b { a } else { b }
}

fn min(a: Number, b: Number) -> Number {
    if a < b { a } else { b }
}

/// Tax on `amount` at rates that each start at a threshold.
fn progressive(amount: Number, rates: &[(Number, Number)]) -> Number {
    let mut tax = Number::ZERO;
    for (k, &(from, rate)) in rates.iter().enumerate() {
        if amount <= from {
            break;
        }
        let to = rates.get(k + 1).map_or(amount, |&(next, _)| min(next, amount));
        tax = tax + (to - from) * rate;
    }
    tax
}

/// Germany's tariff (§32a EStG, 2025), rounded down to the euro.
fn germany(income: Number) -> Number {
    let x = income.floor();
    let n = |s: &str| Number::parse(s).expect("valid constant");
    let tax = if x <= n("12096") {
        Number::ZERO
    } else if x <= n("17443") {
        let y = (x - n("12096")) / n("10000");
        (n("932.30") * y + n("1400")) * y
    } else if x <= n("68480") {
        let z = (x - n("17443")) / n("10000");
        (n("176.64") * z + n("2397")) * z + n("1015.13")
    } else if x <= n("277825") {
        n("0.42") * x - n("10911.92")
    } else {
        n("0.45") * x - n("19246.67")
    };
    tax.floor()
}

fn parse(data: &'static str) -> Vec<Country> {
    let mut countries: Vec<Country> = Vec::new();
    let number = |s: &str| Number::parse(s.trim()).unwrap_or_else(|| panic!("bad number {s:?} in income_tax.txt"));
    let percent = |s: &str| number(s.trim().trim_end_matches('%')) / Number::from_i64(100);
    for line in data.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
        if let Some(header) = line.strip_prefix('[') {
            let (head, names) = header.split_once(" | ").expect("names after the header");
            // "[US] USD 2025": the code and the year are for people reading the file.
            let currency = head.split_whitespace().nth(1).expect("currency");
            countries.push(Country {
                currency,
                names: names.split(", ").collect(),
                allowance: Allowance::None,
                contribution: Number::ZERO,
                schedules: Vec::new(),
                levy: None,
                credit: Credit::None,
                surtax: Number::ZERO,
                cap: None,
                germany: false,
            });
            continue;
        }
        let country = countries.last_mut().expect("a country before its rates");
        let (key, rest) = line.split_once(' ').unwrap_or((line, ""));
        let words: Vec<&str> = rest.split_whitespace().collect();
        match key {
            "allowance" => {
                country.allowance = match words.as_slice() {
                    [n] => Allowance::Fixed(number(n)),
                    [n, "taper", from, rate] => {
                        Allowance::Taper { amount: number(n), from: number(from), rate: number(rate) }
                    }
                    [n, "phase", full, zero] => {
                        Allowance::Phase { amount: number(n), full: number(full), zero: number(zero) }
                    }
                    ["tiers", ..] => {
                        let mut tiers: Vec<(Number, Number)> = Vec::new();
                        let mut above = Number::ZERO;
                        for tier in rest.trim_start_matches("tiers").trim().split(", ") {
                            match tier.split_once(':') {
                                Some((upto, n)) => tiers.push((number(upto), number(n))),
                                None => above = number(tier),
                            }
                        }
                        Allowance::Tiers { tiers, rest: above }
                    }
                    _ => panic!("bad allowance {line:?}"),
                }
            }
            "contribution" => country.contribution = percent(rest),
            "gross" | "net" | "taxable" => {
                let base = match key {
                    "gross" => Base::Gross,
                    "net" => Base::Net,
                    _ => Base::Taxable,
                };
                let rates = rest
                    .split(", ")
                    .map(|r| {
                        let (from, rate) = r.split_once(' ').expect("threshold and rate");
                        (number(from), percent(rate))
                    })
                    .collect();
                country.schedules.push((base, rates));
            }
            "levy" => match words.as_slice() {
                [rate, "above", n] => country.levy = Some((percent(rate), number(n))),
                _ => panic!("bad levy {line:?}"),
            },
            "credit" => {
                country.credit = match words.as_slice() {
                    [n] => Credit::Fixed(number(n)),
                    [n, "at", "first", "rate"] => Credit::AtFirstRate(number(n)),
                    _ => panic!("bad credit {line:?}"),
                }
            }
            "surtax" => country.surtax = percent(rest),
            "cap" => country.cap = Some(percent(rest)),
            "formula" => country.germany = rest == "germany",
            _ => panic!("unknown line {line:?} in income_tax.txt"),
        }
    }
    countries
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tax(country_name: &str, income: i64) -> String {
        country(country_name).unwrap().tax(Number::from_i64(income)).to_string()
    }

    #[test]
    fn schedules() {
        assert_eq!(tax("us", 80_000), "9049");
        assert_eq!(tax("australia", 80_000), "16388");
        assert_eq!(tax("uk", 80_000), "19432");
        assert_eq!(tax("uk", 150_000), "53703");
        assert_eq!(tax("germany", 80_000), "22688");
        assert_eq!(tax("new zealand", 80_000), "16277.5");
        assert_eq!(tax("singapore", 80_000), "3350");
        assert_eq!(tax("russia", 3_000_000), "402000");
        assert_eq!(tax("china", 300_000), "31080");
        assert_eq!(tax("norway", 700_000), "147660.5");
        assert_eq!(tax("germany", 10_000), "0");
    }
}
