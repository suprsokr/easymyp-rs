//! `myp` - Warhammer Online .myp archive tool with a bundled filename dictionary.
//!
//! Commands: ls, extract, verify, hash, dict. No daemon, no GUI, one binary.

mod dict;
mod hash;
mod myp;
mod sniff;
mod textures;

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use dict::Dict;
use myp::{Archive, Entry};
use rayon::prelude::*;

struct Cli {
    cmd: String,
    positional: Vec<String>,
    flags: HashMap<String, String>,
}

impl Cli {
    fn parse() -> Cli {
        let mut positional = Vec::new();
        let mut flags = HashMap::new();
        let mut args = std::env::args().skip(1).peekable();
        while let Some(a) = args.next() {
            if let Some(key) = a.strip_prefix('-') {
                let key = key.trim_start_matches('-').to_string();
                let value = match args.peek() {
                    Some(next) if !next.starts_with('-') => args.next().unwrap(),
                    _ => "true".to_string(),
                };
                flags.insert(key, value);
            } else {
                positional.push(a);
            }
        }
        let cmd = positional.first().cloned().unwrap_or_else(|| {
            usage();
            std::process::exit(2);
        });
        Cli {
            cmd,
            positional: positional[1..].to_vec(),
            flags,
        }
    }

    fn flag(&self, name: &str) -> Option<&str> {
        self.flags.get(name).map(|s| s.as_str())
    }

    fn has(&self, name: &str) -> bool {
        self.flags.contains_key(name)
    }

    fn number(&self, name: &str) -> Option<usize> {
        self.flag(name).and_then(|v| v.parse().ok())
    }
}

fn usage() {
    eprintln!(
        "myp - Warhammer Online archive tool

usage:
  myp ls <archive.myp>                 census: counts by file type, named vs unnamed
  myp extract <archive.myp> -o DIR     write files using real dictionary paths
      --only nif,dds                   only these extensions
      --named-only / --unnamed-only    filter by dictionary coverage
      --flat                           ignore directories, use basenames
      --limit N                        stop after N files
      --threads N                      worker threads (default: all cores)
      -v                               print each file
  myp verify <archive.myp> [--deep]    check the table, and with --deep inflate every entry
  myp textures <extract-dir> -o DIR   flat texture dir named the way models reference them
      --copy                           copy instead of hardlink
      --dry-run                        report only, write nothing
      --check <nif-dir>                resolve texture references in NIFs against the result
      --report                         list every collision and missing reference
  myp find <archive-dir> <path>...     which archive holds a path, and where it was extracted
  myp hash <path>...                   print ph/sh for paths as the client hashes them
  myp dict [--check]                   dictionary stats, --check rehashes every row"
    );
}

fn main() {
    let cli = Cli::parse();
    let code = match cli.cmd.as_str() {
        "ls" => cmd_ls(&cli),
        "extract" => cmd_extract(&cli),
        "verify" => cmd_verify(&cli),
        "hash" => cmd_hash(&cli),
        "dict" => cmd_dict(&cli),
        "find" => cmd_find(&cli),
        "textures" => cmd_textures(&cli),
        "probe" => cmd_probe(&cli),
        "help" | "-h" => {
            usage();
            0
        }
        other => {
            eprintln!("unknown command: {other}");
            usage();
            2
        }
    };
    std::process::exit(code);
}

fn open(cli: &Cli) -> Result<Archive, String> {
    let path = cli
        .positional
        .first()
        .ok_or_else(|| "missing archive path".to_string())?;
    Archive::open(path).map_err(|e| e.to_string())
}

fn cmd_ls(cli: &Cli) -> i32 {
    let archive = match open(cli) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    let dict = Dict::load();

    let mut by_ext: HashMap<&'static str, usize> = HashMap::new();
    let (mut named, mut listed_unnamed, mut unknown) = (0usize, 0usize, 0usize);
    let mut total_unc = 0u64;

    for e in &archive.entries {
        let ext = match archive.head(e, 192) {
            Ok(h) => sniff::detect(&h, true, e.method),
            Err(err) => {
                eprintln!("  read error at offset {}: {err}", e.offset);
                continue;
            }
        };
        *by_ext.entry(ext).or_default() += 1;
        total_unc += e.uncompressed_size as u64;
        if dict.name(e.hash).is_some() {
            named += 1;
        } else if dict.is_unnamed(e.hash) {
            listed_unnamed += 1;
        } else {
            unknown += 1;
        }
    }

    let total = archive.entries.len().max(1);
    println!("{}", archive.path.display());
    println!(
        "  declared {:>7}   walked {:>7}   payload {:.1} MiB",
        archive.declared_count,
        archive.entries.len(),
        total_unc as f64 / (1024.0 * 1024.0)
    );
    println!(
        "  named {named}   listed-unnamed {listed_unnamed}   unknown {unknown}   ({:.1}% named)",
        100.0 * named as f64 / total as f64
    );

    let mut rows: Vec<_> = by_ext.into_iter().collect();
    rows.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    for (ext, n) in rows {
        println!("    {ext:<10} {n:>7}");
    }
    0
}

struct Job {
    entry: Entry,
    rel: String,
    named: bool,
}

fn cmd_extract(cli: &Cli) -> i32 {
    let out = match cli.flag("o").or_else(|| cli.flag("out")) {
        Some(o) => PathBuf::from(o),
        None => {
            eprintln!("error: extract needs -o DIR");
            return 2;
        }
    };
    let archive = match open(cli) {
        Ok(a) => Arc::new(a),
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    let dict = Dict::load();

    let only: Option<Vec<String>> = cli
        .flag("only")
        .map(|v| v.split(',').map(|s| s.trim().to_ascii_lowercase()).collect());
    let named_only = cli.has("named-only");
    let unnamed_only = cli.has("unnamed-only");
    let flat = cli.has("flat");
    let verbose = cli.has("v");
    let limit = cli.number("limit").unwrap_or(usize::MAX);
    let stem = archive
        .path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "archive".to_string());

    if let Some(n) = cli.number("threads") {
        if n > 0 {
            let _ = rayon::ThreadPoolBuilder::new().num_threads(n).build_global();
        }
    }

    // Selection needs a read per entry for extension sniffing, so it stays serial.
    let mut jobs: Vec<Job> = Vec::new();
    for e in &archive.entries {
        if jobs.len() >= limit {
            break;
        }
        let name = dict.name(e.hash);
        if (named_only && name.is_none()) || (unnamed_only && name.is_some()) {
            continue;
        }
        let ext = match archive.head(e, 192) {
            Ok(h) => sniff::detect(&h, true, e.method),
            Err(_) => continue,
        };
        if let Some(only) = &only {
            if !only.iter().any(|o| o == ext) {
                continue;
            }
        }
        let rel = match name.and_then(sanitize) {
            Some(n) => n,
            None => format!("{stem}/{ext}/{}.{ext}", e.fallback_name()),
        };
        let rel = if flat { basename(&rel) } else { rel };
        jobs.push(Job {
            entry: e.clone(),
            rel,
            named: name.is_some(),
        });
    }

    if jobs.is_empty() {
        println!("nothing matched");
        return 0;
    }

    let done = AtomicUsize::new(0);
    let total_files = jobs.len();

    let results: Vec<Result<(u64, bool), (String, String)>> = jobs
        .par_iter()
        .map(|job| {
            let data = archive
                .read(&job.entry)
                .map_err(|e| (job.rel.clone(), e.to_string()))?;
            let dest = out.join(&job.rel);
            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent).map_err(|e| (job.rel.clone(), e.to_string()))?;
            }
            fs::write(&dest, &data).map_err(|e| (job.rel.clone(), e.to_string()))?;
            let n = done.fetch_add(1, Ordering::Relaxed) + 1;
            if verbose {
                println!("  [{n}/{total_files}] {}", job.rel);
            }
            Ok((data.len() as u64, job.named))
        })
        .collect();

    let (mut ok, mut bytes, mut named_ok, mut errs) = (0usize, 0u64, 0usize, 0usize);
    for r in results {
        match r {
            Ok((n, named)) => {
                ok += 1;
                bytes += n;
                named_ok += named as usize;
            }
            Err((rel, err)) => {
                eprintln!("  {rel}: {err}");
                errs += 1;
            }
        }
    }

    println!(
        "wrote {ok} file(s), {:.1} MiB, {named_ok} named, {errs} error(s) into {}",
        bytes as f64 / (1024.0 * 1024.0),
        out.display()
    );
    if errs > 0 {
        1
    } else {
        0
    }
}

fn basename(rel: &str) -> String {
    Path::new(rel)
        .file_name()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_else(|| rel.to_string())
}

/// Reject names that would escape the output directory.
fn sanitize(name: &str) -> Option<String> {
    if name.starts_with('/') || name.contains('\0') {
        return None;
    }
    if name.split('/').any(|c| c == ".." || c == ".") {
        return None;
    }
    Some(name.to_string())
}

fn cmd_verify(cli: &Cli) -> i32 {
    let deep = cli.has("deep");
    let archive = match open(cli) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    let len = match fs::metadata(&archive.path) {
        Ok(m) => m.len(),
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };

    let (mut bad_bounds, mut bad_inflate, mut size_mismatch) = (0usize, 0usize, 0usize);
    for e in &archive.entries {
        if e.offset + e.compressed_size as u64 > len {
            bad_bounds += 1;
            continue;
        }
        if deep {
            match archive.read(e) {
                Ok(d) if d.len() != e.uncompressed_size as usize => size_mismatch += 1,
                Ok(_) => {}
                Err(_) => bad_inflate += 1,
            }
        }
    }

    println!("{}", archive.path.display());
    println!(
        "  entries {}   declared {}   bounds-errors {bad_bounds}",
        archive.entries.len(),
        archive.declared_count
    );
    if deep {
        println!("  inflate-errors {bad_inflate}   size-mismatch {size_mismatch}");
    }
    if bad_bounds + bad_inflate + size_mismatch > 0 {
        1
    } else {
        0
    }
}

fn cmd_find(cli: &Cli) -> i32 {
    let dir = match cli.positional.first() {
        Some(d) => PathBuf::from(d),
        None => {
            eprintln!("error: find needs a directory of archives, then paths");
            return 2;
        }
    };
    let wanted = &cli.positional[1..];
    if wanted.is_empty() {
        eprintln!("error: find needs at least one path");
        return 2;
    }

    let mut archives: Vec<Archive> = Vec::new();
    let entries = match fs::read_dir(&dir) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("error: {}: {e}", dir.display());
            return 1;
        }
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()).map(|e| e.eq_ignore_ascii_case("myp")) != Some(true) {
            continue;
        }
        match Archive::open(&path) {
            Ok(a) => archives.push(a),
            Err(e) => eprintln!("  skip {}: {e}", path.display()),
        }
    }
    if archives.is_empty() {
        eprintln!("error: no .myp archives in {}", dir.display());
        return 1;
    }

    let mut index: HashMap<u64, Vec<(usize, usize)>> = HashMap::new();
    for (ai, a) in archives.iter().enumerate() {
        for (ei, e) in a.entries.iter().enumerate() {
            index.entry(e.hash).or_default().push((ai, ei));
        }
    }

    let dict = Dict::load();
    let mut found = 0usize;
    let mut missing = 0usize;
    for path in wanted {
        let (ph, sh) = hash::war_hash(path);
        let key = ((ph as u64) << 32) | sh as u64;
        let known = dict.name(key);
        match index.get(&key) {
            Some(hits) => {
                found += 1;
                let mut places = Vec::new();
                for (ai, ei) in hits {
                    let a = &archives[*ai];
                    let e = &a.entries[*ei];
                    let ext = a
                        .head(e, 192)
                        .map(|h| sniff::detect(&h, true, e.method))
                        .unwrap_or("bin");
                    let stem = a
                        .path
                        .file_stem()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_default();
                    places.push(format!(
                        "{} (crc {:08X}, {} bytes, {}, extracted as {}/{ext}/{}.{ext})",
                        a.path.file_name().unwrap_or_default().to_string_lossy(),
                        e.crc,
                        e.uncompressed_size,
                        ext,
                        stem,
                        e.fallback_name()
                    ));
                }
                println!("{ph:08X}#{sh:08X}  {path}");
                if known.is_some() {
                    println!("   already in dictionary");
                } else {
                    println!("   NOT in dictionary: recoverable name");
                }
                for p in places {
                    println!("   {p}");
                }
            }
            None => {
                missing += 1;
                println!("{ph:08X}#{sh:08X}  {path}");
                println!("   not in any archive given");
            }
        }
    }
    println!("\n{found} found, {missing} not found");
    0
}

fn cmd_textures(cli: &Cli) -> i32 {
    let root = match cli.positional.first() {
        Some(p) => PathBuf::from(p),
        None => {
            eprintln!("error: textures needs an extracted tree, e.g. myp textures ~/Downloads/ror-extract -o ~/Downloads/ror-extract/textures");
            return 2;
        }
    };
    if !root.is_dir() {
        eprintln!("error: {} is not a directory", root.display());
        return 2;
    }
    let dry_run = cli.has("dry-run");
    let copy = cli.has("copy");
    let report = cli.has("report");

    let out = match cli.flag("o").or_else(|| cli.flag("out")) {
        Some(o) => PathBuf::from(o),
        None => {
            eprintln!("error: textures needs -o DIR");
            return 2;
        }
    };

    let groups = match textures::collect(&root, Some(&out)) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    let scanned: usize = groups.iter().map(|g| g.sources.len()).sum();

    let mut collisions = 0usize;
    let mut differing: Vec<&textures::Group> = Vec::new();
    for g in &groups {
        if g.sources.len() > 1 {
            collisions += 1;
            let first = &g.sources[0];
            let mut any_differs = false;
            for other in &g.sources[1..] {
                match textures::same_bytes(first, other) {
                    Ok(true) => {}
                    Ok(false) => any_differs = true,
                    Err(e) => {
                        eprintln!("  compare {}: {e}", other.display());
                        any_differs = true;
                    }
                }
            }
            if any_differs {
                differing.push(g);
            }
        }
    }

    let mut written = 0usize;
    let mut linked = 0usize;
    let mut copied = 0usize;
    let mut skipped = 0usize;
    if !dry_run {
        if let Err(e) = fs::create_dir_all(&out) {
            eprintln!("error: {}: {e}", out.display());
            return 1;
        }
        for g in &groups {
            let dest = out.join(&g.name);
            if dest.exists() {
                skipped += 1;
                continue;
            }
            let src = &g.sources[0];
            if copy {
                match fs::copy(src, &dest) {
                    Ok(_) => {
                        written += 1;
                        copied += 1;
                    }
                    Err(e) => eprintln!("  copy {}: {e}", src.display()),
                }
            } else {
                match fs::hard_link(src, &dest) {
                    Ok(_) => {
                        written += 1;
                        linked += 1;
                    }
                    Err(_) => match fs::copy(src, &dest) {
                        Ok(_) => {
                            written += 1;
                            copied += 1;
                        }
                        Err(e) => eprintln!("  copy {}: {e}", src.display()),
                    },
                }
            }
        }
    }

    println!("{}", root.display());
    println!(
        "  texture files {scanned}   canonical names {}   collisions {collisions}   of those differing {}{}",
        groups.len(),
        differing.len(),
        if dry_run { "   (dry run, nothing written)" } else { "" }
    );
    if !dry_run {
        println!(
            "  wrote {written} into {} ({linked} hardlinked, {copied} copied, {skipped} already there)",
            out.display()
        );
    }

    if report && !differing.is_empty() {
        println!("\nsame name, different pixels:");
        for g in differing.iter().take(200) {
            let dirs: Vec<String> = g
                .sources
                .iter()
                .map(|p| {
                    p.parent()
                        .map(|d| d.display().to_string())
                        .unwrap_or_default()
                })
                .collect();
            println!("   {}  <- {}", g.name, dirs.join("  "));
        }
        if differing.len() > 200 {
            println!("   ... and {} more", differing.len() - 200);
        }
    }

    if let Some(nif_dir) = cli.flag("check") {
        let have: std::collections::HashSet<String> =
            groups.iter().map(|g| g.name.clone()).collect();
        let mut nifs = 0usize;
        let mut refs = 0usize;
        let mut missing: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
        let mut stack = vec![PathBuf::from(nif_dir)];
        while let Some(dir) = stack.pop() {
            let entries = match fs::read_dir(&dir) {
                Ok(e) => e,
                Err(_) => continue,
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if path.extension().and_then(|e| e.to_str()) != Some("nif") {
                    continue;
                }
                let data = match fs::read(&path) {
                    Ok(d) => d,
                    Err(_) => continue,
                };
                nifs += 1;
                for name in textures::texture_refs(&data) {
                    refs += 1;
                    if !have.contains(&name) {
                        *missing.entry(name).or_default() += 1;
                    }
                }
            }
        }
        let unresolved: usize = missing.values().sum();
        let pct = if refs > 0 {
            100.0 * (refs - unresolved) as f64 / refs as f64
        } else {
            100.0
        };
        println!(
            "\nnif check: {nifs} files, {refs} references, {} resolved ({pct:.2}%), {unresolved} unresolved",
            refs - unresolved
        );
        if report {
            for (name, n) in missing.iter().take(50) {
                println!("   missing {name} (x{n})");
            }
            if missing.len() > 50 {
                println!("   ... and {} more names", missing.len() - 50);
            }
        }
    }

    0
}

fn cmd_hash(cli: &Cli) -> i32 {
    if cli.positional.is_empty() {
        eprintln!("error: hash needs at least one path");
        return 2;
    }
    for p in &cli.positional {
        let (ph, sh) = hash::war_hash(p);
        println!("{ph:08X}#{sh:08X}#{}", hash::normalize(p));
    }
    0
}

fn cmd_probe(cli: &Cli) -> i32 {
    let archive = match open(cli) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    let needle = cli.flag("name").unwrap_or("").to_string();
    let dict = Dict::load();
    let mut shown = 0;
    for e in &archive.entries {
        let name = dict.name(e.hash).unwrap_or("-");
        if needle.is_empty() || name.contains(&needle) {
            let head = archive.head(e, 8).unwrap_or_default();
            let res = archive.read(e).map(|d| d.len());
            println!(
                "off={} hsize={} csize={} usize={} method={} head={} read={:?} {name}",
                e.offset,
                e.header_size,
                e.compressed_size,
                e.uncompressed_size,
                e.method,
                head.iter().map(|b| format!("{b:02x}")).collect::<String>(),
                res
            );
            shown += 1;
            if shown >= 5 {
                break;
            }
        }
    }
    0
}

fn cmd_dict(cli: &Cli) -> i32 {
    let dict = Dict::load();
    println!(
        "dictionary: {} named, {} listed-unnamed ({} rows)",
        dict.named_count(),
        dict.unnamed_count(),
        dict.named_count() + dict.unnamed_count()
    );
    if cli.has("check") {
        let (ok, bad) = dict.self_check();
        println!("rehash: {ok} ok, {bad} mismatched");
        if bad > 0 {
            return 1;
        }
    }
    0
}
