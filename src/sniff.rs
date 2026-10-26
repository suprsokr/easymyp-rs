//! Best-effort file type detection from the first bytes of an entry.
//!
//! Only used to pick an extension for entries the dictionary cannot name, so it
//! stays deliberately small. `.kf` and `.kfm` sniff as nif because they share
//! the Gamebryo header; the dictionary supplies the real extension when it
//! knows one.

use flate2::read::ZlibDecoder;
use std::io::Read;

/// Signatures in match order. `.stx` is WAR's packed texture container and is
/// DDS inside, so dds wins for anything starting with the DDS magic.
const SIGS: &[(&[u8], &str)] = &[
    (b"NetImmerse File Format", "nif"),
    (b"Gamebryo File Format", "nif"),
    (b"Gamebryo KFM File Version", "kfm"),
    (b"DDS ", "dds"),
    (b"RIFF", "riff"),
    (b"XAC", "xac"),
    (b"XSM", "xsm"),
    (b"bdLF", "db"),
    (b"gsLF", "geom"),
    (b"idLF", "diffuse"),
    (b"psLF", "specular"),
    (b"amLF", "mask"),
    (b"ntLF", "tint"),
    (b"lgLF", "glow"),
    (b"8BPS", "8bps"),
    (b"PK\x03\x04", "zip"),
    (b"\x89PNG", "png"),
];

pub fn detect(head: &[u8], compressed: bool, method: u8) -> &'static str {
    let plain: Vec<u8> = if method == 1 && compressed {
        let mut buf = Vec::with_capacity(4096);
        let dec = ZlibDecoder::new(head);
        let _ = dec.take(4096).read_to_end(&mut buf);
        buf
    } else {
        head.to_vec()
    };

    for (sig, ext) in SIGS {
        if plain.starts_with(sig) {
            return ext;
        }
    }
    "bin"
}
