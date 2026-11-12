use std::io;
use std::process::{Child, Stdio};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::acquisition::{adapter_command, detect_devices, find_helper, run_adapter, usb_serial};
use crate::source::invalid;
use crate::{DetectedDevice, Transport};

#[derive(Debug, Serialize)]
pub struct IosRecoveryDevice {
    pub ecid: String,
    pub mode: String,
    pub product: Option<String>,
    pub model: Option<String>,
}

fn ecid_number(value: &str) -> io::Result<u64> {
    value
        .strip_prefix("0x")
        .filter(|hex| !hex.is_empty() && hex.len() <= 16)
        .and_then(|hex| u64::from_str_radix(hex, 16).ok())
        .filter(|value| *value != 0)
        .ok_or_else(|| {
            invalid("use the nonzero hexadecimal ECID shown by iOS recovery diagnostics")
        })
}

fn parse_ios_recovery(output: &str) -> io::Result<IosRecoveryDevice> {
    let get = |field: &str| {
        output.lines().find_map(|line| {
            let (label, value) = line.split_once(':')?;
            let value = value.trim();
            (label == field && !value.is_empty() && value.len() <= 128).then_some(value)
        })
    };
    let ecid = ecid_number(get("ECID").unwrap_or_default())?;
    let mode = get("MODE")
        .filter(|value| matches!(*value, "Recovery" | "DFU" | "WTF" | "Port DFU"))
        .ok_or_else(|| invalid("iOS helper did not report a recognized recovery mode"))?;
    Ok(IosRecoveryDevice {
        ecid: format!("0x{ecid:016x}"),
        mode: mode.to_owned(),
        product: get("PRODUCT").map(str::to_owned),
        model: get("MODEL").map(str::to_owned),
    })
}

pub fn ios_recovery_status() -> io::Result<IosRecoveryDevice> {
    parse_ios_recovery(&run_adapter(
        adapter_command("irecovery")?.arg("-q"),
        Duration::from_secs(15),
    )?)
}

pub fn android_fastboot_devices() -> io::Result<Vec<DetectedDevice>> {
    let output = run_adapter(
        adapter_command("fastboot")?.args(["devices", "-l"]),
        Duration::from_secs(15),
    )?;
    Ok(output
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let id = fields.next()?;
            let state = fields.next()?;
            (usb_serial(id) && state == "fastboot").then(|| DetectedDevice {
                id: id.into(),
                state: state.into(),
                model: None,
            })
        })
        .collect())
}

fn require_device(transport: Transport, id: &str, state: &str) -> io::Result<()> {
    let count = detect_devices(transport)?
        .into_iter()
        .filter(|target| target.id == id && target.state == state)
        .count();
    if count != 1 {
        return Err(invalid(
            "select exactly one connected target in the expected USB state",
        ));
    }
    Ok(())
}

/// Enter recovery mode on an explicitly selected, normally connected iOS device.
pub fn ios_enter_recovery(id: &str) -> io::Result<()> {
    require_device(Transport::Ios, id, "attached")?;
    run_adapter(
        adapter_command("ideviceenterrecovery")?.arg(id),
        Duration::from_secs(20),
    )?;
    Ok(())
}

/// Exit iOS Recovery Mode after checking the target ECID and current mode.
pub fn ios_exit_recovery(ecid: &str) -> io::Result<()> {
    let ecid = format!("0x{:016x}", ecid_number(ecid)?);
    let mut command = adapter_command("irecovery")?;
    command.args(["-i", &ecid]);
    let mut query = adapter_command("irecovery")?;
    query.args(["-i", &ecid, "-q"]);
    let actual = parse_ios_recovery(&run_adapter(&mut query, Duration::from_secs(15))?)?;
    if actual.ecid != ecid || actual.mode != "Recovery" {
        return Err(invalid(
            "normal-mode reboot requires the matching target in Recovery Mode",
        ));
    }
    run_adapter(command.arg("-n"), Duration::from_secs(20))?;
    Ok(())
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum MirrorProfile {
    LowBandwidth,
    #[default]
    Balanced,
    HighQuality,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum MirrorVideo {
    #[default]
    Screen,
    FrontCamera,
    RearCamera,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum MirrorAudio {
    #[default]
    Off,
    Device,
    Microphone,
}

#[derive(Clone, Copy, Debug, Default, Deserialize)]
pub struct MirrorOptions {
    pub profile: MirrorProfile,
    pub video: MirrorVideo,
    pub audio: MirrorAudio,
}

#[derive(Debug, Serialize)]
pub struct AndroidStorage {
    pub capacity: u64,
    pub available: u64,
}

#[derive(Debug, Serialize)]
pub struct AndroidDetails {
    pub properties: std::collections::BTreeMap<&'static str, String>,
    pub shared_storage: Option<AndroidStorage>,
    pub battery_percent: Option<u8>,
    pub display: Option<String>,
}

fn android_shell(id: &str, args: &[&str]) -> io::Result<String> {
    let mut command = crate::android_wifi::adb_command()?;
    command.args(["-s", id, "shell"]).args(args);
    run_adapter(&mut command, Duration::from_secs(8))
}

fn require_android(transport: Transport, id: &str) -> io::Result<()> {
    if !matches!(transport, Transport::Android | Transport::AndroidWireless) {
        return Err(invalid(
            "select an Android USB or authorized wireless target",
        ));
    }
    require_device(transport, id, "device")
}

fn parse_storage(output: &str) -> Option<AndroidStorage> {
    let mut columns = output.lines().last()?.split_whitespace();
    columns.next()?;
    let capacity = columns.next()?.parse::<u64>().ok()?.checked_mul(1024)?;
    columns.next()?;
    let available = columns.next()?.parse::<u64>().ok()?.checked_mul(1024)?;
    (capacity > 0 && available <= capacity).then_some(AndroidStorage {
        capacity,
        available,
    })
}

/// Read only the named Android system properties and capacity/health counters.
pub fn inspect_android_details(transport: Transport, id: &str) -> io::Result<AndroidDetails> {
    require_android(transport, id)?;
    let properties = crate::acquisition::android_properties(id)?;
    let (shared_storage, battery_percent, display) = std::thread::scope(|scope| {
        let storage = scope.spawn(|| {
            android_shell(id, &["df", "-k", "/sdcard"])
                .ok()
                .and_then(|text| parse_storage(&text))
        });
        let battery = scope.spawn(|| {
            android_shell(id, &["dumpsys", "battery"])
                .ok()
                .and_then(|text| {
                    let level = text
                        .lines()
                        .find_map(|line| line.trim().strip_prefix("level:"))?
                        .trim()
                        .parse::<u16>()
                        .ok()?;
                    let scale = text
                        .lines()
                        .find_map(|line| line.trim().strip_prefix("scale:"))?
                        .trim()
                        .parse::<u16>()
                        .ok()?;
                    (scale > 0 && level <= scale)
                        .then(|| ((u32::from(level) * 100) / u32::from(scale)) as u8)
                })
        });
        let display = android_shell(id, &["wm", "size"]).ok().and_then(|text| {
            let size = text
                .lines()
                .rev()
                .find_map(|line| line.split_once(':').map(|(_, size)| size.trim()))?;
            let (width, height) = size.split_once('x')?;
            if width.parse::<u32>().ok()? == 0 || height.parse::<u32>().ok()? == 0 {
                return None;
            }
            Some(size.to_owned())
        });
        (
            storage.join().unwrap_or(None),
            battery.join().unwrap_or(None),
            display,
        )
    });
    Ok(AndroidDetails {
        properties,
        shared_storage,
        battery_percent,
        display,
    })
}

fn mirror_args(options: MirrorOptions) -> Vec<&'static str> {
    let (size, fps, bitrate) = match options.profile {
        MirrorProfile::LowBandwidth => ("--max-size=720", "--max-fps=30", "--video-bit-rate=2M"),
        MirrorProfile::Balanced => ("--max-size=1280", "--max-fps=45", "--video-bit-rate=6M"),
        MirrorProfile::HighQuality => ("--max-size=1920", "--max-fps=60", "--video-bit-rate=12M"),
    };
    let mut args = vec!["--no-control", size, bitrate];
    match options.video {
        MirrorVideo::Screen => args.push(fps),
        MirrorVideo::FrontCamera => args.extend([
            "--video-source=camera",
            "--camera-facing=front",
            "--camera-fps=30",
        ]),
        MirrorVideo::RearCamera => args.extend([
            "--video-source=camera",
            "--camera-facing=back",
            "--camera-fps=30",
        ]),
    }
    match options.audio {
        MirrorAudio::Off => args.push("--no-audio"),
        MirrorAudio::Device => args.extend(["--audio-source=output", "--require-audio"]),
        MirrorAudio::Microphone => args.extend(["--audio-source=mic", "--require-audio"]),
    }
    args
}

/// Launch the installed scrcpy engine for a selected USB or paired TLS device.
pub fn start_android_preview(
    transport: Transport,
    id: &str,
    options: MirrorOptions,
) -> io::Result<Child> {
    require_android(transport, id)?;
    if options.video != MirrorVideo::Screen || options.audio != MirrorAudio::Off {
        let sdk = android_shell(id, &["getprop", "ro.build.version.sdk"])?
            .trim()
            .parse::<u32>()
            .map_err(|_| invalid("could not confirm the Android version for camera or audio"))?;
        if options.video != MirrorVideo::Screen && sdk < 31 {
            return Err(invalid(
                "scrcpy camera preview requires Android 12 or newer",
            ));
        }
        if options.audio != MirrorAudio::Off && sdk < 30 {
            return Err(invalid(
                "scrcpy audio forwarding requires Android 11 or newer",
            ));
        }
    }
    adapter_command("scrcpy")?
        .env("ADB", find_helper("adb")?)
        .env("ADB_SERVER_SOCKET", "tcp:127.0.0.1:5037")
        .env_remove("ANDROID_SERIAL")
        .env_remove("SCRCPY_SERVER_PATH")
        .args([
            "--serial",
            id,
            "--window-title",
            "OmniRecovery Screen Mirror",
        ])
        .args(mirror_args(options))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
}

pub fn start_android_mirror(id: &str) -> io::Result<Child> {
    start_android_preview(Transport::Android, id, MirrorOptions::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camera_and_microphone_are_explicit_and_use_a_bounded_profile() {
        let defaults = mirror_args(MirrorOptions::default());
        assert!(defaults.contains(&"--no-audio"));
        assert!(!defaults.contains(&"--video-source=camera"));
        let camera = mirror_args(MirrorOptions {
            video: MirrorVideo::FrontCamera,
            audio: MirrorAudio::Microphone,
            profile: MirrorProfile::LowBandwidth,
        });
        assert!(camera.contains(&"--camera-facing=front"));
        assert!(camera.contains(&"--audio-source=mic"));
        assert!(camera.contains(&"--require-audio"));
        assert!(camera.contains(&"--video-bit-rate=2M"));
        assert!(camera.contains(&"--no-control"));
        let storage = parse_storage("Filesystem 1K-blocks Used Available Use% Mounted on\n/dev/fuse 1000 450 500 48% /storage/emulated").unwrap();
        assert_eq!(storage.available, 512000);
    }

    #[test]
    fn recovery_diagnostics_read_only_selected_fields() -> io::Result<()> {
        let result = parse_ios_recovery(
            "ECID: 0x1234\nMODE: Recovery\nPRODUCT: iPhone10,6\nSRNM: sample-secret",
        )?;
        assert_eq!(result.ecid, "0x0000000000001234");
        assert_eq!(result.mode, "Recovery");
        assert!(!serde_json::to_string(&result)?.contains("sample-secret"));
        assert!(parse_ios_recovery("ECID: 0x0000\nMODE: Recovery").is_err());
        Ok(())
    }
}
