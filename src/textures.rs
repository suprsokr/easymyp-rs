//! Texture name fixups for extracted trees.
//!
//! Two naming rules stand between an extracted archive tree and a viewer that
//! can find the textures a model asks for:
//!
//! * The asset pipeline labels DDS files `.stx`. Every one is a plain DDS
//!   stream, header and mip chain, with nothing wrapped around it.
//! * Textures under `assetdb/` carry a namespace and two version numbers in
//!   front of the name, matching the client's own `%s.%d.%d.%s` template, so a
//!   NIF asking for `or_treebark01_base.dds` needs the file stored as
//!   `fi.0.0.or_treebark01_base.stx`.
//!
//! Canonicalising both gives a flat directory whose filenames are exactly what
//! NIFs reference, which turns "find the texture" into one search path.

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

/// Map one on-disk filename to the name a model refers to.
pub fn canonical(base: &str) -> String {
    let lower = base.to_ascii_lowercase();
    let stripped = strip_namespace(&lower);
    match stripped.strip_suffix(".stx") {
        Some(stem) => format!("{stem}.dds"),
        None => stripped.to_string(),
    }
}

/// Drop a leading `xx.0.0.` namespace. Anything else is left alone.
fn strip_namespace(s: &str) -> &str {
    let b = s.as_bytes();
    if b.len() < 8 || !b[0].is_ascii_alphabetic() || !b[1].is_ascii_alphabetic() || b[2] != b'.' {
        return s;
    }
    let mut i = 3;
    let first = i;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    if i == first || b.get(i) != Some(&b'.') {
        return s;
    }
    i += 1;
    let second = i;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    if i == second || b.get(i) != Some(&b'.') {
        return s;
    }
    // The client's template puts a real name after the versions, so an empty
    // or extension-only remainder means this was not a namespace after all.
    let rest = &s[i + 1..];
    if rest.is_empty() || rest.starts_with('.') {
        s
    } else {
        rest
    }
}

pub fn is_texture(path: &Path) -> bool {
    match path.extension().and_then(|e| e.to_str()) {
        Some(ext) => {
            let e = ext.to_ascii_lowercase();
            e == "stx" || e == "dds"
        }
        None => false,
    }
}

pub struct Group {
    pub name: String,
    pub sources: Vec<PathBuf>,
}

/// Walk `root`, group every texture by its canonical name. Sources are sorted,
/// so the first one is stable across runs. `exclude` skips a subtree, which
/// matters because the usual output directory lives inside the extract tree and
/// would otherwise be counted as source files on the next run.
pub fn collect(root: &Path, exclude: Option<&Path>) -> io::Result<Vec<Group>> {
    let mut files = Vec::new();
    let skip = exclude.map(|p| fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf()));
    walk(root, &mut files, skip.as_deref())?;
    files.sort();

    let mut map: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
    for path in files {
        let base = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        if base.is_empty() {
            continue;
        }
        map.entry(canonical(&base)).or_default().push(path);
    }
    Ok(map
        .into_iter()
        .map(|(name, sources)| Group { name, sources })
        .collect())
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>, skip: Option<&Path>) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let ft = entry.file_type()?;
        if ft.is_dir() {
            if let Some(skip) = skip {
                if fs::canonicalize(&path).map(|p| p == skip).unwrap_or(false) {
                    continue;
                }
            }
            walk(&path, out, skip)?;
        } else if ft.is_file() && is_texture(&path) {
            out.push(path);
        }
    }
    Ok(())
}

/// Compare two files byte for byte. Used only on collisions.
pub fn same_bytes(a: &Path, b: &Path) -> io::Result<bool> {
    if fs::metadata(a)?.len() != fs::metadata(b)?.len() {
        return Ok(false);
    }
    let mut fa = fs::File::open(a)?;
    let mut fb = fs::File::open(b)?;
    let mut ba = [0u8; 64 * 1024];
    let mut bb = [0u8; 64 * 1024];
    loop {
        let na = read_full(&mut fa, &mut ba)?;
        let nb = read_full(&mut fb, &mut bb)?;
        if na != nb || ba[..na] != bb[..nb] {
            return Ok(false);
        }
        if na == 0 {
            return Ok(true);
        }
    }
}

fn read_full(file: &mut fs::File, buf: &mut [u8]) -> io::Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        let n = file.read(&mut buf[filled..])?;
        if n == 0 {
            break;
        }
        filled += n;
    }
    Ok(filled)
}

/// Texture filenames referenced inside a NIF or any other asset blob.
pub fn texture_refs(data: &[u8]) -> Vec<String> {
    const NEEDLE: &[u8] = b".dds";
    let mut out = Vec::new();
    let mut i = 0;
    while i + NEEDLE.len() <= data.len() {
        if data[i..i + NEEDLE.len()].eq_ignore_ascii_case(NEEDLE) {
            let mut start = i;
            while start > 0 && is_name_byte(data[start - 1]) {
                start -= 1;
            }
            if start < i {
                out.push(
                    String::from_utf8_lossy(&data[start..i + NEEDLE.len()]).to_ascii_lowercase(),
                );
            }
            i += NEEDLE.len();
        } else {
            i += 1;
        }
    }
    out.sort();
    out.dedup();
    out
}

fn is_name_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_names_match_model_references() {
        let cases = [
            ("fi.0.0.or_treebark01_base.stx", "or_treebark01_base.dds"),
            ("it.0.0.some_item.stx", "some_item.dds"),
            ("sk.0.0.skybox01.stx", "skybox01.dds"),
            ("or.1.2.thing.stx", "thing.dds"),
            ("em_reik_rockmoss.dds", "em_reik_rockmoss.dds"),
            ("VC_GARDEN_FLOOR.DDS", "vc_garden_floor.dds"),
            ("fi.0.0.uppercase.STX", "uppercase.dds"),
        ];
        for (input, want) in cases {
            assert_eq!(canonical(input), want, "input {input}");
        }
    }

    #[test]
    fn only_two_letter_namespaces_with_two_versions_are_stripped() {
        // not a namespace: three letters, no version digits, or a missing dot
        assert_eq!(canonical("abc.0.0.x.stx"), "abc.0.0.x.dds");
        assert_eq!(canonical("ab.x.0.y.stx"), "ab.x.0.y.dds");
        assert_eq!(canonical("ab.0.0stx.stx"), "ab.0.0stx.dds");
        assert_eq!(canonical("fi.0.0..stx"), "fi.0.0..dds");
    }

    #[test]
    fn refs_are_pulled_out_of_binary_blobs() {
        let blob = b"\x04\x00or_treebark01_base.dds\x00vc_garden_floor.DDS\x00junk.dds";
        let refs = texture_refs(blob);
        assert!(refs.contains(&"or_treebark01_base.dds".to_string()));
        assert!(refs.contains(&"vc_garden_floor.dds".to_string()));
        assert!(refs.contains(&"junk.dds".to_string()));
        assert_eq!(refs.len(), 3);
    }
}
