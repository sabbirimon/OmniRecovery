use std::fs::{self, File};
use std::io::{self, Read, Seek, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use clap::ValueEnum;
use serde::Serialize;
use sha2::{Digest, Sha256};
use tempfile::{Builder, TempDir};

use crate::source::{invalid, open_image, same_file_state};
use crate::{ExportInventory, inventory};

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Deserialize, Serialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum Transport {
    Android,
    AndroidWireless,
    Ios,
    Camera,
}

#[derive(Clone, Copy, Debug, Default, serde::Deserialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum AndroidScope {
    #[default]
    Shared,
    Camera,
    Downloads,
    Documents,
    Pictures,
    Music,
    Movies,
    DeviceInfo,
}

impl AndroidScope {
    fn path(self) -> &'static str {
        match self {
            Self::Shared => "/sdcard/",
            Self::Camera => "/sdcard/DCIM",
            Self::Downloads => "/sdcard/Download",
            Self::Documents => "/sdcard/Documents",
            Self::Pictures => "/sdcard/Pictures",
            Self::Music => "/sdcard/Music",
            Self::Movies => "/sdcard/Movies",
            Self::DeviceInfo => "device model and OS properties",
        }
    }
}

#[derive(Debug, Serialize)]
pub struct DetectedDevice {
    pub id: String,
    pub state: String,
    pub model: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AcquisitionReport {
    pub schema_version: u32,
    pub engine_version: &'static str,
    pub acquisition: &'static str,
    pub source: String,
    pub scope: &'static str,
    pub started_unix_ms: u64,
    pub completed_unix_ms: u64,
    pub bytes: u64,
    pub output: PathBuf,
    pub verification: &'static str,
    pub files: ExportInventory,
}

pub(crate) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis() as u64)
}

pub(crate) fn find_helper(program: &str) -> io::Result<PathBuf> {
    let filename = if cfg!(windows) {
        format!("{program}.exe")
    } else {
        program.to_owned()
    };
    let mut directories: Vec<PathBuf> = std::env::var_os("PATH")
        .as_deref()
        .map(std::env::split_paths)
        .into_iter()
        .flatten()
        .collect();
    if cfg!(target_os = "macos") {
        directories.extend(
            [
                "/usr/local/bin",
                "/opt/homebrew/bin",
                "/opt/local/bin",
                "/usr/bin",
            ]
            .map(PathBuf::from),
        );
    }
    if program == "scrcpy"
        && cfg!(target_os = "macos")
        && let Some(home) = std::env::var_os("HOME")
    {
        directories.push(PathBuf::from(home).join("Library/Caches/OmniRecovery/tools/scrcpy"));
    }
    if matches!(program, "adb" | "fastboot") {
        for variable in ["ANDROID_HOME", "ANDROID_SDK_ROOT"] {
            if let Some(path) = std::env::var_os(variable) {
                directories.push(PathBuf::from(path).join("platform-tools"));
            }
        }
    }
    directories
        .into_iter()
        .filter(|path| path.is_absolute())
        .find_map(|directory| {
            let path = fs::canonicalize(directory.join(&filename)).ok()?;
            path.is_file().then_some(path)
        })
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("Install the required helper {program} and add it to PATH."),
            )
        })
}

pub(crate) fn adapter_command(program: &str) -> io::Result<Command> {
    let mut command = Command::new(find_helper(program)?);
    command
        .env_remove("ADB_SERVER_SOCKET")
        .env_remove("ANDROID_ADB_SERVER_ADDRESS")
        .env_remove("ANDROID_ADB_SERVER_PORT")
        .env_remove("ADB_SERVER_PORT")
        .env_remove("ADB_LIBUSB")
        .env_remove("USBMUXD_SOCKET_ADDRESS")
        .env_remove("BACKUP_PASSWORD")
        .env("LC_ALL", "C");
    Ok(command)
}

pub(crate) fn run_adapter(command: &mut Command, timeout: Duration) -> io::Result<String> {
    run_adapter_input(command, timeout, None)
}

pub(crate) fn run_adapter_input(
    command: &mut Command,
    timeout: Duration,
    input: Option<&[u8]>,
) -> io::Result<String> {
    let mut output = tempfile::tempfile()?;
    let mut child = command
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::from(output.try_clone()?))
        .stderr(Stdio::null())
        .spawn()?;
    if let Some(input) = input {
        let write_result = child
            .stdin
            .take()
            .ok_or_else(|| io::Error::other("helper input unavailable"))
            .and_then(|mut pipe| pipe.write_all(input));
        if let Err(error) = write_result {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
    }
    let start = Instant::now();
    let result = loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break Ok(()),
            Ok(Some(_)) => {
                return Err(invalid(
                    "transport helper failed; check device authorization and connectivity",
                ));
            }
            Ok(None)
                if start.elapsed() < timeout
                    && output.metadata().is_ok_and(|m| m.len() <= 1024 * 1024) =>
            {
                std::thread::sleep(Duration::from_millis(50));
            }
            Ok(None) => {
                break Err(invalid(
                    "transport helper exceeded its time or output limit",
                ));
            }
            Err(error) => break Err(error),
        }
    };
    if result.is_err() {
        let _ = child.kill();
        let _ = child.wait();
    }
    result?;
    output.rewind()?;
    let mut bytes = Vec::new();
    output.take(1024 * 1024 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > 1024 * 1024 {
        return Err(invalid("transport helper returned too much output"));
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

pub(crate) fn usb_serial(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id.as_bytes()[0].is_ascii_alphanumeric()
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

fn camera_port(id: &str) -> bool {
    id.strip_prefix("usb:")
        .and_then(|port| port.split_once(','))
        .is_some_and(|(bus, dev)| {
            [bus, dev]
                .iter()
                .all(|s| s.len() == 3 && s.bytes().all(|b| b.is_ascii_digit()))
        })
}

fn parse_devices(transport: Transport, output: &str) -> Vec<DetectedDevice> {
    output
        .lines()
        .filter_map(|line| match transport {
            Transport::Android => {
                let mut fields = line.split_whitespace();
                let id = fields.next()?;
                let state = fields.next()?;
                if !usb_serial(id)
                    || id.starts_with("emulator-")
                    || !matches!(
                        state,
                        "device" | "unauthorized" | "offline" | "recovery" | "sideload" | "no"
                    )
                {
                    return None;
                }
                let details: Vec<_> = fields.collect();
                // Network serials are excluded above. Require USB provenance for an authorized device.
                if state == "device" && !details.iter().any(|field| field.starts_with("usb:")) {
                    return None;
                }
                Some(DetectedDevice {
                    id: id.into(),
                    state: if state == "no" {
                        "permission_required".into()
                    } else {
                        state.into()
                    },
                    model: details
                        .iter()
                        .find_map(|field| field.strip_prefix("model:"))
                        .map(str::to_owned),
                })
            }
            Transport::AndroidWireless => None,
            Transport::Ios => {
                let id = line.trim();
                usb_serial(id).then(|| DetectedDevice {
                    id: id.into(),
                    state: "attached".into(),
                    model: None,
                })
            }
            Transport::Camera => {
                let (model, port) = line.trim().rsplit_once(char::is_whitespace)?;
                camera_port(port).then(|| DetectedDevice {
                    id: port.into(),
                    state: "attached".into(),
                    model: Some(model.trim().into()),
                })
            }
        })
        .collect()
}

/// Enumerate local transport devices. An attached iPhone may still require device trust.
pub fn detect_devices(transport: Transport) -> io::Result<Vec<DetectedDevice>> {
    let mut command = match transport {
        Transport::Android => {
            let mut adb = crate::android_wifi::adb_command()?;
            adb.args(["devices", "-l"]);
            adb
        }
        Transport::AndroidWireless => return crate::android_wifi::detect_wireless(),
        Transport::Ios => {
            let mut ios = adapter_command("idevice_id")?;
            ios.arg("-l");
            ios
        }
        Transport::Camera => {
            let mut camera = adapter_command("gphoto2")?;
            camera.arg("--auto-detect");
            camera
        }
    };
    let result = run_adapter(&mut command, Duration::from_secs(15))?;
    Ok(parse_devices(transport, &result))
}

pub(crate) fn stage(parent: &Path) -> io::Result<(TempDir, PathBuf)> {
    let parent = fs::canonicalize(parent)?;
    let staging = Builder::new().prefix(CAPTURE_PREFIX).tempdir_in(parent)?;
    let data = staging.path().join("data");
    fs::create_dir(&data)?;
    Ok((staging, data))
}

pub(crate) fn finish(
    staging: TempDir,
    files: ExportInventory,
    acquisition: &'static str,
    source: String,
    scope: &'static str,
    started_unix_ms: u64,
    verification: &'static str,
) -> io::Result<AcquisitionReport> {
    if files.files.is_empty() || !files.skipped.is_empty() {
        return Err(invalid(
            "capture is empty or contains links, mounts or special entries",
        ));
    }
    let bytes = files.files.values().try_fold(0_u64, |total, file| {
        total
            .checked_add(file.length)
            .ok_or_else(|| invalid("acquired byte count overflow"))
    })?;
    let report = AcquisitionReport {
        schema_version: 1,
        engine_version: env!("CARGO_PKG_VERSION"),
        acquisition,
        source,
        scope,
        started_unix_ms,
        completed_unix_ms: now_ms(),
        bytes,
        output: staging.path().to_owned(),
        verification,
        files,
    };
    let mut manifest = tempfile::NamedTempFile::new_in(staging.path())?;
    serde_json::to_writer_pretty(&mut manifest, &report)?;
    manifest.write_all(b"\n")?;
    manifest.as_file().sync_all()?;
    manifest
        .persist_noclobber(staging.path().join("acquisition.json"))
        .map_err(|error| error.error)?;
    let _ = staging.keep();
    Ok(report)
}

pub(crate) fn android_properties(
    id: &str,
) -> io::Result<std::collections::BTreeMap<&'static str, String>> {
    let mut properties = std::collections::BTreeMap::new();
    for (label, property) in [
        ("manufacturer", "ro.product.manufacturer"),
        ("model", "ro.product.model"),
        ("device", "ro.product.device"),
        ("android_version", "ro.build.version.release"),
        ("sdk", "ro.build.version.sdk"),
        ("security_patch", "ro.build.version.security_patch"),
        ("cpu_abi", "ro.product.cpu.abi"),
        ("chipset", "ro.soc.model"),
        ("build", "ro.build.display.id"),
    ] {
        let mut adb = crate::android_wifi::adb_command()?;
        adb.args(["-s", id, "shell", "getprop", property]);
        let value = run_adapter(&mut adb, Duration::from_secs(12))?;
        let value = value.trim();
        if value.len() > 256 || value.chars().any(char::is_control) {
            return Err(invalid("device returned an invalid system property"));
        }
        if !value.is_empty() {
            properties.insert(label, value.to_owned());
        }
    }
    if !properties.contains_key("model") {
        return Err(invalid("device model could not be read"));
    }
    Ok(properties)
}

fn capture_android_info(id: &str, data: &Path) -> io::Result<()> {
    let properties = android_properties(id)?;
    let mut output = File::create_new(data.join("device-info.json"))?;
    serde_json::to_writer_pretty(&mut output, &properties)?;
    output.write_all(
        b"
",
    )?;
    output.sync_all()
}

/// Collect an authorized logical export into a unique local directory and verify its copied files.
pub fn acquire_device(
    transport: Transport,
    id: &str,
    android_scope: AndroidScope,
    output_parent: &Path,
) -> io::Result<AcquisitionReport> {
    let matches: Vec<_> = detect_devices(transport)?
        .into_iter()
        .filter(|device| device.id == id)
        .collect();
    if matches.len() != 1 || !matches!(matches[0].state.as_str(), "device" | "attached") {
        return Err(invalid(
            "select exactly one authorized device from the current transport inventory",
        ));
    }
    let started = now_ms();
    let (staging, data) = stage(output_parent)?;
    let (acquisition, scope) = match transport {
        Transport::Android | Transport::AndroidWireless => {
            if matches!(android_scope, AndroidScope::DeviceInfo) {
                capture_android_info(id, &data)?;
            } else {
                let mut adb = crate::android_wifi::adb_command()?;
                adb.args(["-s", id, "pull", "-a", android_scope.path()])
                    .arg(&data);
                run_adapter(&mut adb, Duration::from_secs(8 * 60 * 60))?;
            }
            let kind = if transport == Transport::Android {
                "android_adb_usb"
            } else {
                "android_adb_wireless"
            };
            (kind, android_scope.path())
        }
        Transport::Ios => {
            let mut ios = adapter_command("idevicebackup2")?;
            ios.args(["-u", id, "backup", "--full"]).arg(&data);
            run_adapter(&mut ios, Duration::from_secs(8 * 60 * 60))?;
            ("ios_mobilebackup2", "device-provided logical backup")
        }
        Transport::Camera => {
            let mut camera = adapter_command("gphoto2")?;
            camera
                .args(["--port", id, "--quiet", "--keep", "--get-all-files"])
                .current_dir(&data);
            run_adapter(&mut camera, Duration::from_secs(8 * 60 * 60))?;
            ("camera_ptp", "downloadable camera files")
        }
    };
    if transport == Transport::Ios {
        for marker in ["Info.plist", "Status.plist", "Manifest.plist"] {
            if !fs::symlink_metadata(data.join(id).join(marker))
                .is_ok_and(|state| state.is_file() && state.len() > 0)
            {
                return Err(invalid(
                    "iOS backup is missing required MobileBackup2 metadata",
                ));
            }
        }
    }
    let initial = inventory(&data)?;
    let verified = inventory(&data)?;
    if initial.files != verified.files || initial.skipped != verified.skipped {
        return Err(invalid("local export changed during verification"));
    }
    finish(
        staging,
        verified,
        acquisition,
        id.into(),
        scope,
        started,
        "two_matching_local_reads",
    )
}

/// Copy a regular image file and independently hash the resulting image.
pub fn acquire_image(source: &Path, output_parent: &Path) -> io::Result<AcquisitionReport> {
    let mut input = open_image(source)?;
    let state = input.metadata()?;
    let started = now_ms();
    let (staging, data) = stage(output_parent)?;
    let mut output = File::create_new(data.join("capture.dd"))?;
    let sha256 = copy_image(&mut input, &mut output, state.len())?;
    drop(output);
    if !same_file_state(&state, &input.metadata()?) {
        return Err(invalid("image source changed during capture"));
    }
    let files = inventory(&data)?;
    if files
        .files
        .get("capture.dd")
        .is_none_or(|file| file.sha256 != sha256 || file.length != state.len())
    {
        return Err(invalid(
            "image readback did not match the acquisition stream",
        ));
    }
    finish(
        staging,
        files,
        "image_copy",
        source.display().to_string(),
        "whole input image",
        started,
        "stream_and_local_readback_sha256",
    )
}

fn copy_image(input: &mut File, output: &mut File, expected_bytes: u64) -> io::Result<String> {
    if expected_bytes == 0 {
        return Err(invalid("cannot acquire an empty source"));
    }
    let mut digest = Sha256::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    let mut remaining = expected_bytes;
    while remaining != 0 {
        let count = remaining.min(buffer.len() as u64) as usize;
        input.read_exact(&mut buffer[..count])?;
        output.write_all(&buffer[..count])?;
        digest.update(&buffer[..count]);
        remaining -= count as u64;
    }
    if input.read(&mut [0_u8; 1])? != 0 {
        return Err(invalid("source size changed during acquisition"));
    }
    output.sync_all()?;
    Ok(format!("{:x}", digest.finalize()))
}

/// On Linux, acquire an OS-reported read-only whole block device with an exclusive read handle.
#[cfg(target_os = "linux")]
pub fn acquire_disk(source: &Path, output_parent: &Path) -> io::Result<AcquisitionReport> {
    use std::fs::OpenOptions;
    use std::os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt};

    if !source.starts_with("/dev") || source.components().count() != 3 {
        return Err(invalid("select a direct whole-device node under /dev"));
    }
    let mut input = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_EXCL | libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(source)?;
    let state = input.metadata()?;
    if !state.file_type().is_block_device() {
        return Err(invalid("source must be a whole Linux block device"));
    }
    let dev = state.rdev();
    let sys = PathBuf::from(format!(
        "/sys/dev/block/{}:{}",
        libc::major(dev),
        libc::minor(dev)
    ));
    let check = || -> io::Result<(u64, String)> {
        let identity = fs::read_to_string(sys.join("uevent"))?;
        if !identity.lines().any(|line| line == "DEVTYPE=disk")
            || fs::read_to_string(sys.join("ro"))?.trim() != "1"
        {
            return Err(invalid(
                "source must be a whole block device with read-only protection already active",
            ));
        }
        let sectors = fs::read_to_string(sys.join("size"))?;
        let size = sectors
            .trim()
            .parse::<u64>()
            .ok()
            .and_then(|value| value.checked_mul(512))
            .ok_or_else(|| invalid("invalid whole-device capacity"))?;
        let diskseq = fs::read_to_string(sys.join("diskseq"))?;
        Ok((size, diskseq))
    };
    let (size, identity) = check()?;
    let started = now_ms();
    let (staging, data) = stage(output_parent)?;
    let mut output = File::create_new(data.join("capture.dd"))?;
    let sha256 = copy_image(&mut input, &mut output, size)?;
    drop(output);
    if check()? != (size, identity) {
        return Err(invalid("block device changed during acquisition"));
    }
    let files = inventory(&data)?;
    if files
        .files
        .get("capture.dd")
        .is_none_or(|file| file.sha256 != sha256 || file.length != size)
    {
        return Err(invalid(
            "disk image readback did not match the acquisition stream",
        ));
    }
    finish(
        staging,
        files,
        "linux_block_device",
        source.display().to_string(),
        "whole device",
        started,
        "stream_and_local_readback_sha256",
    )
}

const CAPTURE_PREFIX: &str = "OmniRecovery-acquisition-";

#[derive(Debug, Serialize)]
pub struct CaptureHistoryEntry {
    pub name: String,
    pub acquisition: String,
    pub completed_unix_ms: u64,
    pub bytes: u64,
}

#[derive(Debug, Serialize)]
pub struct CaptureHistory {
    pub entries: Vec<CaptureHistoryEntry>,
    pub unreadable: usize,
    pub limited: bool,
}

fn read_manifest(directory: &Path) -> io::Result<Vec<u8>> {
    if !fs::symlink_metadata(directory)?.is_dir() {
        return Err(invalid("acquisition must be a regular directory"));
    }
    let mut manifest = open_image(&directory.join("acquisition.json"))?;
    let state = manifest.metadata()?;
    let mut content = Vec::new();
    (&mut manifest)
        .take(64 * 1024 * 1024 + 1)
        .read_to_end(&mut content)?;
    if content.len() > 64 * 1024 * 1024 || !same_file_state(&state, &manifest.metadata()?) {
        return Err(invalid(
            "manifest exceeded size limit or changed while reading",
        ));
    }
    Ok(content)
}

/// List completed acquisitions below a selected parent folder, without opening payload files.
pub fn capture_history(parent: &Path) -> io::Result<CaptureHistory> {
    #[derive(serde::Deserialize)]
    struct Summary {
        schema_version: u32,
        acquisition: String,
        completed_unix_ms: u64,
        bytes: u64,
    }

    let mut report = CaptureHistory {
        entries: Vec::new(),
        unreadable: 0,
        limited: false,
    };
    let mut remaining = 64 * 1024 * 1024;
    for (index, item) in fs::read_dir(fs::canonicalize(parent)?)?.enumerate() {
        if index == 1000 {
            report.limited = true;
            break;
        }
        let item = item?;
        let name = item.file_name();
        let Some(name) = name.to_str().filter(|name| is_capture_name(name)) else {
            continue;
        };
        if !item.file_type()?.is_dir() {
            continue;
        }
        let path = item.path().join("acquisition.json");
        let length = match fs::symlink_metadata(path) {
            Ok(metadata) => metadata.len(),
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(_) => {
                report.unreadable += 1;
                continue;
            }
        };
        if length > remaining {
            report.limited = true;
            continue;
        }
        remaining -= length;
        match read_manifest(&item.path())
            .and_then(|bytes| Ok(serde_json::from_slice::<Summary>(&bytes)?))
        {
            Ok(summary) if summary.schema_version == 1 => {
                report.entries.push(CaptureHistoryEntry {
                    name: name.into(),
                    acquisition: summary.acquisition,
                    completed_unix_ms: summary.completed_unix_ms,
                    bytes: summary.bytes,
                })
            }
            _ => report.unreadable += 1,
        }
    }
    report.entries.sort_unstable_by(|a, b| {
        b.completed_unix_ms
            .cmp(&a.completed_unix_ms)
            .then_with(|| a.name.cmp(&b.name))
    });
    Ok(report)
}

fn is_capture_name(name: &str) -> bool {
    name.strip_prefix(CAPTURE_PREFIX).is_some_and(usb_serial)
}

/// Verify a backup selected by name from a previously chosen capture folder.
pub fn verify_capture_in_history(parent: &Path, name: &str) -> io::Result<ExportInventory> {
    if !is_capture_name(name) {
        return Err(invalid("select a capture name from this backup history"));
    }
    verify_acquisition(&parent.join(name))
}

/// Verify the recorded payload of a completed acquisition or derivative.
pub fn verify_acquisition(directory: &Path) -> io::Result<ExportInventory> {
    #[derive(serde::Deserialize)]
    struct Manifest {
        schema_version: u32,
        files: CapturedFiles,
    }
    #[derive(serde::Deserialize)]
    struct CapturedFiles {
        files: std::collections::BTreeMap<String, RecordedFile>,
    }
    #[derive(serde::Deserialize)]
    struct RecordedFile {
        length: u64,
        sha256: String,
    }
    let content = read_manifest(directory)?;
    let expected: Manifest = serde_json::from_slice(&content)?;
    let actual = inventory(&directory.join("data"))?;
    if expected.schema_version != 1
        || actual.files.is_empty()
        || !actual.skipped.is_empty()
        || expected.files.files.len() != actual.files.len()
        || actual.files.iter().any(|(path, file)| {
            expected
                .files
                .files
                .get(path)
                .is_none_or(|record| record.sha256 != file.sha256 || record.length != file.length)
        })
    {
        return Err(invalid("capture data does not match its manifest"));
    }
    Ok(actual)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_usb_devices_and_rejects_network_and_emulator_targets() {
        let output = "List of devices attached\nABC123 device usb:1-1 model:Test_Phone\nother unauthorized usb:2-2\n127.0.0.1:5555 device model:Remote\nemulator-5554 device model:Virtual\nphone.local device model:Remote";
        let devices = parse_devices(Transport::Android, output);
        assert_eq!(devices.len(), 2);
        assert_eq!(devices[0].model.as_deref(), Some("Test_Phone"));
        assert_eq!(devices[1].state, "unauthorized");
        assert_eq!(
            parse_devices(Transport::Camera, "Test Camera        usb:001,003")[0].id,
            "usb:001,003"
        );
    }

    #[test]
    fn acquisition_detects_changed_payload_and_keeps_source() -> io::Result<()> {
        let root = tempfile::tempdir()?;
        let input = root.path().join("sample.dd");
        fs::write(&input, b"example bytes")?;
        let report = acquire_image(&input, root.path())?;
        assert_eq!(report.bytes, 13);
        verify_acquisition(&report.output)?;
        let history = capture_history(root.path())?;
        assert_eq!(history.entries.len(), 1);
        assert_eq!(history.entries[0].bytes, 13);
        verify_capture_in_history(root.path(), &history.entries[0].name)?;
        assert!(verify_capture_in_history(root.path(), "../capture").is_err());
        fs::write(report.output.join("data/capture.dd"), b"edited bytes!")?;
        assert!(verify_acquisition(&report.output).is_err());
        assert_eq!(fs::read(input)?, b"example bytes");
        Ok(())
    }
}
