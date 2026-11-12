use omnirecovery::{DetectedDevice, Transport, detect_devices};
use serde::Serialize;
#[cfg(target_os = "macos")]
use std::io::{Read, Seek};
use std::path::PathBuf;
#[cfg(target_os = "macos")]
use std::process::{Command, Stdio};
#[cfg(target_os = "macos")]
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
    #[serde(skip)]
    serial: Option<String>,
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

#[cfg(target_os = "macos")]
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

#[derive(Debug, Serialize)]
pub struct Phone {
    pub name: String,
    pub transport: &'static str,
    pub usb_detected: bool,
    pub state: String,
    pub target: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct PhoneInventory {
    pub phones: Vec<Phone>,
    pub android_error: Option<String>,
    pub ios_error: Option<String>,
    pub usb_error: Option<String>,
}

fn merge_phones(
    usb: Vec<UsbDevice>,
    mut android: Vec<DetectedDevice>,
    mut ios: Vec<DetectedDevice>,
) -> Vec<Phone> {
    let mut phones = Vec::new();
    for physical in usb {
        let (targets, transport) = match physical.platform {
            "Android" => (&mut android, "android"),
            "iOS" => (&mut ios, "ios"),
            _ => continue,
        };
        let found = physical
            .serial
            .as_deref()
            .and_then(|serial| {
                targets.iter().position(|candidate| {
                    if transport == "ios" {
                        candidate.id.eq_ignore_ascii_case(serial)
                    } else {
                        candidate.id == serial
                    }
                })
            })
            .map(|index| targets.remove(index));
        let name = found
            .as_ref()
            .and_then(|device| device.model.as_ref())
            .map_or_else(
                || physical.name.clone(),
                |model| {
                    let model = model.replace('_', "-");
                    if physical.name.eq_ignore_ascii_case("samsung_android") {
                        format!("Samsung {model}")
                    } else {
                        physical.name.clone()
                    }
                },
            );
        let state = found
            .as_ref()
            .map_or_else(|| "usb_detected".into(), |device| device.state.clone());
        let target = found.map(|device| device.id);
        phones.push(Phone {
            name,
            transport,
            usb_detected: true,
            state,
            target,
        });
    }
    for (transport, targets) in [("android", android), ("ios", ios)] {
        phones.extend(targets.into_iter().map(|device| {
            Phone {
                name: device
                    .model
                    .map(|model| model.replace('_', " "))
                    .unwrap_or_else(|| {
                        if transport == "ios" {
                            "iPhone or iPad".into()
                        } else {
                            "Android phone".into()
                        }
                    }),
                transport,
                usb_detected: false,
                state: device.state,
                target: Some(device.id),
            }
        }));
    }
    phones
}

pub fn discover_phones() -> PhoneInventory {
    let (android, ios) = std::thread::scope(|scope| {
        let android = scope.spawn(|| detect_devices(Transport::Android));
        let ios = detect_devices(Transport::Ios);
        (
            android
                .join()
                .unwrap_or_else(|_| Err(std::io::Error::other("Android inventory unavailable"))),
            ios,
        )
    });
    #[cfg(target_os = "macos")]
    let usb = mac::usb();
    #[cfg(not(target_os = "macos"))]
    let usb: Result<Vec<UsbDevice>, String> = Ok(Vec::new());
    let android_error = android.as_ref().err().map(ToString::to_string);
    let ios_error = ios.as_ref().err().map(ToString::to_string);
    let usb_error = usb.as_ref().err().cloned();
    let phones = merge_phones(
        usb.unwrap_or_default(),
        android.unwrap_or_default(),
        ios.unwrap_or_default(),
    );
    PhoneInventory {
        phones,
        android_error,
        ios_error,
        usb_error,
    }
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

    pub(super) fn usb() -> Result<Vec<UsbDevice>, String> {
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
                let platform = phone_platform(name, vendor, integer(row, "idVendor"));
                Some(UsbDevice {
                    name: name.into(),
                    vendor: vendor.into(),
                    platform,
                    serial: text(row, "USB Serial Number").map(str::to_owned),
                })
            })
            .collect())
    }

    fn phone_platform(name: &str, vendor: &str, vendor_id: Option<u64>) -> &'static str {
        let name = name.to_ascii_lowercase();
        let vendor = vendor.to_ascii_lowercase();
        if vendor_id == Some(0x05ac)
            && ["iphone", "ipad", "ipod"]
                .iter()
                .any(|model| name.contains(model))
        {
            return "iOS";
        }
        let model_hints = [
            "android",
            "pixel",
            "galaxy",
            "redmi",
            "poco",
            "fastboot",
            "adb interface",
            "moto ",
            "oneplus",
            "oppo",
            "realme",
            "vivo",
        ];
        let mobile_vendor = matches!(vendor_id, Some(0x2717 | 0x22d9 | 0x2d95 | 0x22b8 | 0x2a70))
            || ["xiaomi", "oneplus", "motorola", "oppo", "realme", "vivo"]
                .iter()
                .any(|maker| vendor.contains(maker));
        if model_hints.iter().any(|hint| name.contains(hint)) || mobile_vendor {
            return "Android";
        }
        "USB peripheral"
    }

    #[cfg(test)]
    #[test]
    fn usb_inventory_recognizes_redmi_before_adb_authorization() {
        assert_eq!(
            phone_platform("Redmi Note 10", "Xiaomi", Some(0x2717)),
            "Android"
        );
        assert_eq!(
            phone_platform("Galaxy A", "Samsung", Some(0x04e8)),
            "Android"
        );
        assert_eq!(
            phone_platform("T2 Controller", "Apple", Some(0x05ac)),
            "USB peripheral"
        );
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usb_presence_and_adb_authorization_are_independent() {
        let physical = || UsbDevice {
            name: "Redmi Note 10".into(),
            vendor: "Xiaomi".into(),
            platform: "Android",
            serial: Some("PHONE1".into()),
        };
        let raw = merge_phones(vec![physical()], Vec::new(), Vec::new());
        assert_eq!(raw[0].state, "usb_detected");
        assert!(raw[0].target.is_none());
        let adb = DetectedDevice {
            id: "PHONE1".into(),
            state: "device".into(),
            model: Some("Redmi".into()),
        };
        let ready = merge_phones(vec![physical()], vec![adb], Vec::new());
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].target.as_deref(), Some("PHONE1"));
    }
}
