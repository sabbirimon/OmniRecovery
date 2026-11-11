use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::Command;

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
}

#[cfg(not(target_os = "macos"))]
pub fn discover() -> Result<Devices, String> {
    Err("Live storage inventory is available in the macOS build.".into())
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

pub fn adb_devices() -> Result<Vec<AdbDevice>, String> {
    let adb = [
        "/opt/homebrew/bin/adb",
        "/usr/local/bin/adb",
        "/usr/bin/adb",
    ]
    .into_iter()
    .find(|path| Path::new(path).is_file())
    .ok_or("Android platform-tools (adb) are unavailable at a supported local path.")?;
    let output = Command::new(adb)
        .env_remove("ADB_SERVER_SOCKET")
        .env_remove("ANDROID_ADB_SERVER_ADDRESS")
        .env_remove("ANDROID_ADB_SERVER_PORT")
        .args(["-H", "127.0.0.1", "-P", "5037", "devices", "-l"])
        .output()
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err("ADB could not query the local USB bridge.".into());
    }
    Ok(parse_adb(&String::from_utf8_lossy(&output.stdout)))
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

    fn plist(command: &str, args: &[&str]) -> Result<Value, String> {
        let output = Command::new(command)
            .args(args)
            .output()
            .map_err(|error| error.to_string())?;
        if !output.status.success() {
            return Err("A local device inventory command failed.".into());
        }
        Value::from_reader(Cursor::new(output.stdout)).map_err(|error| error.to_string())
    }

    pub(super) fn discover() -> Result<Devices, String> {
        let list = plist("/usr/sbin/diskutil", &["list", "-plist"])?;
        let records = list
            .as_dictionary()
            .and_then(|row| row.get("AllDisksAndPartitions"))
            .and_then(Value::as_array)
            .ok_or("macOS did not return a storage list.")?;
        let mut disks = Vec::new();
        for record in records.iter().take(96).filter_map(Value::as_dictionary) {
            let Some(id) = text(record, "DeviceIdentifier").filter(|id| valid_id(id)) else {
                continue;
            };
            let info = disk_info(id).unwrap_or_default();
            let mut volumes = Vec::new();
            for item in ["Partitions", "APFSVolumes"]
                .into_iter()
                .filter_map(|key| record.get(key).and_then(Value::as_array))
                .flatten()
                .filter_map(Value::as_dictionary)
            {
                if let Some(volume) = volume(item) {
                    volumes.push(volume);
                }
            }
            if volumes.is_empty()
                && info.get("MountPoint").is_some()
                && let Some(volume) = volume(record)
            {
                volumes.push(volume);
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
            Err(error) => (Vec::new(), Some(error)),
        };
        Ok(Devices {
            disks,
            usb,
            usb_error,
        })
    }

    fn volume(row: &Dictionary) -> Option<Volume> {
        let id = text(row, "DeviceIdentifier").filter(|id| valid_id(id))?;
        let info = disk_info(id).unwrap_or_default();
        let mount = text(&info, "MountPoint")
            .or_else(|| text(row, "MountPoint"))
            .map(PathBuf::from);
        let uuid = text(&info, "VolumeUUID")
            .or_else(|| text(&info, "DiskUUID"))
            .map(str::to_owned);
        let selectable = uuid.is_some()
            && mount
                .as_ref()
                .is_some_and(|path| path.parent() == Some(Path::new("/Volumes")));
        Some(Volume {
            id: id.into(),
            name: text(row, "VolumeName")
                .or_else(|| text(&info, "VolumeName"))
                .or_else(|| text(row, "Content"))
                .unwrap_or(id)
                .into(),
            size: integer(row, "Size"),
            format: text(&info, "FilesystemType")
                .or_else(|| text(row, "Content"))
                .map(str::to_owned),
            mount,
            free: integer(&info, "VolumeFreeSpace")
                .or_else(|| integer(&info, "APFSContainerFree"))
                .or_else(|| integer(&info, "FreeSpace")),
            writable: flag(&info, "WritableVolume"),
            uuid,
            selectable,
        })
    }

    fn usb() -> Result<Vec<UsbDevice>, String> {
        let devices = plist(
            "/usr/sbin/ioreg",
            &["-a", "-r", "-c", "IOUSBHostDevice", "-d", "1"],
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

    fn disk_info(id: &str) -> Result<Dictionary, String> {
        plist("/usr/sbin/diskutil", &["info", "-plist", id])?
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
