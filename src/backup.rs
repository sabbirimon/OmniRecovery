use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Read, Write};
use std::path::Path;

use aes_gcm::{Aes256Gcm, KeyInit, Nonce, aead::AeadInPlace};
use pbkdf2::pbkdf2_hmac;
use serde::Serialize;
use sha2::{Digest, Sha256};
use tempfile::NamedTempFile;
use zeroize::Zeroizing;

use crate::source::{invalid, open_image, same_file_state};
use crate::{catalogue_file, inventory, verify_file};

const MAGIC: &[u8; 8] = b"OMNIBK01";
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

/// Create a password-protected personal migration archive from an existing file export.
pub fn backup_files(directory: &Path, archive: &Path, password: &str) -> io::Result<BackupReport> {
    make_backup(directory, archive, password, None)
}

/// Package only the approved relative paths from an existing export directory.
pub fn backup_selected_files(
    directory: &Path,
    archive: &Path,
    password: &str,
    paths: &[String],
) -> io::Result<BackupReport> {
    make_backup(directory, archive, password, Some(paths))
}

fn make_backup(
    directory: &Path,
    archive: &Path,
    password: &str,
    paths: Option<&[String]>,
) -> io::Result<BackupReport> {
    if !(12..=128).contains(&password.len())
        || !password.bytes().all(|b| (0x20..=0x7e).contains(&b))
    {
        return Err(invalid(
            "use a passphrase of 12–128 printable ASCII characters",
        ));
    }
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
        let mut writer = ArchiveWriter::new(temporary.as_file_mut(), password)?;
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
        format: "OMNIBK01",
        files: files.len(),
        source_bytes: total,
        archive_bytes,
    })
}

fn portable_path(path: &str) -> bool {
    path.len() <= 1024
        && path.split('/').all(|part| {
            !part.is_empty()
                && part.len() <= 255
                && !part.ends_with(['.', ' '])
                && !part.chars().any(|ch| {
                    ch.is_control() || matches!(ch, '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
                })
        })
}

struct ArchiveWriter<W> {
    output: W,
    cipher: Aes256Gcm,
    header: [u8; 24],
    sequence: u64,
}

impl<W: Write> ArchiveWriter<W> {
    fn new(mut output: W, password: &str) -> io::Result<Self> {
        let mut header = [0; 24];
        header[..8].copy_from_slice(MAGIC);
        getrandom::fill(&mut header[8..])
            .map_err(|_| io::Error::other("could not generate archive salt"))?;
        let mut key = Zeroizing::new([0_u8; 32]);
        pbkdf2_hmac::<Sha256>(password.as_bytes(), &header[8..], KDF_ROUNDS, &mut *key);
        let cipher = Aes256Gcm::new_from_slice(&*key).expect("AES-256 key length");
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
        let sequence = self.sequence.to_be_bytes();
        self.sequence = self
            .sequence
            .checked_add(1)
            .ok_or_else(|| invalid("archive record limit exceeded"))?;
        let mut nonce = [0; 12];
        nonce[4..].copy_from_slice(&sequence);
        let mut aad = [0; 36];
        aad[..24].copy_from_slice(&self.header);
        aad[24..32].copy_from_slice(&sequence);
        aad[32..].copy_from_slice(&length);
        let mut record = Zeroizing::new(Vec::with_capacity(1 + data.len()));
        record.push(kind);
        record.extend_from_slice(data);
        let tag = self
            .cipher
            .encrypt_in_place_detached(Nonce::from_slice(&nonce), &aad, &mut record)
            .map_err(|_| io::Error::other("could not encrypt archive record"))?;
        self.output.write_all(&length)?;
        self.output.write_all(&record)?;
        self.output.write_all(&tag)
    }
}
