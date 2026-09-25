//! A fast hasher for the built-in lookup tables. Their keys are not user
//! input, so they don't need the flood-resistant default hasher.

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

/// The Fx hash used inside rustc: one rotate, xor and multiply per word.
#[derive(Default)]
pub(crate) struct FxHasher(u64);

impl FxHasher {
    fn add(&mut self, word: u64) {
        self.0 = (self.0.rotate_left(5) ^ word).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
    }
}

impl Hasher for FxHasher {
    fn write(&mut self, bytes: &[u8]) {
        let (chunks, rest) = bytes.as_chunks::<8>();
        for chunk in chunks {
            self.add(u64::from_le_bytes(*chunk));
        }
        if !rest.is_empty() {
            let mut buf = [0; 8];
            buf[..rest.len()].copy_from_slice(rest);
            self.add(u64::from_le_bytes(buf));
        }
    }

    fn write_u8(&mut self, byte: u8) {
        self.add(byte.into());
    }

    fn finish(&self) -> u64 {
        // The multiply mixes into the high bits; the table also needs good low bits.
        self.0.rotate_left(26)
    }
}

/// A map for built-in tables.
pub(crate) type TableMap<K, V> = HashMap<K, V, BuildHasherDefault<FxHasher>>;
