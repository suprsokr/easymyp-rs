//! Reader for Warhammer Online `.myp` archives.
//!
//! Layout, as documented by EasyMYP's `MYPInfo.txt` and its C# reader:
//!
//! ```text
//! header   0x00 "MYP\0"
//!          0x0C u64  offset of the first file table
//!          0x18 u32  declared file count (informational only)
//! table    12 bytes: u32 entry count, u64 offset of the next table
//! entry    34 bytes: u64 data offset (before header)
//!                    u32 per-file header size
//!                    u32 compressed size
//!                    u32 uncompressed size
//!                    u64 path hash
//!                    u32 crc32
//!                    u8  compression method, u8 padding
//! payload  at data offset + header size, method 0 = stored, 1 = zlib
//! ```

use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use flate2::read::{DeflateDecoder, ZlibDecoder};

const TABLE_HEADER: usize = 12;
const ENTRY: usize = 34;

#[derive(Debug, Clone)]
pub struct Entry {
    /// Absolute offset of the payload (data offset plus per-file header).
    pub offset: u64,
    #[allow(dead_code)]
    pub header_size: u32,
    pub compressed_size: u32,
    pub uncompressed_size: u32,
    pub hash: u64,
    pub crc: u32,
    pub method: u8,
}

impl Entry {
    /// The name EasyMYP invents when it has no dictionary hit.
    pub fn fallback_name(&self) -> String {
        format!("{:08X}_{:016X}", self.crc, self.hash)
    }
}

pub struct Archive {
    pub path: PathBuf,
    pub declared_count: u32,
    pub entries: Vec<Entry>,
    file: File,
}

impl Archive {
    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        let file = File::open(&path)?;
        let len = file.metadata()?.len();

        let mut header = [0u8; 40];
        read_exact_at(&file, &mut header, 0)?;
        if &header[0..4] != b"MYP\0" {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{}: not a myp archive", path.display()),
            ));
        }

        let first_table = u64::from_le_bytes(header[12..20].try_into().unwrap());
        let declared_count = u32::from_le_bytes(header[24..28].try_into().unwrap());

        let mut entries = Vec::with_capacity(declared_count as usize);
        let mut table = first_table;
        let mut seen = std::collections::HashSet::new();
        while table != 0 && seen.insert(table) {
            if table + TABLE_HEADER as u64 > len {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("{}: file table at {table} runs past end of file", path.display()),
                ));
            }
            let mut thead = [0u8; TABLE_HEADER];
            read_exact_at(&file, &mut thead, table)?;
            let count = u32::from_le_bytes(thead[0..4].try_into().unwrap()) as usize;
            let next = u64::from_le_bytes(thead[4..12].try_into().unwrap());

            let body_len = count * ENTRY;
            let mut body = vec![0u8; body_len];
            if table + TABLE_HEADER as u64 + body_len as u64 > len {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("{}: file table at {table} is truncated", path.display()),
                ));
            }
            read_exact_at(&file, &mut body, table + TABLE_HEADER as u64)?;

            for chunk in body.chunks_exact(ENTRY) {
                let start = u64::from_le_bytes(chunk[0..8].try_into().unwrap());
                let header_size = u32::from_le_bytes(chunk[8..12].try_into().unwrap());
                let compressed_size = u32::from_le_bytes(chunk[12..16].try_into().unwrap());
                let uncompressed_size = u32::from_le_bytes(chunk[16..20].try_into().unwrap());
                let hash = u64::from_le_bytes(chunk[20..28].try_into().unwrap());
                let crc = u32::from_le_bytes(chunk[28..32].try_into().unwrap());
                let method = chunk[32];

                // Slots with any zero size are unused padding in the table.
                if start == 0 || compressed_size == 0 || uncompressed_size == 0 {
                    continue;
                }

                entries.push(Entry {
                    offset: start + header_size as u64,
                    header_size,
                    compressed_size,
                    uncompressed_size,
                    hash,
                    crc,
                    method,
                });
            }
            table = next;
        }

        Ok(Archive {
            path,
            declared_count,
            entries,
            file,
        })
    }

    /// Read up to `n` bytes of an entry's payload without decompressing.
    pub fn head(&self, e: &Entry, n: usize) -> io::Result<Vec<u8>> {
        let n = n.min(e.compressed_size as usize);
        let mut buf = vec![0u8; n];
        read_exact_at(&self.file, &mut buf, e.offset)?;
        Ok(buf)
    }

    pub fn read(&self, e: &Entry) -> io::Result<Vec<u8>> {
        let mut raw = vec![0u8; e.compressed_size as usize];
        read_exact_at(&self.file, &mut raw, e.offset)?;
        if e.method == 0 {
            return Ok(raw);
        }

        let expect = e.uncompressed_size as usize;

        // Strict zlib first, so a genuinely broken entry is still visible.
        if let Ok(out) = inflate(&raw, true) {
            if out.len() == expect {
                return Ok(out);
            }
        }
        // Retail archives contain entries whose deflate stream ends without the
        // final block marker or with an adler32 that never lands inside
        // compressed_size. EasyMYP reads these with SharpZipLib's raw Inflater,
        // which ignores the trailer, so do the same and validate the length
        // instead of trusting the checksum.
        if raw.len() > 2 {
            if let Ok(out) = inflate(&raw[2..], false) {
                if out.len() == expect {
                    return Ok(out);
                }
            }
        }

        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "cannot inflate entry at {} ({} compressed, {} expected)",
                e.offset, e.compressed_size, e.uncompressed_size
            ),
        ))
    }

}

/// Inflate a buffer, keeping whatever came out before the stream ended.
///
/// Returns the bytes decoded so far; the caller checks the length against the
/// size in the file table. `zlib_header` picks zlib framing versus raw deflate.
fn inflate(raw: &[u8], zlib_header: bool) -> io::Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut buf = [0u8; 64 * 1024];
    let mut dec: Box<dyn Read> = if zlib_header {
        Box::new(ZlibDecoder::new(raw))
    } else {
        Box::new(DeflateDecoder::new(raw))
    };
    loop {
        match dec.read(&mut buf) {
            Ok(0) => return Ok(out),
            Ok(n) => out.extend_from_slice(&buf[..n]),
            // Truncated trailer: keep the output, the caller validates the size.
            Err(_) => return Ok(out),
        }
    }
}

/// Positioned read, portable across unix and windows.
pub fn read_exact_at(file: &File, buf: &mut [u8], mut offset: u64) -> io::Result<()> {
    let mut filled = 0;
    while filled < buf.len() {
        let n = pread(file, &mut buf[filled..], offset)?;
        if n == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "short read in archive",
            ));
        }
        filled += n;
        offset += n as u64;
    }
    Ok(())
}

#[cfg(unix)]
fn pread(file: &File, buf: &mut [u8], offset: u64) -> io::Result<usize> {
    use std::os::unix::fs::FileExt;
    file.read_at(buf, offset)
}

#[cfg(windows)]
fn pread(file: &File, buf: &mut [u8], offset: u64) -> io::Result<usize> {
    use std::os::windows::fs::FileExt;
    file.seek_read(buf, offset)
}
