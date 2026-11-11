use std::io;
use std::path::Path;

#[cfg(target_os = "macos")]
pub fn save_media(
    source: &Path,
    target: &Path,
    read: impl FnOnce() -> Result<Vec<u8>, String>,
) -> Result<(), String> {
    mac::save(source, target, read)
}

#[cfg(target_os = "macos")]
pub fn copy_file(source: &Path, target: &Path) -> Result<String, String> {
    mac::copy(source, target)
}

#[cfg(not(target_os = "macos"))]
pub fn copy_file(_: &Path, _: &Path) -> Result<String, String> {
    Err("Verified separate-drive copying is available in the macOS build.".into())
}

#[cfg(not(target_os = "macos"))]
pub fn save_media(
    _: &Path,
    _: &Path,
    _: impl FnOnce() -> Result<Vec<u8>, String>,
) -> Result<(), String> {
    Err("Verified separate-drive media saving is available in the macOS build.".into())
}

pub fn save_report(target: &Path, data: &[u8]) -> io::Result<()> {
    use std::io::Write;
    let folder = target
        .parent()
        .ok_or_else(|| io::Error::other("Choose a destination folder."))?;
    let mut file = tempfile::NamedTempFile::new_in(folder)?;
    file.write_all(data)?;
    file.as_file().sync_all()?;
    file.persist_noclobber(target)
        .map_err(|error| error.error)?;
    Ok(())
}

#[cfg(target_os = "macos")]
mod mac {
    use std::collections::BTreeSet;
    use std::fs::{self, File, Metadata};
    use std::io::{Cursor, Read, Seek, SeekFrom, Write};
    use std::os::unix::fs::MetadataExt;
    use std::path::Path;
    use std::process::Command;

    use plist::{Dictionary, Value};
    use rustix::fs::{
        AtFlags, Mode, OFlags, RenameFlags, fstatfs, open, openat, renameat_with, unlinkat,
    };
    use sha2::{Digest, Sha256};

    const UNKNOWN: &str = "Could not verify the physical disk behind this location. Choose a directly attached volume.";

    pub(super) fn save(
        source: &Path,
        target: &Path,
        read: impl FnOnce() -> Result<Vec<u8>, String>,
    ) -> Result<(), String> {
        save_with(source, target, |_, file, _| {
            let bytes = read()?;
            file.write_all(&bytes).map_err(error)?;
            file.sync_all().map_err(error)?;
            file.seek(SeekFrom::Start(0)).map_err(error)?;
            let mut check = [0; 64 * 1024];
            for expected in bytes.chunks(check.len()) {
                let observed = &mut check[..expected.len()];
                file.read_exact(observed).map_err(error)?;
                if observed != expected {
                    return Err("Recovered media failed verification after writing.".into());
                }
            }
            if file.read(&mut check[..1]).map_err(error)? != 0 {
                return Err("The saved file has an unexpected length.".into());
            }
            Ok(())
        })
    }

    pub(super) fn copy(source: &Path, target: &Path) -> Result<String, String> {
        save_with(source, target, |source, output, length| {
            let mut digest = Sha256::new();
            let mut read = 0;
            let mut buffer = vec![0; 1024 * 1024];
            let mut input = source.take(length.saturating_add(1));
            loop {
                let count = input.read(&mut buffer).map_err(error)?;
                if count == 0 {
                    break;
                }
                read += count as u64;
                if read > length {
                    return Err("The source grew while it was copied.".into());
                }
                digest.update(&buffer[..count]);
                output.write_all(&buffer[..count]).map_err(error)?;
            }
            if read != length {
                return Err("The source size changed while it was copied.".into());
            }
            output.sync_all().map_err(error)?;
            output.seek(SeekFrom::Start(0)).map_err(error)?;
            let mut actual = Sha256::new();
            loop {
                let count = output.read(&mut buffer).map_err(error)?;
                if count == 0 {
                    break;
                }
                actual.update(&buffer[..count]);
            }
            let expected = digest.finalize();
            if actual.finalize() != expected {
                return Err("The output failed readback verification.".into());
            }
            Ok(format!("{expected:x}"))
        })
    }

    fn save_with<T>(
        source: &Path,
        target: &Path,
        write: impl FnOnce(&mut File, &mut File, u64) -> Result<T, String>,
    ) -> Result<T, String> {
        let mut source_file = File::from(
            open(
                source,
                OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(error)?,
        );
        let identity = source_file.metadata().map_err(error)?;
        if !identity.is_file() {
            return Err("Choose an acquired regular file.".into());
        }
        let folder = target.parent().ok_or("Choose a destination folder.")?;
        let name = target
            .file_name()
            .ok_or("Choose a destination file name.")?;
        let destination = open(
            folder,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(error)?;
        if !physical_disks(&source_file)?.is_disjoint(&physical_disks(&destination)?) {
            return Err(
                "Choose a destination on a different physical drive from the source image or file."
                    .into(),
            );
        }
        let mut random = [0; 8];
        getrandom::fill(&mut random).map_err(error)?;
        let staging = format!(".omnirecovery-{:016x}.partial", u64::from_ne_bytes(random));
        let file = openat(
            &destination,
            staging.as_str(),
            OFlags::RDWR | OFlags::CREATE | OFlags::EXCL | OFlags::CLOEXEC,
            Mode::RUSR | Mode::WUSR,
        )
        .map_err(error)?;
        let result = (|| {
            let mut file = File::from(file);
            let value = write(&mut source_file, &mut file, identity.len())?;
            if !same_file(&identity, &fs::symlink_metadata(source).map_err(error)?) {
                return Err("The source changed before the recovered file could be saved.".into());
            }
            renameat_with(
                &destination,
                staging.as_str(),
                &destination,
                name,
                RenameFlags::NOREPLACE,
            )
            .map_err(error)?;
            Ok(value)
        })();
        if result.is_err() {
            let _ = unlinkat(&destination, staging.as_str(), AtFlags::empty());
        }
        result
    }

    fn same_file(before: &Metadata, after: &Metadata) -> bool {
        after.is_file()
            && (
                before.dev(),
                before.ino(),
                before.len(),
                before.ctime(),
                before.ctime_nsec(),
            ) == (
                after.dev(),
                after.ino(),
                after.len(),
                after.ctime(),
                after.ctime_nsec(),
            )
    }

    fn physical_disks(file: impl std::os::fd::AsFd) -> Result<BTreeSet<String>, String> {
        let stats = fstatfs(file).map_err(error)?;
        let bytes: Vec<_> = stats
            .f_mntfromname
            .iter()
            .take_while(|byte| **byte != 0)
            .map(|byte| *byte as u8)
            .collect();
        let device = std::str::from_utf8(&bytes)
            .ok()
            .and_then(|path| path.strip_prefix("/dev/"))
            .ok_or(UNKNOWN)?;
        let info = disk_info(device)?;
        if let Some(stores) = info.get("APFSPhysicalStores").and_then(Value::as_array) {
            if stores.is_empty() {
                return Err(UNKNOWN.into());
            }
            return stores
                .iter()
                .map(|store| {
                    let id = store
                        .as_dictionary()
                        .and_then(|value| field(value, "APFSPhysicalStore"))
                        .ok_or(UNKNOWN)?;
                    whole_disk(id, &disk_info(id)?)
                })
                .collect();
        }
        if field(&info, "FilesystemType") == Some("apfs") {
            return Err(UNKNOWN.into());
        }
        Ok(BTreeSet::from([whole_disk(device, &info)?]))
    }

    fn whole_disk(device: &str, info: &Dictionary) -> Result<String, String> {
        if flag(info, "RAIDMaster") || flag(info, "RAIDSlice") {
            return Err(UNKNOWN.into());
        }
        let whole = if flag(info, "WholeDisk") {
            device
        } else {
            field(info, "ParentWholeDisk").ok_or(UNKNOWN)?
        };
        let physical = disk_info(whole)?;
        if field(&physical, "VirtualOrPhysical") != Some("Physical")
            || !flag(&physical, "WholeDisk")
        {
            return Err(UNKNOWN.into());
        }
        Ok(whole.to_owned())
    }

    fn disk_info(device: &str) -> Result<Dictionary, String> {
        let suffix = device.strip_prefix("disk").ok_or(UNKNOWN)?;
        if !suffix.starts_with(|ch: char| ch.is_ascii_digit())
            || !suffix
                .bytes()
                .all(|byte| byte == b's' || byte.is_ascii_digit())
        {
            return Err(UNKNOWN.into());
        }
        let output = Command::new("/usr/sbin/diskutil")
            .args(["info", "-plist", device])
            .output()
            .map_err(error)?;
        if !output.status.success() {
            return Err(UNKNOWN.into());
        }
        Value::from_reader(Cursor::new(output.stdout))
            .ok()
            .and_then(Value::into_dictionary)
            .ok_or_else(|| UNKNOWN.into())
    }

    fn field<'a>(info: &'a Dictionary, key: &str) -> Option<&'a str> {
        info.get(key).and_then(Value::as_string)
    }
    fn flag(info: &Dictionary, key: &str) -> bool {
        info.get(key).and_then(Value::as_boolean) == Some(true)
    }
    fn error(error: impl std::fmt::Display) -> String {
        error.to_string()
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn rejects_a_destination_on_the_same_physical_disk_before_reading_or_writing() {
            let folder = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
            let source = folder.path().join("source.img");
            let target = folder.path().join("recovered.png");
            fs::write(&source, b"example").unwrap();
            let reason = save(&source, &target, || {
                panic!("the source read should be gated")
            })
            .unwrap_err();
            assert!(reason.contains("different physical drive"), "{reason}");
            assert!(!target.exists());
        }
    }
}
