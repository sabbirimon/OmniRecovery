#![cfg(unix)]

use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

use serde_json::Value;

fn helper(directory: &Path, name: &str, script: &str) -> io::Result<()> {
    let script_path = directory.join(name);
    fs::write(&script_path, format!("#!/bin/sh\nset -eu\n{script}"))?;
    fs::set_permissions(script_path, fs::Permissions::from_mode(0o700))
}

fn cli(directory: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_omnirecovery"));
    command.env("PATH", directory);
    command
}

#[test]
fn collects_and_verifies_three_usb_transports_and_rejects_unauthorized_devices() -> io::Result<()> {
    let temp = tempfile::tempdir()?;
    let bin = temp.path().join("helpers");
    let out = temp.path().join("captures");
    fs::create_dir(&bin)?;
    fs::create_dir(&out)?;
    helper(
        &bin,
        "adb",
        r#"
        test "$1 $2 $3 $4" = "-H 127.0.0.1 -P 5037"
        shift 4
        if [ "$1" = devices ]; then
            printf 'List of devices attached\nPHONE device usb:3-1 model:Example\nOTHER unauthorized usb:3-2\n'
        else
            test "$1 $2 $3 $4 $5" = "-s PHONE pull -a /sdcard/DCIM"
            printf 'camera bytes' > "$6/phone.jpg"
        fi
    "#,
    )?;
    helper(
        &bin,
        "idevice_id",
        "test \"$1\" = -l; printf 'test-phone\n'",
    )?;
    helper(
        &bin,
        "idevicebackup2",
        r#"
        test "$1 $2 $3 $4" = "-u test-phone backup --full"
        /bin/mkdir "$5/test-phone"
        for marker in Info.plist Status.plist Manifest.plist; do
            printf 'fixture' > "$5/test-phone/$marker"
        done
    "#,
    )?;
    helper(
        &bin,
        "gphoto2",
        r#"
        if [ "$1" = --auto-detect ]; then
            printf 'Example camera          usb:001,002\n'
        else
            test "$*" = "--port usb:001,002 --quiet --keep --get-all-files"
            printf 'fixture' > "camera.jpg"
        fi
    "#,
    )?;
    let rejected = cli(&bin)
        .args(["acquire", "android", "OTHER"])
        .arg(&out)
        .output()?;
    assert!(!rejected.status.success());
    assert_eq!(fs::read_dir(&out)?.count(), 0);
    for (transport, id, scope) in [
        ("android", "PHONE", "camera"),
        ("ios", "test-phone", "shared"),
        ("camera", "usb:001,002", "shared"),
    ] {
        let response = cli(&bin)
            .args(["acquire", transport, id])
            .arg(&out)
            .args(["--android-scope", scope])
            .output()?;
        assert!(
            response.status.success(),
            "{}",
            String::from_utf8_lossy(&response.stderr)
        );
        let report: Value = serde_json::from_slice(&response.stdout)?;
        let capture = report["output"].as_str().unwrap();
        assert!(
            cli(&bin)
                .args(["verify-acquisition", capture])
                .output()?
                .status
                .success()
        );
    }
    Ok(())
}

#[test]
fn device_service_checks_recovery_identity_and_wireless_tls_inventory() -> io::Result<()> {
    let root = tempfile::tempdir()?;
    let bin = root.path();
    helper(bin, "idevice_id", "test \"$1\" = -l; printf 'test-phone\n'")?;
    helper(bin, "ideviceenterrecovery", "test \"$*\" = test-phone")?;
    helper(
        bin,
        "irecovery",
        r#"
        case "$*" in
            -q|"-i 0x0000000000001234 -q")
                printf 'ECID: 0x0000000000001234\nMODE: Recovery\nPRODUCT: iPhone10,6\n'
                ;;
            "-i 0x0000000000001234 -n")
                ;;
            "-i 0x0000000000002345 -q")
                printf 'ECID: 0x0000000000002345\nMODE: DFU\n'
                ;;
            *) exit 1 ;;
        esac
    "#,
    )?;
    helper(
        bin,
        "fastboot",
        "test \"$*\" = 'devices -l'; printf 'DEVICE fastboot usb:1-1\n'",
    )?;
    helper(
        bin,
        "adb",
        r#"
        shift 4
        case "$*" in
            "mdns services")
                printf 'Test._adb-tls-connect._tcp\t_adb-tls-connect._tcp.\t192.168.5.2:37099\n'
                ;;
            "devices -l")
                printf 'List of devices attached\n192.168.5.2:37099 device model:Paired\n192.168.5.3:5555 device model:Legacy\nPHONE device usb:1-1 model:USB\n'
                ;;
            "connect 192.168.5.2:37099")
                printf 'connected to 192.168.5.2:37099\n'
                ;;
            *) exit 1 ;;
        esac
    "#,
    )?;
    helper(
        bin,
        "scrcpy",
        "test \"$*\" = '--serial PHONE --no-control --no-audio --window-title OmniRecovery Screen Mirror'",
    )?;

    let run = |args: &[&str]| cli(bin).args(args).output();
    let recovery: Value = serde_json::from_slice(&run(&["ios-recovery-status"])?.stdout)?;
    assert_eq!(recovery["mode"], "Recovery");
    assert!(
        run(&[
            "ios-enter-recovery",
            "test-phone",
            "--confirm-id",
            "test-phone"
        ])?
        .status
        .success()
    );
    assert!(
        run(&["ios-exit-recovery", "0x1234", "--confirm-id", "0x1234"])?
            .status
            .success()
    );
    assert!(
        !run(&["ios-exit-recovery", "0x2345", "--confirm-id", "0x2345"])?
            .status
            .success()
    );
    assert!(
        !run(&["ios-enter-recovery", "test-phone", "--confirm-id", "wrong"])?
            .status
            .success()
    );
    assert!(run(&["mirror-android", "PHONE"])?.status.success());
    let fastboot: Value = serde_json::from_slice(&run(&["fastboot-devices"])?.stdout)?;
    assert_eq!(fastboot[0]["id"], "DEVICE");
    let wifi: Value = serde_json::from_slice(&run(&["devices", "android-wireless"])?.stdout)?;
    assert_eq!(wifi.as_array().unwrap().len(), 1);
    assert_eq!(wifi[0]["model"], "Paired");
    assert!(
        run(&["connect-android", "192.168.5.2:37099"])?
            .status
            .success()
    );
    assert!(
        !run(&["connect-android", "192.168.5.3:5555"])?
            .status
            .success()
    );
    Ok(())
}
