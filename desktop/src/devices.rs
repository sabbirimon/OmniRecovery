use serde::Serialize;
use std::io::{Read, Seek};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, Serialize)]
pub struct Disk {
    pub id: String,
    pub name: String,
    pub size: Option<u64>,
    pub bus: Option<String>,
    pub internal: bool,
    pub physical: bool,
    pub solid_state: Option<bool>,
    pub volumes: Vec<Volume>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Volume {
    pub id: String,
    pub name: String,
    pub size: Option<u64>,
    pub format: Option<String>,
    pub mount: Option<PathBuf>,
    pub free: Option<u64>,
    pub writable: Option<bool>,
    #[serde(skip)]
    pub uuid: Option<String>,
    pub selectable: bool,
}

#[derive(Debug, Serialize)]
pub struct UsbDevice {
    pub name: String,
    pub vendor: String,
    pub platform: &'static str,
}

#[derive(Debug, Serialize)]
pub struct Devices {
    pub disks: Vec<Disk>,
    pub usb: Vec<UsbDevice>,
    pub usb_error: Option<String>,
    pub storage_error: Option<String>,
}

#[cfg(not(target_os = "macos"))]
pub fn discover() -> Result<Devices, String> {
    Ok(Devices {
        disks: Vec::new(),
        usb: Vec::new(),
        usb_error: Some("USB product inventory is available in the macOS build.".into()),
        storage_error: Some("Live storage inventory is available in the macOS build.".into()),
    })
}

#[cfg(target_os = "macos")]
pub fn discover() -> Result<Devices, String> {
    mac::discover()
}

pub fn volume<'a>(disks: &'a [Disk], id: &str) -> Option<&'a Volume> {
    disks
        .iter()
        .flat_map(|disk| &disk.volumes)
        .find(|volume| volume.id == id)
}

#[cfg(target_os = "macos")]
pub fn recheck_volume(id: &str) -> Option<Volume> {
    mac::recheck_volume(id)
}

#[cfg(not(target_os = "macos"))]
pub fn recheck_volume(_id: &str) -> Option<Volume> {
    None
}

fn command_output(command: &mut Command, timeout: Duration) -> Result<Vec<u8>, String> {
    let mut output = tempfile::tempfile().map_err(|error| error.to_string())?;
    let writer = output.try_clone().map_err(|error| error.to_string())?;
    let mut child = command
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .stdout(Stdio::from(writer))
        .spawn()
        .map_err(|error| error.to_string())?;
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break,
            Ok(Some(_)) => return Err("A local device inventory command failed.".into()),
            Ok(None) if started.elapsed() < timeout => {
                std::thread::sleep(Duration::from_millis(20));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(
                    "A local device inventory command timed out or could not be monitored.".into(),
                );
            }
        }
    }
    output.rewind().map_err(|error| error.to_string())?;
    let mut bytes = Vec::new();
    output
        .take(8 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() > 8 * 1024 * 1024 {
        return Err("A local device inventory command returned too much data.".into());
    }
    Ok(bytes)
}

#[cfg(windows)]
fn adb_path() -> Option<PathBuf> {
    ["ANDROID_HOME", "ANDROID_SDK_ROOT"]
        .into_iter()
        .filter_map(std::env::var_os)
        .map(PathBuf::from)
        .chain(std::env::var_os("LOCALAPPDATA").map(|path| PathBuf::from(path).join("Android/Sdk")))
        .filter(|root| root.is_absolute())
        .map(|root| root.join("platform-tools/adb.exe"))
        .find(|path| path.is_file())
}

#[cfg(not(windows))]
fn adb_path() -> Option<PathBuf> {
    [
        "/opt/homebrew/bin/adb",
        "/usr/local/bin/adb",
        "/usr/bin/adb",
    ]
    .into_iter()
    .map(PathBuf::from)
    .find(|path| path.is_file())
}

pub fn adb_devices() -> Result<Vec<AdbDevice>, String> {
    let adb = adb_path()
        .ok_or("Android platform-tools (adb) are unavailable at a supported local path.")?;
    let mut command = Command::new(adb);
    command
        .env_remove("ADB_SERVER_SOCKET")
        .env_remove("ANDROID_ADB_SERVER_ADDRESS")
        .env_remove("ANDROID_ADB_SERVER_PORT")
        .args(["-H", "127.0.0.1", "-P", "5037", "devices", "-l"]);
    let output = command_output(&mut command, Duration::from_secs(8))
        .map_err(|error| format!("ADB could not query the local USB bridge. {error}"))?;
    Ok(parse_adb(&String::from_utf8_lossy(&output)))
}

#[derive(Debug, Serialize)]
pub struct AdbDevice {
    pub model: String,
    pub state: String,
}

fn parse_adb(output: &str) -> Vec<AdbDevice> {
    output
        .lines()
        .skip_while(|line| !line.starts_with("List of devices attached"))
        .skip(1)
        .filter_map(|line| {
            let mut columns = line.split_whitespace();
            let id = columns.next()?;
            let state = columns.next()?;
            if id.starts_with("emulator-")
                || id.contains(':')
                || id.ends_with("._adb-tls-connect._tcp")
            {
                return None;
            }
            let model = columns
                .find_map(|item| item.strip_prefix("model:"))
                .unwrap_or("Android device")
                .replace('_', " ");
            Some(AdbDevice {
                model,
                state: match state {
                    "device" => "authorized",
                    "unauthorized" => "awaiting_usb_debugging_approval",
                    "offline" => "offline",
                    _ => "unavailable",
                }
                .into(),
            })
        })
        .collect()
}

#[cfg(target_os = "macos")]
mod mac {
    use super::*;
    use plist::{Dictionary, Value};
    use std::io::Cursor;
    use std::path::Path;

    fn plist(command: &str, args: &[&str], timeout: Duration) -> Result<Value, String> {
        let bytes = command_output(Command::new(command).args(args), timeout)?;
        Value::from_reader(Cursor::new(bytes)).map_err(|error| error.to_string())
    }

    pub(super) fn discover() -> Result<Devices, String> {
        let list = plist(
            "/usr/sbin/diskutil",
            &["list", "-plist"],
            Duration::from_secs(2),
        );
        let records = list
            .as_ref()
            .ok()
            .and_then(Value::as_dictionary)
            .and_then(|row| row.get("AllDisksAndPartitions"))
            .and_then(Value::as_array);
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut missing_details = false;
        let mut details = |id| {
            let timeout = deadline
                .saturating_duration_since(Instant::now())
                .min(Duration::from_millis(800));
            let info = (!timeout.is_zero())
                .then(|| disk_info(id, timeout).ok())
                .flatten();
            missing_details |= info.is_none();
            info.unwrap_or_default()
        };
        let mut disks = Vec::new();
        for record in records
            .into_iter()
            .flatten()
            .take(96)
            .filter_map(Value::as_dictionary)
        {
            let Some(id) = text(record, "DeviceIdentifier").filter(|id| valid_id(id)) else {
                continue;
            };
            let info = details(id);
            let mut volumes = Vec::new();
            for item in ["Partitions", "APFSVolumes"]
                .into_iter()
                .filter_map(|key| record.get(key).and_then(Value::as_array))
                .flatten()
                .filter_map(Value::as_dictionary)
            {
                if let Some(id) = text(item, "DeviceIdentifier").filter(|id| valid_id(id)) {
                    volumes.push(volume(id, item, &details(id)));
                }
            }
            if volumes.is_empty() && info.get("MountPoint").is_some() {
                volumes.push(volume(id, record, &info));
            }
            disks.push(Disk {
                id: id.into(),
                name: text(&info, "MediaName")
                    .filter(|name| !name.is_empty())
                    .or_else(|| text(record, "Content"))
                    .unwrap_or(id)
                    .into(),
                size: integer(record, "Size"),
                bus: text(&info, "BusProtocol").map(str::to_owned),
                internal: flag(&info, "Internal") == Some(true),
                physical: text(&info, "VirtualOrPhysical") == Some("Physical"),
                solid_state: flag(&info, "SolidState"),
                volumes,
            });
        }
        let (usb, usb_error) = match usb() {
            Ok(usb) => (usb, None),
            Err(_) => (
                Vec::new(),
                Some("USB inventory is temporarily unavailable.".into()),
            ),
        };
        Ok(Devices {
            disks,
            usb,
            usb_error,
            storage_error: if records.is_none() {
                Some("macOS did not return a storage list. Refresh devices to try again.".into())
            } else if missing_details {
                Some("Some drives did not return details. Their read-only listings may be incomplete.".into())
            } else {
                None
            },
        })
    }

    pub(super) fn recheck_volume(id: &str) -> Option<Volume> {
        let info = disk_info(id, Duration::from_secs(2)).ok()?;
        Some(volume(id, &Dictionary::new(), &info))
    }

    fn volume(id: &str, row: &Dictionary, info: &Dictionary) -> Volume {
        let mount = text(info, "MountPoint")
            .or_else(|| text(row, "MountPoint"))
            .map(PathBuf::from);
        let uuid = text(info, "VolumeUUID")
            .or_else(|| text(info, "DiskUUID"))
            .map(str::to_owned);
        let selectable = uuid.is_some()
            && mount
                .as_ref()
                .is_some_and(|path| path.parent() == Some(Path::new("/Volumes")));
        Volume {
            id: id.into(),
            name: text(row, "VolumeName")
                .or_else(|| text(info, "VolumeName"))
                .or_else(|| text(row, "Content"))
                .unwrap_or(id)
                .into(),
            size: integer(row, "Size"),
            format: text(info, "FilesystemType")
                .or_else(|| text(row, "Content"))
                .map(str::to_owned),
            mount,
            free: integer(info, "VolumeFreeSpace")
                .or_else(|| integer(info, "APFSContainerFree"))
                .or_else(|| integer(info, "FreeSpace")),
            writable: flag(info, "WritableVolume"),
            uuid,
            selectable,
        }
    }

    fn usb() -> Result<Vec<UsbDevice>, String> {
        let devices = plist(
            "/usr/sbin/ioreg",
            &["-a", "-r", "-c", "IOUSBHostDevice", "-d", "1"],
            Duration::from_secs(2),
        )?;
        let rows = devices
            .as_array()
            .ok_or("macOS did not return a USB list.")?;
        Ok(rows
            .iter()
            .take(64)
            .filter_map(Value::as_dictionary)
            .filter_map(|row| {
                let name = text(row, "USB Product Name")
                    .or_else(|| text(row, "IORegistryEntryName"))?
                    .trim();
                let vendor = text(row, "USB Vendor Name")
                    .unwrap_or("Unknown vendor")
                    .trim();
                let lower = name.to_lowercase();
                let platform = if ["iphone", "ipad", "ipod"]
                    .iter()
                    .any(|word| lower.contains(word))
                    && integer(row, "idVendor") == Some(0x05ac)
                {
                    "iOS"
                } else if [
                    "android",
                    "pixel",
                    "galaxy",
                    "fastboot",
                    "adb interface",
                    "mtp",
                ]
                .iter()
                .any(|word| lower.contains(word))
                {
                    "Possible Android"
                } else {
                    "USB peripheral"
                };
                Some(UsbDevice {
                    name: name.into(),
                    vendor: vendor.into(),
                    platform,
                })
            })
            .collect())
    }

    fn disk_info(id: &str, timeout: Duration) -> Result<Dictionary, String> {
        if !valid_id(id) {
            return Err("Invalid macOS disk identifier.".into());
        }
        plist("/usr/sbin/diskutil", &["info", "-plist", id], timeout)?
            .into_dictionary()
            .ok_or("macOS disk details were unavailable.".into())
    }
    fn valid_id(value: &str) -> bool {
        value.strip_prefix("disk").is_some_and(|tail| {
            !tail.is_empty() && tail.bytes().all(|b| b.is_ascii_digit() || b == b's')
        })
    }
    fn text<'a>(row: &'a Dictionary, key: &str) -> Option<&'a str> {
        row.get(key).and_then(Value::as_string)
    }
    fn flag(row: &Dictionary, key: &str) -> Option<bool> {
        row.get(key).and_then(Value::as_boolean)
    }
    fn integer(row: &Dictionary, key: &str) -> Option<u64> {
        row.get(key).and_then(Value::as_unsigned_integer)
    }
}
