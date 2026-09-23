//! Physical dimensions as exponent vectors over base quantities.

use std::fmt;

const BASES: usize = 9;
const NAMES: [&str; BASES] = ["length", "mass", "time", "current", "temperature", "amount", "angle", "data", "money"];

/// Exponents of the base quantities, e.g. speed is `length¹·time⁻¹`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default, Debug)]
pub struct Dim([i8; BASES]);

impl Dim {
    pub const NONE: Dim = Dim([0; BASES]);
    pub const LENGTH: Dim = Dim::base(0);
    pub const MASS: Dim = Dim::base(1);
    pub const TIME: Dim = Dim::base(2);
    pub const CURRENT: Dim = Dim::base(3);
    pub const TEMPERATURE: Dim = Dim::base(4);
    pub const AMOUNT: Dim = Dim::base(5);
    pub const ANGLE: Dim = Dim::base(6);
    pub const DATA: Dim = Dim::base(7);
    pub const MONEY: Dim = Dim::base(8);

    pub const AREA: Dim = Dim::LENGTH.pow(2);
    pub const VOLUME: Dim = Dim::LENGTH.pow(3);
    pub const FREQUENCY: Dim = Dim::TIME.pow(-1);
    pub const SPEED: Dim = Dim::LENGTH.div(Dim::TIME);
    pub const ACCELERATION: Dim = Dim::SPEED.div(Dim::TIME);
    pub const FORCE: Dim = Dim::MASS.mul(Dim::ACCELERATION);
    pub const ENERGY: Dim = Dim::FORCE.mul(Dim::LENGTH);
    pub const POWER: Dim = Dim::ENERGY.div(Dim::TIME);
    pub const PRESSURE: Dim = Dim::FORCE.div(Dim::AREA);
    pub const CHARGE: Dim = Dim::CURRENT.mul(Dim::TIME);
    pub const VOLTAGE: Dim = Dim::POWER.div(Dim::CURRENT);
    pub const RESISTANCE: Dim = Dim::VOLTAGE.div(Dim::CURRENT);
    pub const DATA_RATE: Dim = Dim::DATA.div(Dim::TIME);

    const fn base(i: usize) -> Dim {
        let mut d = [0; BASES];
        d[i] = 1;
        Dim(d)
    }

    pub const fn mul(self, other: Dim) -> Dim {
        let mut d = self.0;
        let mut i = 0;
        while i < BASES {
            d[i] += other.0[i];
            i += 1;
        }
        Dim(d)
    }

    pub const fn div(self, other: Dim) -> Dim {
        self.mul(other.pow(-1))
    }

    pub const fn pow(self, exp: i8) -> Dim {
        let mut d = self.0;
        let mut i = 0;
        while i < BASES {
            d[i] *= exp;
            i += 1;
        }
        Dim(d)
    }

    pub fn is_none(self) -> bool {
        self == Dim::NONE
    }

    /// Divides all exponents by `n` if they are all divisible (for roots).
    pub fn root(self, n: i8) -> Option<Dim> {
        let mut d = self.0;
        for e in &mut d {
            if *e % n != 0 {
                return None;
            }
            *e /= n;
        }
        Some(Dim(d))
    }

    /// A human name such as "length" or "speed", for error messages.
    pub fn name(self) -> String {
        let named = [
            (Dim::NONE, "number"),
            (Dim::AREA, "area"),
            (Dim::VOLUME, "volume"),
            (Dim::FREQUENCY, "frequency"),
            (Dim::SPEED, "speed"),
            (Dim::ACCELERATION, "acceleration"),
            (Dim::FORCE, "force"),
            (Dim::ENERGY, "energy"),
            (Dim::POWER, "power"),
            (Dim::PRESSURE, "pressure"),
            (Dim::VOLTAGE, "voltage"),
            (Dim::RESISTANCE, "resistance"),
            (Dim::DATA_RATE, "data rate"),
        ];
        if let Some((_, name)) = named.iter().find(|(d, _)| *d == self) {
            return name.to_string();
        }
        self.to_string()
    }
}

impl fmt::Display for Dim {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let parts: Vec<String> = self
            .0
            .iter()
            .zip(NAMES)
            .filter(|(e, _)| **e != 0)
            .map(|(e, name)| if *e == 1 { name.to_string() } else { format!("{name}^{e}") })
            .collect();
        write!(f, "{}", if parts.is_empty() { "number".into() } else { parts.join("·") })
    }
}
