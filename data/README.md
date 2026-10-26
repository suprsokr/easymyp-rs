# data

`hashes_filename.txt` maps a `.myp` entry's path hash to a filename. Rows are:

```
ph#sh#filename#crc
```

- `ph`, `sh`: the two 32-bit halves produced by the client's path hash
- `filename`: path relative to the client root, lowercase, forward slashes
- `crc`: CRC32 recorded for the file, `00000000` when unknown
- empty `filename`: the hash exists in some archive but no name was recovered

## Provenance

The names come from the EasyMYP hash dictionary, published on the project's
Google Code downloads page, which is now read-only:

    https://code.google.com/archive/p/easymyp/downloads

The release used here is `Hash_v1.6.rar`:

| field | value |
|---|---|
| filename | Hash_v1.6.rar |
| size | 3,470,043 bytes |
| uploaded | 2009-06-28 |
| SHA-1 | 38b33646fb04f6d2b0881cdae6c26a225e662e9c |
| release note | "EasyMyp-Hash - 98.5% on WAR 1.3.469" |

The project feed adds: "28 Jun 2009: Hash dictionary v1.6 is released, adding
WAR 1.3 filenames (after brute force analysis)." Earlier releases in the same
listing are `Hash_v1.1` through `Hash_v1.5`, from September 2008 to May 2009.

The file bundled here is not that archive verbatim. It is the expanded version
from the `Allen-K-McWood/EasyMYP-WAR` mirror, which is a series of
`Hash/oldHashList_*.txt` snapshots. Two checks tie the two together:

- The earliest snapshot has 193,468 rows, the same count as the `hashes_filename.txt`
  inside `Hash_v1.6.rar`.
- The named rows are identical: 190,539 in both, with zero names present in one
  and not the other.

So every name here traces back to v1.6, and the mirror's contribution is the
16,668 added rows that still have no name. Nothing has been renamed since 2009.

## Rebuilding and verifying

Merge an EasyMYP installation's `Hash/hashes_filename.txt` and
`Hash/oldHashList_*.txt` to regenerate this file. Verify with:

```
cargo test dict::tests::bundled_dictionary_is_self_consistent
myp dict --check
```

The names are metadata produced by a GPLv2 tool and are redistributed here under
the same license. No game content is included.
