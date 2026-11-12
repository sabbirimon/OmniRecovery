# User manual

## Install and start

| Platform | Package | Start |
| --- | --- | --- |
| macOS | Universal `.dmg` | Drag the app to Applications. Open **OmniRecovery** and select **Toolbox → Scan media → Try demo** |
| Windows x64 | Desktop `.exe` | Start the desktop executable on a host with WebView2. The separate CLI executable emits JSON |
| Android | Debug companion `.apk` | Install the test APK on the receiving phone and open **OmniRecovery**. It restores encrypted transfer archives to user-selected storage |

See the [visual guide](visual-guide.md) for navigation. Use trusted, immutable inputs for evidence examination. The desktop distribution is a development build. The macOS bundle has no Developer ID notarization and the Android APK is debug signed.

## Scan, preview and recover

1. Open **Scan media** and choose **Disk image**, **Folder**, **Single file**, **Active mounted volume**, or a local phone export. For a mounted volume on macOS, select **Scan files** from **Devices & storage**.
2. Choose the desired file types and optional filename search. Folder and phone-export catalogues recognize photos, videos, audio, documents, logs, contacts, message and call exports, databases, account exports, browser exports, archives and other files by name or extension. Select a file to calculate its SHA-256 and preview a verified standalone JPEG or PNG when available.
3. For disk images, choose **All supported media**, **JPEG**, **PNG**, **MP4 video**, **MOV video**, or **WAV audio (PCM)**. Optionally set a byte range and start the scan. Review offsets, SHA-256, verification reasons and partition origin. The offline triage counts separate decode-verified photos, container-verified media, candidates cut short at a source boundary, rejected structures and validation limits. Listed MBR and CRC-verified GPT partitions are shown when available. A backup-only GPT entry is a partition candidate. A file's presence in the image cannot establish a prior deletion by itself.
4. On macOS, **Save recovered file** or **Recover a copy** verifies a destination on a different directly attached physical disk. A saved copy is verified by reading it back. **Save report** exports JSON findings. Destination media may be a local disk or external drive when the separate-disk check passes. For reports and archives, a mounted NAS or cloud-synced folder can also be selected in the system dialog. Its provider handles any synchronization.

Image carving uses contiguous observed bytes. MP4 and MOV support covers selected conventional non-fragmented files with a video track and one media-data box. MOV detection requires a QuickTime file-type atom. The reported range ends with the core movie or media-data box, so optional trailing boxes may be omitted. WAV support covers integer PCM in RIFF. The app checks container structure and sample bounds and displays verification details without playing the file. The limits are 16 MiB per photo candidate and 128 MiB per video or WAV candidate. Deleted directory entries, fragmented file assembly and direct acquisition from cameras, IoT devices and phones require additional support.

## Capture, compare and clone

Open **Capture & clone**, select a folder or an existing image, then choose **Capture before**. When the later state is available, select it and choose **Capture after**. **Compare saved captures** compares that saved pair, including after reopening the app. In **Compare captures** you can also choose two independently prepared images or export folders. A changed file hash indicates changed bytes. Differences in scope, missing regions or unavailable files can also produce before-only and after-only observations.

Folder snapshots store relative paths and hashes. Image snapshots store the source hash and validated media observations. **Clone image file** copies the bytes of an existing image to a verified separate drive on macOS. The app does not acquire a physical image by choosing that action.

## Android and iOS

**Devices & storage** shows USB product names supplied by macOS. For Android, install Android SDK Platform-Tools and select **Check Android USB debugging** to start the local ADB bridge. On Windows, OmniRecovery looks in the Android Studio SDK location or in the SDK directory selected by `ANDROID_HOME` or `ANDROID_SDK_ROOT`. Unlock the phone, enable USB debugging using its Developer options, connect with a data cable, and approve the displayed fingerprint on the phone if prompted. The app reports authorization status. Import a readable local export to inspect files. For iOS, use an existing readable export folder. [Android USB debugging guide](https://developer.android.com/tools/adb), [Android storage access](https://developer.android.com/training/data-storage/shared/documents-files)

In **Forensics → Review phone exports**, choose Android or iOS, select your export folder, and select categories such as SMS and messages, call history, contacts, mail, chat exports, WhatsApp, Messenger, or device info and IMEI files. These are filename and path hints. The catalogue lists and hashes selected files. It does not extract individual messages, read an IMEI directly from a phone, decrypt private app backups, or bypass phone permissions.

For **Phone clone**, choose the local folder, click **Review files**, select the files and create a passphrase-protected `.omnibackup`. On the receiving Android device, select the archive and a destination in the companion app. The app verifies the archive before writing and checks each restored file. This copies the exported files, including supported account or browser-export records. The receiving apps handle account sign-in and importing their exported records. [Transfer format and setup](repair-and-transfer.md)

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
