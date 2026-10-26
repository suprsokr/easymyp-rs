# easymyp-rs

A `myp` command line tool for Warhammer Online archives, with the community filename dictionary built in. One binary, no Windows-only runtime, no GUI.

The original [EasyMYP](https://github.com/WarEmu/EasyMYP) is a .NET Framework 3.5 WinForms application. Microsoft has not shipped 3.5 since Windows 10, and it never ran on macOS or Linux, so repacking assets on a modern machine meant fighting a 2008 runtime. This does the same job from a terminal.

## What it does

```
myp ls <archive.myp>                 census: counts by type, named vs unnamed
myp extract <archive.myp> -o DIR     write files using real dictionary paths
myp verify <archive.myp> [--deep]    check the table, and with --deep inflate everything
myp find <archive-dir> <path>...     which archive holds a path, and where extract put it
myp textures <extract-dir> -o DIR    flat texture dir named the way models reference them
myp hash <path>...                   print ph/sh for a path the way the client hashes it
myp dict [--check]                   dictionary stats, --check rehashes every row
myp probe <archive.myp> --name X     dump raw entry fields for entries matching a name
```

`myp extract` writes real paths, so `assetdb/fixtures/fi.0.0.tk_maze_entrance_door.nif` lands where its name says. Entries the dictionary cannot name fall back to `<archive>/<ext>/<crc>_<hash>.<ext>`.

Options that matter: `--only nif,dds`, `--named-only`, `--unnamed-only`, `--flat`, `--limit N`, `--threads N`.

Measured on an M-series Mac against a retail install:

| job | result | time |
|---|---|---|
| `extract art2.myp` (all) | 57,030 files, 3.9 GiB, 0 errors | 9.3s |
| `extract art.myp --only nif` | 4,850 files, 165 MiB, 0 errors | 3.8s |
| `verify art3.myp --deep` | 4,248 entries, 0 mismatches | 1.8s |
| `textures` over 16,269 files | 15,744 names, plus `--check` | 8.9s |

## Dictionary coverage

Bundled from the EasyMYP hash dictionary, `Hash_v1.6.rar`, published 2009-06-28 on [the project's Google Code page](https://code.google.com/archive/p/easymyp/downloads) and mirrored since. 190,539 names, plus 16,668 hashes that are known to exist but were never named. See `data/README.md` for checksums and how the two sources line up.

| archive | named | listed-unnamed | unknown | coverage |
|---|---|---|---|---|
| world.myp | 47,754 | 1,936 | 2,019 | 92.4% |
| art.myp | 39,340 | 3,468 | 1,476 | 88.8% |
| art2.myp | 50,457 | 4,015 | 2,558 | 88.5% |
| art3.myp | 2,105 | 1,875 | 268 | 49.6% |
| data.myp | 1,848 | 3,022 | 39 | 37.6% |

Every NIF-shaped entry in the art archives resolves to a name. `data.myp` is the weak spot and will stay that way: 3,022 of its 3,061 unnamed entries are ones the original effort never cracked either, so the names are lost rather than wrong.

`myp dict --check` rehashes all 190,539 rows and asserts each one returns its own key. It runs in the test suite too.

## Textures

Extracted textures need two name changes before a viewer will find them, and `myp textures` makes both.

`.stx` files are DDS streams wearing a different extension. Every one of the 10,547 in `art`, `art2` and `art3` carries a standard 128-byte DDS header, and its mip chain accounts for the file size exactly, with no wrapper bytes anywhere. `world.myp` has not been checked yet.

Textures under `assetdb/` also carry a namespace and two version numbers in front of the name, matching the client's own `%s.%d.%d.%s` template. A NIF asks for `or_treebark01_base.dds` while the archive stores `assetdb/textures/fi.0.0.or_treebark01_base.stx`.

```
myp textures ~/ror-extract -o ~/ror-extract/textures --check ~/ror-extract/assetdb/fixtures
```

That writes a flat directory of hardlinks whose filenames are exactly what models reference, so a viewer needs one texture path and no edits to the models. On a full install it collapses 16,269 files into 15,744 names. There are 490 name collisions and 162 of them hold different pixels; the tool keeps the first source in sorted order and `--report` lists the rest.

`--copy` writes real copies instead of hardlinks, `--dry-run` reports without writing, and `--report` lists every collision and every unresolved reference by name. `--check` resolves the texture names found inside NIFs against the result. Over 7,167 fixture NIFs that is 44,805 references with 99.50% resolved. Most of the remainder are entries the 2009 dictionary never named rather than textures that do not exist, which is what `myp find` is for.

## Format notes

MYP layout, reversed by EasyMYP and documented in its `MYPInfo.txt`:

```
header   0x00 "MYP\0"
         0x0C u64  offset of first file table
         0x18 u32  declared file count (informational)
table    12 bytes: u32 entry count, u64 offset of next table
entry    34 bytes: u64 data offset, u32 header size, u32 compressed size,
                   u32 uncompressed size, u64 path hash, u32 crc32,
                   u8 compression method, u8 padding
payload  at data offset + header size; method 0 = stored, 1 = zlib
```

## Building

```
cargo build --release
cargo test          # includes the reference hash vectors and the full rehash
./target/release/myp ls /path/to/art3.myp
```

The dictionary is embedded in the binary at compile time, so no data files need to ship alongside it.

## Not implemented

- Recovering names for the 16,668 unnamed hashes. NIF texture references make a free wordlist for this: hashing `assetdb/textures/it.0.0.ch_axe_1h_t2_02_base.stx` matches a real entry in `art.myp`, so that name was recoverable all along. 22 of the first 50 unresolved texture references resolve this way. The extension is `myp crack`.
- Writing or repacking archives. EasyMYP has `ReplaceFile`; reproducing tables a live client accepts is unproven and out of scope for now.

## License and provenance

GPLv2. The crate is a derived work of EasyMYP (GPLv2), specifically its `WarhammerHash.cs`, which was itself reversed from the client's `warpatch` routine. The bundled dictionary comes from the same community effort and is distributed here on the same terms.

No game assets are included or generated by this project.
