//! The WAR path hash.
//!
//! Ported from EasyMYP's `HasherFunctions/WarhammerHash.cs`, which was itself
//! reversed out of the client's `warpatch` code. Because this is a derived work
//! of a GPLv2 source, the whole crate is GPLv2.
//!
//! Two details are easy to get wrong and both were verified against the C#
//! implementation compiled with dotnet, not just against the hash list:
//!   * the seed is 0xDEADBEEF, not 0
//!   * the C# tail switch falls through case n..1, so byte j is shifted by 8*j

pub const SEED: u32 = 0xDEAD_BEEF;

/// Normalise a path the way the client hashes it: lowercase, forward slashes.
pub fn normalize(path: &str) -> String {
    path.trim_start_matches(['/', '\\'])
        .replace('\\', "/")
        .to_ascii_lowercase()
}

/// Hash a path, returning `(ph, sh)`.
pub fn war_hash(path: &str) -> (u32, u32) {
    war_hash_seed(path, SEED)
}

#[allow(unused_assignments)]
pub fn war_hash_seed(path: &str, seed: u32) -> (u32, u32) {
    let s: Vec<u32> = normalize(path).bytes().map(|b| b as u32).collect();
    let len = s.len();

    let mut edx: u32 = 0;
    let mut eax: u32 = 0;
    let mut ecx: u32 = 0;
    let mut ebx: u32 = (len as u32).wrapping_add(seed);
    let mut edi: u32 = ebx;
    let mut esi: u32 = ebx;

    let mut i = 0;
    while i + 12 < len {
        edi = ((s[i + 7] << 24) | (s[i + 6] << 16) | (s[i + 5] << 8) | s[i + 4]).wrapping_add(edi);
        esi = ((s[i + 11] << 24) | (s[i + 10] << 16) | (s[i + 9] << 8) | s[i + 8]).wrapping_add(esi);
        edx = ((s[i + 3] << 24) | (s[i + 2] << 16) | (s[i + 1] << 8) | s[i]).wrapping_sub(esi);

        edx = (edx.wrapping_add(ebx) ^ (esi >> 28) ^ (esi << 4)) as u32;
        esi = esi.wrapping_add(edi);
        edi = (edi.wrapping_sub(edx) ^ (edx >> 26) ^ (edx << 6)) as u32;
        edx = edx.wrapping_add(esi);
        esi = (esi.wrapping_sub(edi) ^ (edi >> 24) ^ (edi << 8)) as u32;
        edi = edi.wrapping_add(edx);
        ebx = (edx.wrapping_sub(esi) ^ (esi >> 16) ^ (esi << 16)) as u32;
        esi = esi.wrapping_add(edi);
        edi = (edi.wrapping_sub(ebx) ^ (ebx >> 13) ^ (ebx << 19)) as u32;
        ebx = ebx.wrapping_add(esi);
        esi = (esi.wrapping_sub(edi) ^ (edi >> 28) ^ (edi << 4)) as u32;
        edi = edi.wrapping_add(ebx);

        i += 12;
    }

    if len > i {
        for (j, b) in s[i..].iter().enumerate() {
            if j < 4 {
                ebx = ebx.wrapping_add(b << (8 * j));
            } else if j < 8 {
                edi = edi.wrapping_add(b << (8 * (j - 4)));
            } else {
                esi = esi.wrapping_add(b << (8 * (j - 8)));
            }
        }

        esi = (esi ^ edi).wrapping_sub((edi >> 18) ^ (edi << 14));
        ecx = (esi ^ ebx).wrapping_sub((esi >> 21) ^ (esi << 11));
        edi = (edi ^ ecx).wrapping_sub((ecx >> 7) ^ (ecx << 25));
        esi = (esi ^ edi).wrapping_sub((edi >> 16) ^ (edi << 16));
        edx = (esi ^ ecx).wrapping_sub((esi >> 28) ^ (esi << 4));
        edi = (edi ^ edx).wrapping_sub((edx >> 18) ^ (edx << 14));
        eax = (esi ^ edi).wrapping_sub((edi >> 8) ^ (edi << 24));
        return (edi, eax);
    }

    (esi, eax)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reference values produced by running EasyMYP's C# HashWAR with seed 0.
    #[test]
    fn matches_reference_vectors() {
        let cases: [(&str, u32, u32); 5] = [
            ("", 0x0000_0000, 0x0000_0000),
            ("a", 0xF4E8_F764, 0x9B05_E29C),
            ("abcd", 0x5BFB_B0D5, 0x86A4_FFDC),
            ("abcdefghijklm", 0xE752_BD27, 0xADF7_680E),
            ("art/textures/x.dds", 0x386F_61C0, 0xCD11_871E),
        ];
        for (s, ph, sh) in cases {
            assert_eq!(war_hash_seed(s, 0), (ph, sh), "seed 0 mismatch for {s:?}");
        }
    }

    #[test]
    fn deadbeef_seed_matches_known_dictionary_row() {
        assert_eq!(
            war_hash("assetdb/charmesh/fg.0.0.ver_bull_01.geom"),
            (0x8000_0D86, 0x4C7C_B8C4)
        );
    }

    #[test]
    fn normalize_lowercases_and_flips_slashes() {
        assert_eq!(normalize("Art\\Textures\\X.DDS"), "art/textures/x.dds");
    }
}
