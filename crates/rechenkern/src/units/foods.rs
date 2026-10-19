//! Foods and what a cup of them weighs, so their amounts convert between weight and volume.

use std::sync::LazyLock;

use crate::number::Number;

/// Lowercase food names, grams per US cup and grams per piece, from USDA FoodData Central.
const FOODS: &str = include_str!("foods.txt");

/// A food, known by its weight per volume: "1 cup of flour" weighs 125 g.
#[derive(Clone, Copy, Debug)]
pub struct Food {
    pub grams_per_cup: Number,
    /// What a piece weighs, for foods that are counted: "stick" of butter, "each" egg, "large" egg.
    pub pieces: &'static [(&'static str, Number)],
}

impl Food {
    /// Grams in one piece: a "stick", a "clove", or a size of a food counted by itself ("each", "large").
    pub fn piece(&self, name: &str) -> Option<Number> {
        self.pieces.iter().find(|(n, _)| *n == name).map(|&(_, grams)| grams)
    }
}

static ALL: LazyLock<Vec<(&'static str, Food)>> = LazyLock::new(|| {
    FOODS
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|line| {
            let (name, rest) = line.split_once(" = ")?;
            let (grams, pieces) = rest.split_once("; ").unwrap_or((rest, ""));
            let pieces = pieces
                .split(", ")
                .filter(|p| !p.is_empty())
                .map(|p| {
                    let (piece, grams) = p.rsplit_once(' ')?;
                    Some((piece, Number::parse(grams)?))
                })
                .collect::<Option<Vec<_>>>()?;
            Some((name, Food { grams_per_cup: Number::parse(grams)?, pieces: pieces.leak() }))
        })
        .collect()
});

/// Every food name, in table order.
pub fn all() -> &'static [(&'static str, Food)] {
    &ALL
}
