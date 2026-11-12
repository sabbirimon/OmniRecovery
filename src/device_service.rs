use std::io;
use std::process::{Child, Stdio};
use std::time::Duration;

use serde::Serialize;

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

/// Start an explicit display-only Android mirror using the installed scrcpy helper.
pub fn start_android_mirror(id: &str) -> io::Result<Child> {
    require_device(Transport::Android, id, "device")?;
    adapter_command("scrcpy")?
        .env("ADB", find_helper("adb")?)
        .env("ADB_SERVER_SOCKET", "tcp:127.0.0.1:5037")
        .env_remove("ANDROID_SERIAL")
        .env_remove("SCRCPY_SERVER_PATH")
        .args([
            "--serial",
            id,
            "--no-control",
            "--no-audio",
            "--window-title",
            "OmniRecovery Screen Mirror",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
}

#[cfg(test)]
mod tests {
    use super::*;

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
