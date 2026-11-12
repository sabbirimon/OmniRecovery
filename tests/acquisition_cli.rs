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
        "test \"$*\" = '--serial PHONE --window-title OmniRecovery Screen Mirror --no-control --max-size=1280 --video-bit-rate=6M --max-fps=45 --no-audio'",
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

#[test]
fn external_parsers_receive_staged_sources_and_produce_verified_reports() -> io::Result<()> {
    use sha2::{Digest, Sha256};
    let root = tempfile::tempdir()?;
    let output = root.path().join("output");
    let export = root.path().join("export");
    let checkout = root.path().join("checkout");
    for directory in [&output, &export, &checkout] {
        fs::create_dir(directory)?;
    }
    fs::write(export.join("fixture.txt"), b"example export")?;
    let script = b"mock installed parser";
    fs::write(checkout.join("aleapp.py"), script)?;
    fs::write(checkout.join("ileapp.py"), script)?;
    helper(
        root.path(),
        "python",
        r#"
        test "$1" = -s
        test "$3" = -t
        test "$5" = -i
        test "$7" = -o
        test "$(/bin/cat "$6/fixture.txt")" = 'example export'
        printf 'kind\tvalue\nfixture\tvalidated\n' > "$8/report.tsv"
    "#,
    )?;
    let sha256 = format!("{:x}", Sha256::digest(script));
    for (engine, format) in [("aleapp", "fs"), ("ileapp", "itunes")] {
        let result = cli(root.path())
            .args(["analyze-leapp", engine])
            .arg(&export)
            .arg(&output)
            .args(["--format", format, "--checkout"])
            .arg(&checkout)
            .arg("--python")
            .arg(root.path().join("python"))
            .args(["--script-sha256", &sha256])
            .output()?;
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let report: Value = serde_json::from_slice(&result.stdout)?;
        assert_eq!(
            report["verification"],
            "verified_staging_and_hashed_reports"
        );
        assert!(report["files"]["files"]["report.tsv"].is_object());
    }
    helper(
        root.path(),
        "fls",
        r#"
        if [ "$1" = "-V" ]; then printf 'Sleuth Kit fixture\n'; exit; fi
        test "$1 $2 $3 $4 $5 $6 $7 $8 $9" = "-d -r -p -i raw -b 512 -o 0"
        shift 9
        test -f "$1"
        printf 'r/r * 42: deleted.txt\n'
    "#,
    )?;
    let image = root.path().join("fixture.dd");
    fs::write(&image, [0_u8; 512])?;
    let result = cli(root.path())
        .arg("tsk-deleted")
        .arg(&image)
        .arg(&output)
        .output()?;
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let report: Value = serde_json::from_slice(&result.stdout)?;
    assert_eq!(report["acquisition"], "sleuth_kit_deleted_listing");
    assert!(report["files"]["files"]["tsk-deleted.txt"].is_object());
    Ok(())
}

#[test]
fn browses_and_exports_selected_android_files_then_verifies_archive_delivery() -> io::Result<()> {
    let root = tempfile::tempdir()?;
    let export = root.path().join("export");
    let target = root.path().join("receiving");
    fs::create_dir(&export)?;
    fs::create_dir(&target)?;
    let photo = export.join("example.jpg");
    fs::write(&photo, b"fixture photo bytes")?;
    let archive = root.path().join("backup.omnibackup");
    omnirecovery::backup_files(&export, &archive, "synthetic test passphrase")?;
    let photo_hash = omnirecovery::verify_file(&photo)?.sha256;
    let archive_hash = omnirecovery::verify_file(&archive)?.sha256;
    helper(
        root.path(),
        "adb",
        r#"
        test "$1 $2 $3 $4" = "-H 127.0.0.1 -P 5037"
        shift 4
        if [ "$1" = devices ]; then
            printf 'List of devices attached\nSOURCE device usb:3-1 model:Samsung\nTARGET device usb:3-2 model:Pixel\nLOCKED unauthorized usb:3-3\n'
            exit
        fi
        test "$1" = "-s"
        case "$2 $3" in
          "SOURCE exec-out")
            case "$4" in
              *'head -c 262145')
                printf "/sdcard/DCIM/O'Brien.jpg\000/sdcard/DCIM/clip.mp4\000/sdcard/DCIM/../private.jpg\000/sdcard/Android/data/private.jpg\000\000OK\000"
                ;;
              *'sha256sum'*)
                case "$4" in *"O'\''Brien.jpg"*) ;; *) exit 1;; esac
                printf '%s  photo\n' "$PHOTO_HASH"
                ;;
              *) exit 1 ;;
            esac ;;
          "SOURCE pull")
            test "$4" = "/sdcard/DCIM/O'Brien.jpg"
            /bin/cp "$PHOTO" "$5" ;;
          "TARGET push")
            case "$5" in /sdcard/Download/OmniRecovery-*/phone-transfer.part) ;; *) exit 1;; esac
            /bin/cp "$4" "$TARGET_DIRECTORY/transfer/phone-transfer.part" ;;
          "TARGET shell")
            case "$4" in
              mkdir) /bin/mkdir "$TARGET_DIRECTORY/transfer" ;;
              sha256sum) printf '%s  transfer\n' "$DELIVERY_HASH" ;;
              mv) /bin/mv "$TARGET_DIRECTORY/transfer/phone-transfer.part" "$TARGET_DIRECTORY/transfer/phone-transfer.omnibackup" ;;
              rm) /bin/rm -f "$TARGET_DIRECTORY/transfer/phone-transfer.part" ;;
              rmdir) /bin/rmdir "$TARGET_DIRECTORY/transfer" ;;
              *) exit 1 ;;
            esac ;;
          *) exit 1 ;;
        esac
        "#,
    )?;
    let command = || {
        let mut command = cli(root.path());
        command
            .env("PHOTO", &photo)
            .env("PHOTO_HASH", &photo_hash)
            .env("TARGET_DIRECTORY", &target)
            .env("DELIVERY_HASH", &archive_hash);
        command
    };
    let response = command()
        .args(["browse-android", "SOURCE", "--category", "photos"])
        .output()?;
    assert!(response.status.success(), "{:?}", response);
    let report: Value = serde_json::from_slice(&response.stdout)?;
    assert_eq!(report["partial"], false);
    assert_eq!(report["entries"].as_array().unwrap().len(), 1);
    assert_eq!(report["entries"][0]["path"], "DCIM/O'Brien.jpg");

    let saved = root.path().join("saved.jpg");
    let export = || {
        let mut command = command();
        command
            .args(["export-android-file", "SOURCE", "DCIM/O'Brien.jpg"])
            .arg(&saved);
        command.output()
    };
    let response = export()?;
    assert!(response.status.success(), "{:?}", response);
    assert_eq!(fs::read(&saved)?, fs::read(&photo)?);
    assert!(!export()?.status.success());

    let delivery = |hash: &str| {
        command()
            .arg("send-phone-archive")
            .arg(&archive)
            .args(["TARGET", "--confirm-id", "TARGET"])
            .env("DELIVERY_HASH", hash)
            .output()
    };
    assert!(!delivery(&photo_hash)?.status.success());
    assert!(!target.join("transfer").exists());
    let response = delivery(&archive_hash)?;
    assert!(response.status.success(), "{:?}", response);
    let report: Value = serde_json::from_slice(&response.stdout)?;
    assert_eq!(report["sha256"], archive_hash);
    assert_eq!(
        fs::read(target.join("transfer/phone-transfer.omnibackup"))?,
        fs::read(archive)?
    );
    Ok(())
}
