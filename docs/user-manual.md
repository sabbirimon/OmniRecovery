# User manual

## Install and start

| Platform | Package | Start |
| --- | --- | --- |
| macOS | Universal `.dmg` | Drag the app to Applications. Open **OmniRecovery** and select **Toolbox → Scan media → Try demo** |
| Windows x64 | Desktop `.exe` | Start the desktop executable on a host with WebView2. The separate CLI executable emits JSON |
| Linux x64 | Desktop and CLI `.deb`, or standalone CLI `.tar.gz` | Install the Debian package and open **OmniRecovery** from the desktop menu, or run the standalone `omnirecovery` CLI |
| Android | Debug companion `.apk` | Open **OmniRecovery** to create, verify, or restore encrypted file archives |

See the [visual guide](visual-guide.md) for navigation. Use trusted, immutable inputs for evidence examination. The desktop distribution is a development build. The macOS bundle has no Developer ID notarization and the Android APK is debug signed.

## Scan, preview and recover

1. Open **Scan media** and choose **Disk image**, **Folder**, **Single file**, **Active mounted volume**, or a local phone export. For a mounted volume on macOS, select **Scan files** from **Devices & storage**.
2. Choose the desired file types and optional filename search. Folder and phone-export catalogues recognize photos, videos, audio, documents, logs, contacts, message and call exports, databases, account exports, browser exports, archives and other files by name or extension. Select a file to calculate its SHA-256 and preview a verified standalone JPEG or PNG when available.
3. For disk images, choose **All supported media**, **JPEG**, **PNG**, **MP4 video**, **MOV video**, or **WAV audio (PCM)**. Optionally set a byte range and start the scan. Review offsets, SHA-256, verification reasons and partition origin. The offline triage counts separate decode-verified photos, container-verified media, candidates cut short at a source boundary, rejected structures and validation limits. Listed MBR and CRC-verified GPT partitions are shown when available. A backup-only GPT entry is a partition candidate. A file's presence in the image cannot establish a prior deletion by itself.
4. On macOS, **Save recovered file** or **Recover a copy** verifies a destination on a different directly attached physical disk. A saved copy is verified by reading it back. **Save report** exports JSON findings. Destination media may be a local disk or external drive when the separate-disk check passes. For reports and archives, a mounted NAS or cloud-synced folder can also be selected in the system dialog. Its provider handles any synchronization.

Image carving uses contiguous observed bytes. MP4 and MOV support covers selected conventional non-fragmented files with a video track and one media-data box. MOV detection requires a QuickTime file-type atom. The reported range ends with the core movie or media-data box, so optional trailing boxes may be omitted. WAV support covers integer PCM in RIFF. The app checks container structure and sample bounds and displays verification details without playing the file. The limits are 16 MiB per photo candidate and 128 MiB per video or WAV candidate. Use the Sleuth Kit adapter to list deleted entries on supported file systems. Fragmented file reconstruction requires additional support.

## Capture, compare and clone

Open **Capture & clone → Before & after**, select a folder or an existing image, then choose **Capture before**. When the later state is available, select it and choose **Capture after**. **Compare saved captures** compares that saved pair, including after reopening the app. In **Compare captures** you can also choose two independently prepared images or export folders. A changed file hash indicates changed bytes. Differences in scope, missing regions or unavailable files can also produce before-only and after-only observations.

**Connected device** captures authorized Android files, iOS logical backups, or supported PTP camera files. **Backup history** verifies completed captures in the selected output folder.

Folder snapshots store relative paths and hashes. Image snapshots store the source hash and validated media observations. **Clone image file** copies the bytes of an existing image to a verified separate drive on macOS. The app does not acquire a physical image by choosing that action.

## Android and iOS

Open **Device service** and select **Android** or **iPhone & iPad**.

- **Android service** detects USB and paired Wi-Fi debugging connections. Select a phone to read specifications or open **Capture & clone** with that phone selected. **Detect fastboot** checks for a device already in fastboot mode. **Open Screen mirror** opens a separate scrcpy window with the current transport selected.
- **iPhone & iPad service** separates normal-mode USB detection from Recovery and DFU detection. Unlock the device and approve its trust prompt for a logical backup. In normal mode, select the device and choose **Capture iOS backup** or **Enter Recovery Mode**. A device detected in Recovery can be selected for **Exit Recovery Mode**. DFU reports its status. Entering or exiting Recovery restarts the device.
- **Firmware preflight** verifies a selected firmware file against the vendor SHA-256 you supply. It saves a review plan bound to the case, exact device model and target identifier.

Android uses Android SDK Platform-Tools. Enable USB debugging on the phone and approve its fingerprint, or pair it under **My devices → Wireless**. iOS USB workflows use libimobiledevice and libirecovery. [Android debugging guide](https://developer.android.com/tools/adb) · [libimobiledevice](https://github.com/libimobiledevice/libimobiledevice) · [libirecovery](https://github.com/libimobiledevice/libirecovery)

In **Forensics → Review phone exports**, choose Android or iOS, select your export folder, and select categories such as SMS and messages, call history, contacts, mail, chat exports, WhatsApp, Messenger, or device info and IMEI files. These are filename and path hints. The catalogue lists and hashes selected files. It does not extract individual messages, read an IMEI directly from a phone, decrypt private app backups, or bypass phone permissions.

### Browse a connected Android phone

Open **My devices**, select the phone, and click **Photos**, **Videos**, **Music**, **Documents**, **Downloads**, or **Files**. Files appear on the same page. Use **Filter filenames** or choose **Save copy** on an individual result. The export compares the copy with a SHA-256 digest from the phone.

If the connection needs authorization, open **Connection setup** and complete the device prompt. Shared-file browsing uses Android USB debugging or an already paired Wireless debugging connection. It lists standard shared folders and reports when the listing reaches its limit. For private records such as SMS, calls, contacts, and chat histories, use a readable export or supported backup.

### Transfer selected phone files

1. From the selected phone, choose **Phone clone**. You can also open it from the sidebar and select the source manually.
2. Choose the target Android USB phone or **Save archive to this computer**. **Flip** swaps two selected USB phones.
3. Select the shared folders to transfer. If the source is a local export, use **Review files** to select individual items.
4. Enter an archive passphrase and start the transfer. For a connected target, OmniRecovery compares the transferred archive's SHA-256 with the desktop copy.
5. In the Android companion on the target, select the archive from **Downloads** and choose **Verify and restore files**. The companion asks for the passphrase and output folder.

The Android companion shipped in 0.3.6 reads the same archive format. [Transfer format and setup](repair-and-transfer.md)

## Verification, security and logs

- **Forensics** verifies a file in two read passes, scans an acquired image using optional byte offset, length and media arguments, and clones an acquired image to a separate physical drive on macOS. Add a case identifier for the unsigned JSON report and local activity summary. The image scan reports coverage, elapsed time, findings, rejections and source SHA-256. The clone reports bytes, elapsed time and readback SHA-256. Live-device acquisition requires a separate workflow.
- **Repair files** handles specific JPEG and PNG damage, selected PCM WAV header fields, a final `moov` or `mdat` box size in supported MP4 or MOV, and the ZIP directory size or offset of supported DOCX, XLSX and PPTX packages. It assesses files up to 16 MiB and requires a trusted original SHA-256 except for the supported PNG end-marker repair. Missing media samples, corrupt Office XML and PDFs are outside this repair scope.
- **Security scan** runs a selected sample through a locally installed ClamAV with its existing signature database. Alternatively, review a UTF-8 text log for supported security indicators. A no-detection result is limited to the engine and signatures used. Public scanning services may share sample files, so handle any submission separately from the app. [ClamAV scanning guide](https://docs.clamav.net/manual/Usage/Scanning.html), [VirusTotal private scanning](https://docs.virustotal.com/docs/private-scanning)
- **Scan logs** records timing, scope and outcome of scans, captures, security checks and verification. Select a record to export JSON. **Delete log** moves it to **Deleted logs**, where **Restore log** brings it back. These local activity records are separate from a future signed custody ledger.
- **Sanitize** prepares a media-specific plan for review. The destructive step requires an evidence-free target, qualified method, and dual authorization. SHA-256 may document readback verification after an applicable overwrite. [NIST SP 800-88 Rev. 2](https://csrc.nist.gov/pubs/sp/800/88/r2/final)

## Local automation

On macOS, call `/Applications/OmniRecovery.app/Contents/MacOS/omnirecovery-cli`. On Windows, use the companion CLI executable. Commands such as `catalog PATH --query DCIM`, `verify FILE`, and `scan IMAGE` emit JSON.

`agent PATH` accepts one JSON request per line with a scope set by the launch path. It serves read-only `search` and `verify` requests and accepts up to 1,024 requests of up to 4 KiB per session. It exposes no network listener.

```json
{"operation":"search","query":"DCIM","category":"photo"}
{"operation":"verify","path":"DCIM/IMG_0001.jpg"}
```

Review the [security design](architecture.md) before connecting an external agent to case data. Use only sources you have permission to inspect.

## Android companion backups

Install the APK for the matching preview from GitHub Releases. In **Backup & restore**, choose **Create a phone file backup**, select the files through Android’s picker, and enter the same passphrase in both fields. Choose **Save encrypted backup** and select the destination. The companion verifies the staged archive and rechecks the saved file before reporting success.

To check an archive received from another device, choose it and select **Verify without restoring**. To restore, choose an output folder and select **Verify and restore files**. Android creates a new destination folder. On the desktop, `omnirecovery restore-files ARCHIVE DESTINATION` reads the same archive format.

The companion operates on files you select in Android’s picker. Whole-device images, protected application state and desktop forensic adapters require the supported desktop workflows.

[Illustrated storage and mirroring guide](visual-guide.md#browse-drives-and-phone-storage) · [Open-source parser and filesystem integrations](forensic-integrations.md)

## Workspace settings

Open **Settings** to choose separate Toolbox windows, compact navigation, USB refresh and its interval, the default Android capture scope, and separate USB and Wi-Fi mirror quality presets. **Restore defaults** resets these preferences for the workspace.
