use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use aes_gcm::{Aes256Gcm, KeyInit, Nonce, aead::AeadInPlace};
use pbkdf2::pbkdf2_hmac;
use serde::Serialize;
use sha2::{Digest, Sha256};
use tempfile::NamedTempFile;
use zeroize::Zeroizing;

use crate::source::{invalid, open_image, same_file_state};
use crate::{catalogue_file, inventory, verify_file};

pub(crate) const MAGIC: &[u8; 8] = b"OMNIBK01";
pub(crate) const UNENCRYPTED_MAGIC: &[u8; 8] = b"OMNIBK00";
const KDF_ROUNDS: u32 = 600_000;
const BLOCK: usize = 1024 * 1024;
const MAX_FILES: usize = 20_000;
const MAX_BYTES: u64 = 64 * 1024 * 1024 * 1024;

#[derive(Debug, Serialize)]
pub struct BackupReport {
    pub format: &'static str,
    pub files: usize,
    pub source_bytes: u64,
    pub archive_bytes: u64,
}

#[derive(Clone, Copy)]
pub enum TransferProtection<'a> {
    Unencrypted,
    Passphrase(&'a str),
}

impl TransferProtection<'_> {
    pub(crate) fn validate(self) -> io::Result<()> {
        match self {
            Self::Passphrase(password) => validate_password(password),
            Self::Unencrypted => Ok(()),
        }
    }
}

/// Create a password-protected personal migration archive from an existing file export.
pub fn backup_files(directory: &Path, archive: &Path, password: &str) -> io::Result<BackupReport> {
    package_transfer(
        directory,
        archive,
        TransferProtection::Passphrase(password),
        None,
    )
}

/// Package only the approved relative paths from an existing export directory.
pub fn backup_selected_files(
    directory: &Path,
    archive: &Path,
    password: &str,
    paths: &[String],
) -> io::Result<BackupReport> {
    package_transfer(
        directory,
        archive,
        TransferProtection::Passphrase(password),
        Some(paths),
    )
}

/// Package explicitly selected computer files, preserving each basename.
pub fn backup_individual_files(
    paths: &[PathBuf],
    archive: &Path,
    password: &str,
) -> io::Result<BackupReport> {
    package_selected_computer_files(paths, archive, TransferProtection::Passphrase(password))
}

/// Package selected computer files using the requested transfer protection.
pub fn package_selected_computer_files(
    paths: &[PathBuf],
    archive: &Path,
    protection: TransferProtection<'_>,
) -> io::Result<BackupReport> {
    protection.validate()?;
    if paths.is_empty() || paths.len() > MAX_FILES {
        return Err(invalid("select 1–20,000 ordinary files"));
    }
    let staging = tempfile::Builder::new()
        .prefix("omnirecovery-picked-files-")
        .tempdir()?;
    let mut total = 0_u64;
    for path in paths {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| portable_path(name))
            .ok_or_else(|| invalid("a selected file name is incompatible with Android restore"))?;
        let expected = verify_file(path)?;
        total = total
            .checked_add(expected.length)
            .filter(|size| *size <= MAX_BYTES)
            .ok_or_else(|| invalid("migration exceeds the 64 GiB payload limit"))?;
        let destination = staging.path().join(name);
        let mut output = fs::File::create_new(&destination).map_err(|error| {
            if error.kind() == io::ErrorKind::AlreadyExists {
                invalid("two selected files have the same name")
            } else {
                error
            }
        })?;
        let mut input = open_image(path)?.take(expected.length + 1);
        if io::copy(&mut input, &mut output)? != expected.length {
            return Err(invalid("a selected file changed while copying"));
        }
        drop(output);
        if verify_file(&destination)? != expected {
            return Err(invalid("a selected file changed while copying"));
        }
    }
    package_transfer(staging.path(), archive, protection, None)
}

pub(crate) fn validate_password(password: &str) -> io::Result<()> {
    if !(12..=128).contains(&password.len())
        || !password.bytes().all(|b| (0x20..=0x7e).contains(&b))
    {
        return Err(invalid(
            "use a passphrase of 12–128 printable ASCII characters",
        ));
    }
    Ok(())
}

/// Package all files or selected relative paths from an export for phone migration.
pub fn package_transfer(
    directory: &Path,
    archive: &Path,
    protection: TransferProtection<'_>,
    paths: Option<&[String]>,
) -> io::Result<BackupReport> {
    protection.validate()?;
    let file_name = archive
        .file_name()
        .ok_or_else(|| invalid("archive must have a file name"))?;
    let parent = archive
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let parent = fs::canonicalize(parent)?;
    let source = fs::canonicalize(directory)?;
    let archive = parent.join(file_name);
    if parent.starts_with(&source) {
        return Err(invalid("archive must be outside the source directory"));
    }
    match fs::symlink_metadata(&archive) {
        Ok(_) => {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "archive already exists",
            ));
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let files = if let Some(paths) = paths {
        if paths.is_empty() || paths.len() > MAX_FILES {
            return Err(invalid("select 1–20,000 regular files"));
        }
        let mut files = BTreeMap::new();
        for path in paths {
            if !portable_path(path) {
                return Err(invalid("selected path is incompatible with phone restore"));
            }
            let resolved = catalogue_file(directory, path)?;
            if files
                .insert(path.clone(), verify_file(&resolved)?)
                .is_some()
            {
                return Err(invalid("duplicate archive selection"));
            }
        }
        files
    } else {
        let snapshot = inventory(directory)?;
        if !snapshot.skipped.is_empty() {
            return Err(invalid(
                "source contains skipped links, mounts or special files",
            ));
        }
        snapshot.files
    };
    if files.is_empty() || files.len() > MAX_FILES {
        return Err(invalid("migration requires 1–20,000 regular files"));
    }
    let mut total = 0_u64;
    let mut names = BTreeMap::new();
    for (path, entry) in &files {
        if !portable_path(path) {
            return Err(invalid(
                "source contains a path incompatible with phone restore",
            ));
        }
        for prefix in path
            .match_indices('/')
            .map(|(index, _)| &path[..index])
            .chain([path.as_str()])
        {
            if names
                .insert(prefix.to_ascii_lowercase(), prefix)
                .is_some_and(|name| name != prefix)
            {
                return Err(invalid(
                    "source contains case-colliding file or directory names",
                ));
            }
        }
        total = total
            .checked_add(entry.length)
            .filter(|total| *total <= MAX_BYTES)
            .ok_or_else(|| invalid("migration exceeds the 64 GiB payload limit"))?;
    }

    let mut temporary = NamedTempFile::new_in(&parent)?;
    {
        let mut writer = ArchiveWriter::new(temporary.as_file_mut(), protection)?;
        let mut buffer = Zeroizing::new(vec![0; BLOCK]);
        for (path, entry) in &files {
            let mut input = open_image(&catalogue_file(&source, path)?)?;
            let state = input.metadata()?;
            if state.len() != entry.length {
                return Err(invalid("source changed after inventory"));
            }
            writer.record(1, path.as_bytes())?;
            let mut digest = Sha256::new();
            let mut remaining = entry.length;
            while remaining != 0 {
                let count = remaining.min(BLOCK as u64) as usize;
                input.read_exact(&mut buffer[..count])?;
                digest.update(&buffer[..count]);
                writer.record(2, &buffer[..count])?;
                remaining -= count as u64;
            }
            let hash = digest.finalize();
            if format!("{hash:x}") != entry.sha256 || !same_file_state(&state, &input.metadata()?) {
                return Err(invalid("source changed during backup"));
            }
            let mut end = [0; 40];
            end[..8].copy_from_slice(&entry.length.to_be_bytes());
            end[8..].copy_from_slice(&hash);
            writer.record(3, &end)?;
        }
        let mut end = [0; 12];
        end[..4].copy_from_slice(&(files.len() as u32).to_be_bytes());
        end[4..].copy_from_slice(&total.to_be_bytes());
        writer.record(4, &end)?;
    }
    temporary.as_file().sync_all()?;
    let archive_bytes = temporary.as_file().metadata()?.len();
    temporary
        .persist_noclobber(archive)
        .map_err(|error| error.error)?;
    Ok(BackupReport {
        format: match protection {
            TransferProtection::Unencrypted => "OMNIBK00",
            TransferProtection::Passphrase(_) => "OMNIBK01",
        },
        files: files.len(),
        source_bytes: total,
        archive_bytes,
    })
}

fn reserved_device_name(part: &str) -> bool {
    let stem = part
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end_matches(' ')
        .to_ascii_uppercase();
    if matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) {
        return true;
    }
    stem.strip_prefix("COM")
        .or_else(|| stem.strip_prefix("LPT"))
        .is_some_and(|suffix| {
            matches!(
                suffix,
                "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
            )
        })
}

fn portable_path(path: &str) -> bool {
    path.len() <= 1024
        && path.split('/').all(|part| {
            !part.is_empty()
                && part.len() <= 255
                && !part.ends_with(['.', ' '])
                && !reserved_device_name(part)
                && !part.chars().any(|ch| {
                    ch.is_control() || matches!(ch, '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
                })
        })
}

struct ArchiveWriter<W> {
    output: W,
    cipher: Option<Aes256Gcm>,
    header: [u8; 24],
    sequence: u64,
}

impl<W: Write> ArchiveWriter<W> {
    fn new(mut output: W, protection: TransferProtection<'_>) -> io::Result<Self> {
        let mut header = [0; 24];
        let cipher = match protection {
            TransferProtection::Passphrase(password) => {
                header[..8].copy_from_slice(MAGIC);
                getrandom::fill(&mut header[8..])
                    .map_err(|_| io::Error::other("could not generate archive salt"))?;
                Some(archive_cipher(password, &header))
            }
            TransferProtection::Unencrypted => {
                header[..8].copy_from_slice(UNENCRYPTED_MAGIC);
                None
            }
        };
        output.write_all(&header)?;
        Ok(Self {
            output,
            cipher,
            header,
            sequence: 0,
        })
    }

    fn record(&mut self, kind: u8, data: &[u8]) -> io::Result<()> {
        let length = ((1 + data.len()) as u32).to_be_bytes();
        let (nonce, aad) = record_context(&self.header, self.sequence, length);
        self.sequence = self
            .sequence
            .checked_add(1)
            .ok_or_else(|| invalid("archive record limit exceeded"))?;
        let mut record = Zeroizing::new(Vec::with_capacity(1 + data.len()));
        record.push(kind);
        record.extend_from_slice(data);
        self.output.write_all(&length)?;
        if let Some(cipher) = &self.cipher {
            let tag = cipher
                .encrypt_in_place_detached(Nonce::from_slice(&nonce), &aad, &mut record)
                .map_err(|_| io::Error::other("could not encrypt archive record"))?;
            self.output.write_all(&record)?;
            self.output.write_all(&tag)
        } else {
            self.output.write_all(&record)?;
            self.output.write_all(&record_checksum(&aad, &record))
        }
    }
}

/// Restore an OmniRecovery archive without overwriting existing files.
pub fn restore_files(
    archive: &Path,
    destination: &Path,
    password: &str,
) -> io::Result<crate::AcquisitionReport> {
    use std::fs::File;

    use crate::acquisition::{finish, now_ms, stage};

    let start = now_ms();
    let mut input = open_image(archive)?;
    let state = input.metadata()?;
    let mut header = [0_u8; 24];
    input.read_exact(&mut header)?;
    let cipher = match &header[..8] {
        magic if magic == MAGIC => {
            validate_password(password)?;
            Some(archive_cipher(password, &header))
        }
        magic if magic == UNENCRYPTED_MAGIC && header[8..] == [0; 16] => {
            if !password.is_empty() {
                return Err(invalid(
                    "this archive is unencrypted; leave the passphrase blank",
                ));
            }
            None
        }
        _ => return Err(invalid("unrecognized OmniRecovery archive")),
    };
    let (staging, data) = stage(destination)?;
    let mut current: Option<(String, File, u64, Sha256)> = None;
    let mut expected = BTreeMap::new();
    let mut names = BTreeMap::new();
    let mut bytes = 0_u64;
    for sequence in 0_u64.. {
        let mut encoded_len = [0_u8; 4];
        input.read_exact(&mut encoded_len)?;
        let count = u32::from_be_bytes(encoded_len) as usize;
        if !(1..=BLOCK + 1).contains(&count) {
            return Err(invalid("invalid archive record length"));
        }
        let mut record = Zeroizing::new(vec![0_u8; count]);
        input.read_exact(&mut record)?;
        let (nonce, aad) = record_context(&header, sequence, encoded_len);
        if let Some(cipher) = &cipher {
            let mut tag = [0; 16];
            input.read_exact(&mut tag)?;
            cipher
                .decrypt_in_place_detached(
                    Nonce::from_slice(&nonce),
                    &aad,
                    &mut record,
                    aes_gcm::Tag::from_slice(&tag),
                )
                .map_err(|_| invalid("archive authentication failed"))?;
        } else {
            let mut checksum = [0; 32];
            input.read_exact(&mut checksum)?;
            if checksum != record_checksum(&aad, &record) {
                return Err(invalid("archive record checksum mismatch"));
            }
        }
        let payload = &record[1..];
        match record[0] {
            1 if current.is_none() && expected.len() < MAX_FILES => {
                let path = std::str::from_utf8(payload)
                    .ok()
                    .filter(|path| portable_path(path))
                    .ok_or_else(|| invalid("archive contains an unsafe path"))?;
                for prefix in path
                    .match_indices('/')
                    .map(|(i, _)| &path[..i])
                    .chain([path])
                {
                    if names
                        .insert(prefix.to_ascii_lowercase(), prefix.to_owned())
                        .is_some_and(|name| name != prefix)
                    {
                        return Err(invalid("archive contains case-colliding paths"));
                    }
                }
                let output = data.join(path);
                fs::create_dir_all(
                    output
                        .parent()
                        .ok_or_else(|| invalid("invalid restore path"))?,
                )?;
                current = Some((path.to_owned(), File::create_new(output)?, 0, Sha256::new()));
            }
            2 if !payload.is_empty() => {
                let (_, output, size, digest) = current
                    .as_mut()
                    .ok_or_else(|| invalid("orphan archive data record"))?;
                let incoming = payload.len() as u64;
                bytes = bytes
                    .checked_add(incoming)
                    .filter(|total| *total <= MAX_BYTES)
                    .ok_or_else(|| invalid("archive exceeds the 64 GiB restore limit"))?;
                *size += incoming;
                output.write_all(payload)?;
                digest.update(payload);
            }
            3 if payload.len() == 40 => {
                let (path, output, size, digest) = current
                    .take()
                    .ok_or_else(|| invalid("orphan archive file terminator"))?;
                let hash = digest.finalize();
                if payload[..8] != size.to_be_bytes() || payload[8..] != hash[..] {
                    return Err(invalid("archived file digest or size mismatch"));
                }
                output.sync_all()?;
                expected.insert(
                    path,
                    crate::ExportFile {
                        length: size,
                        sha256: format!("{hash:x}"),
                    },
                );
            }
            4 if current.is_none() && payload.len() == 12 && !expected.is_empty() => {
                if payload[..4] != (expected.len() as u32).to_be_bytes()
                    || payload[4..] != bytes.to_be_bytes()
                    || input.read(&mut [0; 1])? != 0
                    || !same_file_state(&state, &input.metadata()?)
                {
                    return Err(invalid("archive footer or input stability check failed"));
                }
                let files = inventory(&data)?;
                if files.files != expected {
                    return Err(invalid("restore readback verification failed"));
                }
                return finish(
                    staging,
                    files,
                    "omnibackup_restore",
                    archive.display().to_string(),
                    if cipher.is_some() {
                        "authenticated archive files"
                    } else {
                        "unencrypted archive files"
                    },
                    start,
                    if cipher.is_some() {
                        "authenticated_records_and_local_readback_sha256"
                    } else {
                        "record_checksums_and_local_readback_sha256"
                    },
                );
            }
            _ => return Err(invalid("unexpected archive record")),
        }
    }
    Err(invalid("archive sequence exhausted"))
}

fn archive_cipher(password: &str, header: &[u8; 24]) -> Aes256Gcm {
    let mut key = Zeroizing::new([0_u8; 32]);
    pbkdf2_hmac::<Sha256>(password.as_bytes(), &header[8..], KDF_ROUNDS, &mut *key);
    Aes256Gcm::new_from_slice(&*key).expect("AES-256 key length")
}

fn record_context(header: &[u8; 24], sequence: u64, length: [u8; 4]) -> ([u8; 12], [u8; 36]) {
    let sequence = sequence.to_be_bytes();
    let mut nonce = [0; 12];
    nonce[4..].copy_from_slice(&sequence);
    let mut aad = [0; 36];
    aad[..24].copy_from_slice(header);
    aad[24..32].copy_from_slice(&sequence);
    aad[32..].copy_from_slice(&length);
    (nonce, aad)
}

fn record_checksum(aad: &[u8; 36], record: &[u8]) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(aad);
    digest.update(record);
    digest.finalize().into()
}

#[cfg(test)]
mod restore_tests {
    use super::*;

    #[test]
    fn restores_authenticated_bytes_and_discards_incomplete_results() -> io::Result<()> {
        let tmp = tempfile::tempdir()?;
        let source = tmp.path().join("source");
        let restored = tmp.path().join("restored");
        fs::create_dir_all(source.join("nested"))?;
        fs::create_dir(&restored)?;
        fs::write(source.join("nested/video.mp4"), vec![7; BLOCK + 5])?;
        fs::write(source.join("empty"), [])?;
        let archive = tmp.path().join("example.omnibackup");
        let password = "Long passphrase for a synthetic test";
        backup_files(&source, &archive, password)?;
        assert!(restore_files(&archive, &restored, "another valid passphrase").is_err());
        assert_eq!(fs::read_dir(&restored)?.count(), 0);
        let result = restore_files(&archive, &restored, password)?;
        assert_eq!(result.files.files.len(), 2);
        assert_eq!(
            fs::read(result.output.join("data/nested/video.mp4"))?,
            vec![7; BLOCK + 5]
        );
        let file = fs::OpenOptions::new().write(true).open(&archive)?;
        file.set_len(file.metadata()?.len() - 3)?;
        drop(file);
        assert!(restore_files(&archive, &restored, password).is_err());
        assert_eq!(fs::read_dir(&restored)?.count(), 1);
        Ok(())
    }

    #[test]
    fn restores_unencrypted_transfer_and_rejects_changed_records() -> io::Result<()> {
        let tmp = tempfile::tempdir()?;
        let source = tmp.path().join("example.txt");
        let archive = tmp.path().join("transfer.omnibackup");
        let destination = tmp.path().join("restored");
        fs::write(&source, b"transfer fixture")?;
        fs::create_dir(&destination)?;
        assert!(backup_individual_files(std::slice::from_ref(&source), &archive, "").is_err());
        let report =
            package_selected_computer_files(&[source], &archive, TransferProtection::Unencrypted)?;
        assert_eq!(report.format, "OMNIBK00");
        assert!(restore_files(&archive, &destination, "a valid passphrase").is_err());
        let restored = restore_files(&archive, &destination, "")?;
        assert_eq!(
            fs::read(restored.output.join("data/example.txt"))?,
            b"transfer fixture"
        );
        let mut data = fs::read(&archive)?;
        data[29] ^= 1; // Change the archived filename without updating its record checksum.
        fs::write(&archive, data)?;
        assert!(restore_files(&archive, &destination, "").is_err());
        assert_eq!(fs::read_dir(destination)?.count(), 1);
        Ok(())
    }
}
