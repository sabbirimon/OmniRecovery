use std::io::{self, Read};
use std::path::Path;
use std::time::Duration;

use clap::ValueEnum;
use serde::{Deserialize, Serialize};

use crate::acquisition::run_adapter;
use crate::android_wifi::adb_command;
use crate::phone_browse::require_android_phone;
use crate::source::{invalid, open_image};
use crate::{BackupReport, Transport, backup_files, verify_file};

const DEVICE_CHECK: Duration = Duration::from_secs(20);
const COPY_LIMIT: Duration = Duration::from_secs(8 * 60 * 60);

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum PhoneFolder {
    Camera,
    Pictures,
    Videos,
    Music,
    Documents,
    Downloads,
}

impl PhoneFolder {
    fn directory(self) -> &'static str {
        match self {
            Self::Camera => "DCIM",
            Self::Pictures => "Pictures",
            Self::Videos => "Movies",
            Self::Music => "Music",
            Self::Documents => "Documents",
            Self::Downloads => "Download",
        }
    }
}

#[derive(Debug, Serialize)]
pub struct PhoneBackupReport {
    pub backup: BackupReport,
    pub unavailable_folders: Vec<&'static str>,
}

#[derive(Debug, Serialize)]
pub struct PhoneDelivery {
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
}

/// Create an encrypted file archive from explicitly selected Android shared folders.
pub fn backup_android_folders(
    id: &str,
    folders: &[PhoneFolder],
    archive: &Path,
    password: &str,
) -> io::Result<PhoneBackupReport> {
    if folders.is_empty()
        || folders.len() > 6
        || folders
            .iter()
            .enumerate()
            .any(|(index, folder)| folders[..index].contains(folder))
    {
        return Err(invalid(
            "select at least one distinct Android shared folder",
        ));
    }
    crate::backup::validate_password(password)?;
    require_android_phone(Transport::Android, id)?;
    let staging = tempfile::Builder::new()
        .prefix("omnirecovery-phone-files-")
        .tempdir()?;
    let mut unavailable_folders = Vec::new();
    for folder in folders {
        let directory = folder.directory();
        let source = format!("/sdcard/{directory}");
        // Only the fixed folder names above are interpolated into the remote shell.
        let check = format!(
            "if [ ! -d {source} ]; then printf no; else \
             base=$(realpath /sdcard) && path=$(realpath {source}) && \
             [ \"$path\" = \"$base/{directory}\" ] && printf yes; fi"
        );
        let exists = run_adapter(
            adb_command()?.args(["-s", id, "shell", &check]),
            DEVICE_CHECK,
        )?;
        match exists.trim() {
            "no" => unavailable_folders.push(directory),
            "yes" => {
                run_adapter(
                    adb_command()?
                        .args(["-s", id, "pull", "-a", &source])
                        .arg(staging.path().join(directory)),
                    COPY_LIMIT,
                )?;
            }
            _ => {
                return Err(invalid(
                    "Android folder availability could not be confirmed",
                ));
            }
        }
    }
    if unavailable_folders.len() == folders.len() {
        return Err(invalid("none of the selected shared folders is available"));
    }
    let backup = backup_files(staging.path(), archive, password)?;
    Ok(PhoneBackupReport {
        backup,
        unavailable_folders,
    })
}

/// Send a local encrypted archive to a new directory on an authorized Android USB phone.
pub fn deliver_phone_archive(archive: &Path, id: &str) -> io::Result<PhoneDelivery> {
    if archive.extension().and_then(|value| value.to_str()) != Some("omnibackup") {
        return Err(invalid("choose a .omnibackup archive"));
    }
    let mut header = [0_u8; 8];
    open_image(archive)?.read_exact(&mut header)?;
    if &header != crate::backup::MAGIC {
        return Err(invalid("choose an OmniRecovery file archive"));
    }
    let source = verify_file(archive)?;
    require_android_phone(Transport::Android, id)?;
    let mut nonce = [0_u8; 16];
    getrandom::fill(&mut nonce).map_err(|_| io::Error::other("secure randomness unavailable"))?;
    let folder = format!("/sdcard/Download/OmniRecovery-{}", hex::encode(nonce));
    let pending = format!("{folder}/phone-transfer.part");
    let final_path = format!("{folder}/phone-transfer.omnibackup");
    let shell = |arguments: &[&str], timeout| {
        run_adapter(
            adb_command()?.args(["-s", id, "shell"]).args(arguments),
            timeout,
        )
    };
    shell(&["mkdir", &folder], DEVICE_CHECK).map_err(|_| {
        io::Error::other("could not create a transfer folder in the receiving phone's Downloads")
    })?;
    let publish = || -> io::Result<()> {
        run_adapter(
            adb_command()?
                .args(["-s", id, "push"])
                .arg(archive)
                .arg(&pending),
            COPY_LIMIT,
        )
        .map_err(|_| io::Error::other("could not copy the archive to the receiving phone"))?;
        let digest = shell(&["sha256sum", &pending], COPY_LIMIT).map_err(|_| {
            io::Error::other("the receiving phone could not verify the transferred archive")
        })?;
        if digest.split_whitespace().next() != Some(source.sha256.as_str()) {
            return Err(invalid("the receiving phone reported a different SHA-256"));
        }
        shell(&["mv", &pending, &final_path], DEVICE_CHECK)
            .map_err(|_| io::Error::other("could not publish the verified archive in Downloads"))?;
        Ok(())
    };
    if let Err(error) = publish() {
        // A disconnected target can retain the partial file. Never remove a finalized archive.
        let _ = shell(&["rm", "-f", &pending], DEVICE_CHECK);
        let _ = shell(&["rmdir", &folder], DEVICE_CHECK);
        return Err(error);
    }
    Ok(PhoneDelivery {
        path: final_path,
        bytes: source.length,
        sha256: source.sha256,
    })
}
