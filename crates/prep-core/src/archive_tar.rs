//! Strict, streaming USTAR preflight with bounded memory usage.
//! Returns an inventory only. It never writes files or proves source integrity.
//! Callers must first verify the locked archive digest on stable bytes and must
//! still use fd-relative containment and streaming size guards to extract.
use crate::validate_archive_inventory;
use crate::{ArchiveEntry, ArchiveEntryKind, ArchiveLimits, SourceVerificationError};
use std::io::Read;

const BLOCK: usize = 512;

pub fn inspect_ustar(
    input: &mut impl Read,
    limits: ArchiveLimits,
    max_archive_bytes: u64,
) -> Result<Vec<ArchiveEntry>, SourceVerificationError> {
    let mut raw_bytes = 0u64;
    let mut expanded_total = 0u64;
    let mut entries = Vec::new();
    loop {
        let mut header = [0u8; BLOCK];
        read_block(input, &mut header, &mut raw_bytes, max_archive_bytes)?;
        if header.iter().all(|byte| *byte == 0) {
            // Require the two USTAR terminator blocks, then only zero padding.
            let mut second = [0u8; BLOCK];
            read_block(input, &mut second, &mut raw_bytes, max_archive_bytes)?;
            if second.iter().any(|byte| *byte != 0) {
                return Err(SourceVerificationError::UnsafeEntryPath);
            }
            loop {
                let mut remainder = [0u8; BLOCK];
                match input.read(&mut remainder) {
                    Ok(0) => {
                        validate_archive_inventory(&entries, limits)?;
                        return Ok(entries);
                    }
                    Ok(count) => {
                        raw_bytes = raw_bytes
                            .checked_add(count as u64)
                            .ok_or(SourceVerificationError::TooLarge)?;
                        if raw_bytes > max_archive_bytes
                            || remainder[..count].iter().any(|byte| *byte != 0)
                        {
                            return Err(SourceVerificationError::UnsafeEntryPath);
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(error) => return Err(SourceVerificationError::Io(error)),
                }
            }
        }
        if entries.len() >= limits.max_entries {
            return Err(SourceVerificationError::TooLarge);
        }
        if &header[257..263] != b"ustar\0" || &header[263..265] != b"00" {
            return Err(SourceVerificationError::UnsafeEntryPath);
        }
        let expected_checksum = parse_octal(&header[148..156])?;
        let mut sum = 0u64;
        for (index, byte) in header.iter().enumerate() {
            sum += u64::from(if (148..156).contains(&index) {
                b' '
            } else {
                *byte
            });
        }
        if sum != expected_checksum {
            return Err(SourceVerificationError::UnsafeEntryPath);
        }
        let size = parse_octal(&header[124..136])?;
        if size > limits.max_entry_bytes {
            return Err(SourceVerificationError::TooLarge);
        }
        let kind = match header[156] {
            0 | b'0' => ArchiveEntryKind::RegularFile,
            b'5' => ArchiveEntryKind::Directory,
            _ => return Err(SourceVerificationError::UnsafeEntryPath),
        };
        if kind == ArchiveEntryKind::Directory && size != 0 {
            return Err(SourceVerificationError::UnsafeEntryPath);
        }
        let name = read_name(&header[0..100])?;
        let prefix = read_name(&header[345..500])?;
        let raw_name = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        let normalized = if kind == ArchiveEntryKind::Directory {
            raw_name.strip_suffix('/').unwrap_or(&raw_name).to_owned()
        } else {
            raw_name
        };
        // Conservative until Unicode normalization/case-folding policy exists
        // for case-insensitive source filesystems.
        if !normalized.is_ascii() {
            return Err(SourceVerificationError::UnsafeEntryPath);
        }
        crate::validate_archive_entry_path(&normalized, limits.max_path_bytes)?;
        expanded_total = expanded_total
            .checked_add(size)
            .ok_or(SourceVerificationError::TooLarge)?;
        if expanded_total > limits.max_total_bytes {
            return Err(SourceVerificationError::TooLarge);
        }
        entries.push(ArchiveEntry {
            name: normalized,
            kind,
            expanded_bytes: size,
        });

        let padded = size
            .checked_add(511)
            .ok_or(SourceVerificationError::TooLarge)?
            / 512;
        for block_index in 0..padded {
            let mut data = [0u8; BLOCK];
            read_block(input, &mut data, &mut raw_bytes, max_archive_bytes)?;
            if block_index == padded - 1 && size % 512 != 0 {
                let used = (size % 512) as usize;
                if data[used..].iter().any(|byte| *byte != 0) {
                    return Err(SourceVerificationError::UnsafeEntryPath);
                }
            }
        }
    }
}

fn read_block(
    input: &mut impl Read,
    block: &mut [u8; BLOCK],
    consumed: &mut u64,
    maximum: u64,
) -> Result<(), SourceVerificationError> {
    *consumed = consumed
        .checked_add(BLOCK as u64)
        .ok_or(SourceVerificationError::TooLarge)?;
    if *consumed > maximum {
        return Err(SourceVerificationError::TooLarge);
    }
    input.read_exact(block).map_err(SourceVerificationError::Io)
}

fn read_name(raw: &[u8]) -> Result<String, SourceVerificationError> {
    let end = raw.iter().position(|byte| *byte == 0).unwrap_or(raw.len());
    if raw[end..].iter().any(|byte| *byte != 0) {
        return Err(SourceVerificationError::UnsafeEntryPath);
    }
    std::str::from_utf8(&raw[..end])
        .map(str::to_owned)
        .map_err(|_| SourceVerificationError::UnsafeEntryPath)
}

fn parse_octal(raw: &[u8]) -> Result<u64, SourceVerificationError> {
    let end = raw.iter().position(|byte| *byte == 0).unwrap_or(raw.len());
    if raw[end..].iter().any(|byte| *byte != 0 && *byte != b' ') {
        return Err(SourceVerificationError::UnsafeEntryPath);
    }
    let text =
        std::str::from_utf8(&raw[..end]).map_err(|_| SourceVerificationError::UnsafeEntryPath)?;
    let digits = text.trim_matches(' ');
    if digits.is_empty() || !digits.bytes().all(|byte| (b'0'..=b'7').contains(&byte)) {
        return Err(SourceVerificationError::UnsafeEntryPath);
    }
    u64::from_str_radix(digits, 8).map_err(|_| SourceVerificationError::TooLarge)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn archive(name: &str, kind: u8, bytes: &[u8]) -> Vec<u8> {
        let mut h = [0u8; BLOCK];
        h[0..name.len()].copy_from_slice(name.as_bytes());
        h[124..136].copy_from_slice(format!("{:011o}\0", bytes.len()).as_bytes());
        h[156] = kind;
        h[257..263].copy_from_slice(b"ustar\0");
        h[263..265].copy_from_slice(b"00");
        let checksum: u64 = h
            .iter()
            .enumerate()
            .map(|(i, b)| u64::from(if (148..156).contains(&i) { b' ' } else { *b }))
            .sum();
        h[148..156].copy_from_slice(format!("{checksum:06o}\0 ").as_bytes());
        let mut out = h.to_vec();
        out.extend_from_slice(bytes);
        out.resize(BLOCK + bytes.len().div_ceil(BLOCK) * BLOCK, 0);
        out.extend_from_slice(&[0; BLOCK * 2]);
        out
    }

    #[test]
    fn accepts_conservative_ustar_and_enforces_byte_limits() {
        let data = archive("src/readme.txt", b'0', b"hello");
        let entries = inspect_ustar(
            &mut Cursor::new(&data),
            ArchiveLimits::default(),
            data.len() as u64,
        )
        .unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "src/readme.txt");
        assert_eq!(entries[0].expanded_bytes, 5);
        assert!(
            inspect_ustar(
                &mut Cursor::new(&data),
                ArchiveLimits::default(),
                (data.len() - 1) as u64
            )
            .is_err()
        );
    }

    #[test]
    fn rejects_traversal_link_types_and_bad_checksums() {
        for name in ["../escape", "/abs", "src/../escape"] {
            let data = archive(name, b'0', b"a");
            assert!(inspect_ustar(&mut Cursor::new(data), ArchiveLimits::default(), 4096).is_err());
        }
        let link = archive("link", b'2', b"");
        assert!(inspect_ustar(&mut Cursor::new(link), ArchiveLimits::default(), 4096).is_err());
        let mut bad = archive("valid", b'0', b"a");
        bad[15] ^= 1;
        assert!(inspect_ustar(&mut Cursor::new(bad), ArchiveLimits::default(), 4096).is_err());
    }

    #[test]
    fn trailer_retries_transient_interrupted_reads() {
        struct InterruptedOnce {
            inner: Cursor<Vec<u8>>,
            reads: usize,
        }
        impl Read for InterruptedOnce {
            fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
                if self.reads == 4 {
                    self.reads += 1;
                    return Err(std::io::ErrorKind::Interrupted.into());
                }
                self.reads += 1;
                self.inner.read(buffer)
            }
        }
        let archive = archive("file", b'0', b"data");
        let mut interrupted = InterruptedOnce {
            inner: Cursor::new(archive),
            reads: 0,
        };
        assert_eq!(
            inspect_ustar(&mut interrupted, ArchiveLimits::default(), 4096)
                .expect("interrupted trailer should retry")
                .len(),
            1
        );
    }

    #[test]
    fn rejects_truncated_and_extended_archive() {
        let mut data = archive("file", b'0', b"abc");
        data.truncate(data.len() - 512);
        assert!(inspect_ustar(&mut Cursor::new(data), ArchiveLimits::default(), 4096).is_err());
        let mut data = archive("file", b'0', b"abc");
        data.extend_from_slice(b"malicious");
        assert!(inspect_ustar(&mut Cursor::new(data), ArchiveLimits::default(), 4096).is_err());
    }
}
