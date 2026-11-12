use std::io;
use std::path::Path;
use std::time::Duration;

use clap::ValueEnum;
use serde::{Deserialize, Serialize};

use crate::acquisition::run_adapter;
use crate::android_wifi::adb_command;
use crate::catalogue::classify;
use crate::source::invalid;
use crate::{ExportFile, FileCategory, Transport, detect_devices, verify_file};

const SHARED: &[&str] = &[
    "DCIM",
    "Pictures",
    "Movies",
    "Music",
    "Documents",
    "Download",
];
const RESULT_BYTES: usize = 256 * 1024;

#[derive(Clone, Copy, Debug, Deserialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum PhoneCategory {
    Photos,
    Videos,
    Music,
    Documents,
    Downloads,
    Files,
}

impl PhoneCategory {
    fn folders(self) -> &'static [&'static str] {
        match self {
            Self::Photos => &["DCIM", "Pictures", "Download"],
            Self::Videos => &["DCIM", "Pictures", "Movies", "Download"],
            Self::Music => &["Music", "Download"],
            Self::Documents => &["Documents", "Download"],
            Self::Downloads => &["Download"],
            Self::Files => SHARED,
        }
    }

    fn category(self) -> Option<FileCategory> {
        match self {
            Self::Photos => Some(FileCategory::Photo),
            Self::Videos => Some(FileCategory::Video),
            Self::Music => Some(FileCategory::Audio),
            Self::Documents => Some(FileCategory::Document),
            Self::Downloads | Self::Files => None,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct LivePhoneFile {
    pub path: String,
    pub category: FileCategory,
}

#[derive(Debug, Serialize)]
pub struct LivePhoneFiles {
    pub entries: Vec<LivePhoneFile>,
    pub partial: bool,
}

pub(crate) fn require_android_phone(transport: Transport, id: &str) -> io::Result<()> {
    if !matches!(transport, Transport::Android | Transport::AndroidWireless) {
        return Err(invalid("choose an Android USB or paired Wi-Fi connection"));
    }
    let targets = detect_devices(transport)?
        .into_iter()
        .filter(|device| device.id == id && device.state == "device")
        .count();
    if targets != 1 {
        return Err(invalid(
            "connect the selected Android phone and approve debugging",
        ));
    }
    Ok(())
}

fn allowed_path(relative: &str) -> bool {
    let Some((folder, name)) = relative.split_once('/') else {
        return false;
    };
    SHARED.contains(&folder)
        && relative.len() <= 4096
        && !relative.chars().any(|c| c.is_control() || c == '\u{fffd}')
        && name.split('/').all(|part| !matches!(part, "" | "." | ".."))
}

fn parse_phone_files(output: &str, category: PhoneCategory) -> LivePhoneFiles {
    let partial = !output.ends_with("\0OK\0") || output.len() > RESULT_BYTES;
    let complete = output
        .rsplit_once('\0')
        .map_or("", |(complete, _)| complete);
    let mut entries: Vec<_> = complete
        .split('\0')
        .filter_map(|name| {
            let path = name
                .strip_prefix("/sdcard/")
                .filter(|path| allowed_path(path))?;
            let kind = classify(path);
            category
                .category()
                .is_none_or(|required| required == kind)
                .then(|| LivePhoneFile {
                    path: path.to_owned(),
                    category: kind,
                })
        })
        .collect();
    entries.sort_unstable_by(|a, b| a.path.cmp(&b.path));
    entries.dedup_by(|a, b| a.path == b.path);
    LivePhoneFiles { entries, partial }
}

/// List metadata for readable files in standard Android shared folders.
pub fn browse_android_files(
    transport: Transport,
    id: &str,
    category: PhoneCategory,
) -> io::Result<LivePhoneFiles> {
    require_android_phone(transport, id)?;
    let roots = category
        .folders()
        .iter()
        .map(|name| format!("/sdcard/{name}"))
        .collect::<Vec<_>>()
        .join(" ");
    let script = format!(
        "{{ state=OK; for path in {roots}; do \
         if [ -d \"$path\" ]; then \
         find \"$path\" -xdev -maxdepth 9 -type f -print0 || state=PARTIAL; \
         fi; done; printf '\\000%s\\000' \"$state\"; }} | head -c {}",
        RESULT_BYTES + 1,
    );
    let output = run_adapter(
        adb_command()?.args(["-s", id, "exec-out", &script]),
        Duration::from_secs(40),
    )?;
    Ok(parse_phone_files(&output, category))
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// Export one file to a new local path and compare its digest with the phone.
pub fn export_android_file(
    transport: Transport,
    id: &str,
    relative: &str,
    destination: &Path,
) -> io::Result<ExportFile> {
    if !allowed_path(relative) {
        return Err(invalid("choose a file from a standard shared folder"));
    }
    require_android_phone(transport, id)?;
    let remote = format!("/sdcard/{relative}");
    let relative = shell_quote(relative);
    let check = format!(
        "base=$(realpath /sdcard) && file=$(realpath {}) && \
         [ \"$file\" = \"$base\"/{relative} ] && [ -f \"$file\" ] && sha256sum \"$file\"",
        shell_quote(&remote),
    );
    let parent = destination
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let staging = tempfile::NamedTempFile::new_in(parent)?.into_temp_path();
    let expected = run_adapter(
        adb_command()?.args(["-s", id, "exec-out", &check]),
        Duration::from_secs(300),
    )?;
    let expected_digest = expected
        .split_whitespace()
        .next()
        .filter(|digest| digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .ok_or_else(|| invalid("the phone file is inaccessible or outside the shared folder"))?;
    run_adapter(
        adb_command()?
            .args(["-s", id, "pull", &remote])
            .arg(&staging),
        Duration::from_secs(30 * 60),
    )?;
    let actual = verify_file(&staging)?;
    if !expected_digest.eq_ignore_ascii_case(&actual.sha256) {
        return Err(invalid("the exported file did not match its phone SHA-256"));
    }
    staging
        .persist_noclobber(destination)
        .map_err(|error| error.error)?;
    Ok(actual)
}
