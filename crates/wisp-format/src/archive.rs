use std::{
    collections::{BTreeMap, BTreeSet},
    io::{Cursor, Read, Write},
};

use anyhow::{Context, Result, ensure};
use zip::{CompressionMethod, ZipWriter, read::read_zipfile_from_stream, write::SimpleFileOptions};

use crate::PackageLimits;

/// A deliberately portable subset, independent of the host's path rules.
pub(crate) fn validate_path(path: &str) -> Result<()> {
    ensure!(
        !path.is_empty() && path.len() <= 240,
        "invalid package path length: {path:?}"
    );
    for component in path.split('/') {
        ensure!(
            !component.is_empty()
                && component != "."
                && component != ".."
                && !component.ends_with('.')
                && component
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c)),
            "unsafe or nonportable package path: {path:?}"
        );
        let stem = component.split('.').next().unwrap().to_ascii_uppercase();
        ensure!(
            !matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
                && !(stem.len() == 4
                    && (stem.starts_with("COM") || stem.starts_with("LPT"))
                    && stem.as_bytes()[3].is_ascii_digit()),
            "reserved Windows path: {path:?}"
        );
    }
    Ok(())
}

fn u16_at(bytes: &[u8], offset: usize) -> usize {
    u16::from_le_bytes(bytes[offset..offset + 2].try_into().unwrap()) as usize
}
fn u32_at(bytes: &[u8], offset: usize) -> usize {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap()) as usize
}

struct Entry<'a> {
    name: &'a str,
    offset: usize,
    end: usize,
    size: u64,
}

// Parse bounded directory records ourselves, then use zip's per-file decoder.
// This avoids a general ZIP reader allocating metadata from untrusted counts.
fn preflight<'a>(bytes: &'a [u8], limits: &PackageLimits) -> Result<Vec<Entry<'a>>> {
    ensure!(bytes.len() >= 22, "truncated ZIP archive");
    let end = (bytes.len().saturating_sub(22 + 65535)..=bytes.len() - 22)
        .rev()
        .find(|&i| {
            bytes[i..i + 4] == *b"PK\x05\x06" && i + 22 + u16_at(bytes, i + 20) == bytes.len()
        })
        .context("missing ZIP end record")?;
    let count = u16_at(bytes, end + 10);
    ensure!(
        u16_at(bytes, end + 4) == 0
            && u16_at(bytes, end + 6) == 0
            && u16_at(bytes, end + 8) == count,
        "multi-disk ZIP is unsupported"
    );
    ensure!(
        count != 65535 && count <= limits.max_entries,
        "ZIP entry count exceeds limit or uses ZIP64"
    );
    let start = u32_at(bytes, end + 16);
    let size = u32_at(bytes, end + 12);
    ensure!(
        start.checked_add(size) == Some(end),
        "invalid ZIP directory bounds or unsupported ZIP64"
    );
    ensure!(
        bytes.starts_with(b"PK\x03\x04"),
        "package must start with a ZIP file header"
    );
    ensure!(
        end < 20 || bytes[end - 20..end - 16] != *b"PK\x06\x07",
        "ZIP64 is unsupported"
    );
    let mut entries = Vec::new();
    let mut pos = start;
    let mut names = BTreeSet::new();
    for _ in 0..count {
        ensure!(
            pos <= end && end - pos >= 46 && bytes[pos..pos + 4] == *b"PK\x01\x02",
            "invalid ZIP directory entry"
        );
        let name_len = u16_at(bytes, pos + 28);
        let next = pos
            .checked_add(46 + name_len + u16_at(bytes, pos + 30) + u16_at(bytes, pos + 32))
            .context("ZIP directory overflow")?;
        ensure!(next <= end, "truncated ZIP directory entry");
        let name = std::str::from_utf8(&bytes[pos + 46..pos + 46 + name_len])
            .context("ZIP path is not UTF-8")?;
        validate_path(name.strip_suffix('/').unwrap_or(name))?;
        ensure!(
            names.insert(name.trim_end_matches('/').to_ascii_lowercase()),
            "duplicate or case-colliding ZIP path: {name}"
        );
        ensure!(
            u16_at(bytes, pos + 34) == 0,
            "multi-disk ZIP entry is unsupported"
        );
        ensure!(
            u32_at(bytes, pos + 20) != u32::MAX as usize
                && u32_at(bytes, pos + 24) != u32::MAX as usize
                && u32_at(bytes, pos + 42) < start,
            "ZIP64 or invalid local header offset"
        );
        let flags = u16_at(bytes, pos + 8);
        ensure!(
            flags & !0x0806 == 0,
            "encrypted or streamed ZIP entries are unsupported: {name}"
        );
        ensure!(
            matches!(u16_at(bytes, pos + 10), 0 | 8),
            "unsupported ZIP compression: {name}"
        );
        // Ignore benign timestamp and other extra fields; reject ZIP64 overrides.
        validate_extra(&bytes[pos + 46 + name_len..pos + 46 + name_len + u16_at(bytes, pos + 30)])?;
        let mode = (u32_at(bytes, pos + 38) >> 16) & 0o170000;
        ensure!(
            mode == 0
                || mode
                    == if name.ends_with('/') {
                        0o040000
                    } else {
                        0o100000
                    },
            "non-regular ZIP entry: {name}"
        );
        let offset = u32_at(bytes, pos + 42);
        ensure!(
            start - offset >= 30 && bytes[offset..offset + 4] == *b"PK\x03\x04",
            "invalid local ZIP header: {name}"
        );
        let local_name_len = u16_at(bytes, offset + 26);
        let local_extra_len = u16_at(bytes, offset + 28);
        let data_start = offset + 30 + local_name_len + local_extra_len;
        let data_end = data_start
            .checked_add(u32_at(bytes, pos + 20))
            .context("ZIP size overflow")?;
        ensure!(data_end <= start, "ZIP entry overlaps directory: {name}");
        ensure!(
            bytes[offset + 30..offset + 30 + local_name_len] == *name.as_bytes()
                && bytes[offset + 6..offset + 10] == bytes[pos + 8..pos + 12]
                && bytes[offset + 14..offset + 26] == bytes[pos + 16..pos + 28],
            "local/central ZIP header mismatch: {name}"
        );
        validate_extra(&bytes[offset + 30 + local_name_len..data_start])?;
        let size = u32_at(bytes, pos + 24) as u64;
        ensure!(
            !name.ends_with('/') || size == 0,
            "directory has contents: {name}"
        );
        entries.push(Entry {
            name,
            offset,
            end: data_end,
            size,
        });
        pos = next;
    }
    ensure!(pos == end, "ZIP directory size/count mismatch");
    entries.sort_by_key(|entry| entry.offset);
    let mut previous_end = 0;
    for entry in &entries {
        ensure!(entry.offset >= previous_end, "overlapping ZIP entries");
        previous_end = entry.end;
    }
    Ok(entries)
}

fn validate_extra(mut bytes: &[u8]) -> Result<()> {
    while !bytes.is_empty() {
        ensure!(bytes.len() >= 4, "truncated ZIP extra field");
        let size = u16_at(bytes, 2);
        ensure!(u16_at(bytes, 0) != 1, "ZIP64 is unsupported");
        ensure!(size <= bytes.len() - 4, "truncated ZIP extra field");
        bytes = &bytes[4 + size..];
    }
    Ok(())
}

pub(crate) fn decode(bytes: &[u8], limits: &PackageLimits) -> Result<BTreeMap<String, Vec<u8>>> {
    ensure!(
        bytes.len() as u64 <= limits.max_archive_bytes,
        "package exceeds archive byte limit"
    );
    let entries = preflight(bytes, limits)?;
    let mut files = BTreeMap::new();
    let mut total = 0u64;
    for entry in entries {
        let name = entry.name;
        let limit = limits
            .max_file_bytes
            .min(limits.max_uncompressed_bytes.saturating_sub(total));
        ensure!(
            entry.size <= limit,
            "ZIP entry exceeds file or total byte limit: {name}"
        );
        let expected = entry.size;
        let mut reader = Cursor::new(&bytes[entry.offset..entry.end]);
        let mut file = read_zipfile_from_stream(&mut reader)
            .with_context(|| format!("could not open ZIP entry: {name}"))?
            .context("missing local ZIP header")?;
        let mut data = Vec::new();
        (&mut file)
            .take(limit.saturating_add(1))
            .read_to_end(&mut data)
            .with_context(|| format!("could not decode ZIP entry: {name}"))?;
        ensure!(
            data.len() as u64 <= limit && data.len() as u64 == expected,
            "ZIP entry size mismatch or byte limit exceeded: {name}"
        );
        total += data.len() as u64;
        if !name.ends_with('/') {
            files.insert(name.to_owned(), data);
        }
    }
    Ok(files)
}

pub(crate) fn encode(files: &BTreeMap<String, Vec<u8>>, limits: &PackageLimits) -> Result<Vec<u8>> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Stored)
        .unix_permissions(0o644);
    for (name, bytes) in files {
        writer.start_file(name, options)?;
        writer.write_all(bytes)?;
    }
    let bytes = writer.finish()?.into_inner();
    ensure!(
        bytes.len() as u64 <= limits.max_archive_bytes,
        "export exceeds archive byte limit"
    );
    Ok(bytes)
}
