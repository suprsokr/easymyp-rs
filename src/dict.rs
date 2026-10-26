//! The bundled filename dictionary.
//!
//! Rows are `ph#sh#filename#crc`. An empty filename means the hash is known to
//! exist in some archive but nobody recovered its name; those rows are kept
//! because they tell you an entry is "unnamed" rather than "unknown".

use std::collections::HashMap;

use crate::hash;

/// The dictionary as shipped, embedded so the binary is self contained.
const RAW: &str = include_str!("../data/hashes_filename.txt");

pub struct Dict {
    names: HashMap<u64, &'static str>,
    unnamed: std::collections::HashSet<u64>,
}

impl Dict {
    pub fn load() -> Self {
        Self::parse(RAW)
    }

    pub fn parse(raw: &'static str) -> Self {
        let mut names = HashMap::with_capacity(200_000);
        let mut unnamed = std::collections::HashSet::new();
        for line in raw.lines() {
            let mut it = line.split('#');
            let (Some(ph), Some(sh), Some(name)) = (it.next(), it.next(), it.next()) else {
                continue;
            };
            let (Ok(ph), Ok(sh)) = (u32::from_str_radix(ph.trim(), 16), u32::from_str_radix(sh.trim(), 16)) else {
                continue;
            };
            let key = key(ph, sh);
            if name.is_empty() {
                unnamed.insert(key);
            } else {
                names.insert(key, name);
            }
        }
        Dict { names, unnamed }
    }

    /// Look a table entry up by its stored hash.
    pub fn name(&self, hash_u64: u64) -> Option<&'static str> {
        let ph = (hash_u64 >> 32) as u32;
        let sh = hash_u64 as u32;
        // The dictionary is keyed the way EasyMYP writes it (ph from the high
        // half). A handful of tools emit the halves the other way round, so
        // fall back to that instead of silently missing names.
        self.names
            .get(&key(ph, sh))
            .or_else(|| self.names.get(&key(sh, ph)))
            .copied()
    }

    /// True when the hash is listed but has no recovered name.
    pub fn is_unnamed(&self, hash_u64: u64) -> bool {
        let ph = (hash_u64 >> 32) as u32;
        let sh = hash_u64 as u32;
        self.unnamed.contains(&key(ph, sh)) || self.unnamed.contains(&key(sh, ph))
    }

    pub fn named_count(&self) -> usize {
        self.names.len()
    }

    pub fn unnamed_count(&self) -> usize {
        self.unnamed.len()
    }

    /// Sanity check: does every name hash back to its own key? Cheap to run and
    /// it catches a broken hasher immediately.
    pub fn self_check(&self) -> (usize, usize) {
        let mut ok = 0;
        let mut bad = 0;
        for (k, name) in &self.names {
            let (ph, sh) = hash::war_hash(name);
            if key(ph, sh) == *k {
                ok += 1;
            } else {
                bad += 1;
            }
        }
        (ok, bad)
    }
}

fn key(ph: u32, sh: u32) -> u64 {
    ((ph as u64) << 32) | sh as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_dictionary_is_self_consistent() {
        let d = Dict::load();
        let (ok, bad) = d.self_check();
        assert_eq!(bad, 0, "{bad} rows did not hash back");
        assert_eq!(ok, d.named_count());
    }

    #[test]
    fn known_row_resolves() {
        let d = Dict::load();
        let (ph, sh) = hash::war_hash("assetdb/charmesh/fg.0.0.ver_bull_01.geom");
        let h = ((ph as u64) << 32) | sh as u64;
        assert_eq!(
            d.name(h),
            Some("assetdb/charmesh/fg.0.0.ver_bull_01.geom")
        );
    }
}
