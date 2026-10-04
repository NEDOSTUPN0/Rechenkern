//! Foods and what a cup of them weighs, so their amounts convert between weight and volume.

use std::sync::LazyLock;

use crate::number::Number;

/// Lowercase food names and grams per US cup, from USDA FoodData Central.
const FOODS: &str = include_str!("foods.txt");

/// A food, known by its weight per volume: "1 cup of flour" weighs 125 g.
#[derive(Clone, Copy, Debug)]
pub struct Food {
    pub grams_per_cup: Number,
}

static ALL: LazyLock<Vec<(&'static str, Food)>> = LazyLock::new(|| {
    FOODS
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|line| {
            let (name, grams) = line.split_once(" = ")?;
            Some((name, Food { grams_per_cup: Number::parse(grams)? }))
        })
        .collect()
});

/// Every food name, in table order.
pub fn all() -> &'static [(&'static str, Food)] {
    &ALL
}
