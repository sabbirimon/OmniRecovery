# OmniRecovery requests and progress

The MIT [0.3.3 preview](https://github.com/sabbirimon/OmniRecovery/releases/tag/v0.3.3) adds verified acquisition and the recovery, cloud, AI and custody CLI commands below. The published [0.3.2 release](https://github.com/sabbirimon/OmniRecovery/releases/tag/v0.3.2) includes a macOS universal DMG, Windows x64 desktop and CLI executables, and Linux x64 GUI/CLI packages. The [0.3.0 release](https://github.com/sabbirimon/OmniRecovery/releases/tag/v0.3.0) also includes the Android debug companion.

## 0.3.11 Windows detection and forensic recovery

- **My devices** checks Windows Plug and Play, macOS USB inventory, or Linux sysfs alongside ADB. It distinguishes supported phone presence from debugging authorization. Current Platform-Tools report USB transport explicitly.
- The Windows portable package accepts Platform-Tools and scrcpy beside the executable. [Windows setup](windows-setup.md)
- **Forensics** supports selected Sleuth Kit recovery, TestDisk partition listing, PhotoRec carving, and FQLite CASE report import. [Available adapters and upstream plan](forensic-integrations.md)

## 0.3.10 phone file manager, transfers and forensic analysis

- The phone overview opens a separate file manager for its selected Android device. It supports category views, folder navigation, search, and verified exports. **Device details** opens the extended specifications panel.
- **Phone clone** can send or save an unencrypted archive when both passphrase fields are blank. It checks the transferred archive hash and verifies file contents on restore.
- The updated Android companion verifies and restores both transfer formats. Its own backup creation requires a passphrase.
- **Forensics** runs the installed ALEAPP, iLEAPP, or Sleuth Kit helper against a verified staging copy and saves the resulting reports.

## 0.3.9 concurrent phone workspaces

- **Open separate workspace** starts an independent window assigned to the selected phone. Workspaces have independent browse results, capture selections, and saved Before and After snapshots.
- The assigned phone is rechecked before live browsing or capture. A disconnected workspace does not switch targets.

## 0.3.8 device dashboard, browsing and selective transfer

- Selecting an authorized Android phone opens its device overview with reported specs, available RAM, battery charge, and shared storage. Quick actions keep the selected phone in context.
- Choose or drop individual computer files onto the phone panel to prepare a transfer to that USB phone. The Android companion restores the encrypted archive.

- **My devices** opens live Android shared-file categories on the selected phone. It supports filename filtering and verified single-file export over USB and paired Wi-Fi. The connection dialog explains how to authorize the selected device.
- **Phone clone** offers source and target selection, a flip control, and selective encrypted Android USB transfer or local archive save. The Android companion restores the delivered archive.
- SMS, contacts, and call logs are reviewed from supported exports. See [forensic integration status](forensic-integrations.md) for upstream adapters.

## 0.3.6 storage, Android companion and forensic adapters

- Storage filters, capacity charts and device specifications are available in the desktop workspace. Android screen, camera and audio preview uses a dedicated scrcpy window over an authorized USB or paired Wi-Fi connection.
- The Android companion can create encrypted backups of selected files, verify archives without restoring, and restore compatible file archives.
- ALEAPP and iLEAPP can parse staged exports through separately installed, digest-checked entrypoints. The Sleuth Kit adapter lists deleted entries from a verified raw image copy.
- [Illustrated guide](visual-guide.md#browse-drives-and-phone-storage) · [Integration plan and remaining adapters](forensic-integrations.md)

## 0.3.5 recovery and desktop workspace

- Toolbox cards open independent tool windows for scan, repair, phone capture, record review and other operations. Current selections and running jobs belong to each window. Saved records remain available across windows.
- The USB dashboard refreshes while open. The CLI can save verified scan findings and repair derivatives directly without replacing an existing destination.
- **Chats & records** previews supported exported SMS, call and contact databases and WhatsApp text. Native chat migration and full app-state restore remain open.

## [0.3.4](https://github.com/sabbirimon/OmniRecovery/releases/tag/v0.3.4) device workspace

- **My devices** shows phone tiles and status, sidebar model shortcuts and per-device actions. On macOS it recognizes Samsung and Xiaomi/Redmi USB entries even before ADB authorization. Device capture rechecks the chosen target.
- Android USB and Wireless debugging offer shared-file categories and device-info capture. Wireless pairing uses the phone's current six-digit code and an advertised local TLS service.
- Backup history lists completed captures and verifies their contents. Device service offers Android fastboot detection, display-only mirroring, iOS Recovery/DFU detection and Recovery Mode entry/exit requests for the selected target.
- Native messaging-app migration, iOS wireless pairing, full OS repair, firmware and wipe execution, complete device restore, lock bypass and credential extraction remain open. [Device workspace guide](acquisition-and-recovery.md#my-devices-and-android-wireless-debugging)

## 0.3.3 implementation and remaining work

| Area | Implemented in 0.3.3 | Remaining |
| --- | --- | --- |
| USB acquisition | Android shared storage, iOS MobileBackup2 backups and PTP camera collection via platform helpers. Desktop selector and CLI. Verified destination manifests and Linux whole-block capture | Physical phone and camera validation, OEM driver packaging, per-model database and proprietary IoT protocols, privileged phone partition acquisition and macOS/Windows live block cloning |
| Deeper recovery | FAT32 deleted entries with contiguous hypotheses, trusted-hash reconstruction of supplied fragments, structured SMS/call/contact export reading and WhatsApp text import | Automatic fragment discovery, other filesystem undelete, deleted database pages, private encrypted messenger and mail formats, broader carving and repair |
| Backup, cloud and firmware | Authenticated OmniRecovery restore on desktop, Drive Trash listing and blob copy, Photos Picker and download, firmware and wipe approval plans | Full-device or app-state restore, flashing and wipe execution, OS repair, cloud device backups, and live OAuth qualification |
| AI and custody | Local Ollama, explicit HTTPS provider, bounded JSON-line scan API, individual Ed25519 case events and independently anchored chain verification | Hardware-backed credentials, production remote job service and mTLS, case vault, classified workflow enforcement and accreditation |
| Packaging | macOS, Windows x64, Linux x64 DEB and standalone CLI build recipes | Physical Windows GUI, ARM Linux and RPM packages, server deployment and USB passthrough qualification |

[Device acquisition and recovery](acquisition-and-recovery.md) · [Dr.Fone screenshot comparison](drfone-feature-inventory.md#follow-up-screenshots-and-omnirecovery-coverage)

The tables below retain the earlier request inventory and design context. The 0.3.3 table records the latest implementation.

## Product and recovery

| Requested capability | Progress and next scope |
| --- | --- |
| Full GUI, original logo, guided and expert modes | Tauri Toolbox, original logo, device dashboard, selected-finding details, confidence, byte ranges and hex inspector are implemented. Improve the persistent device/source panel, category icons, accessibility and localization. Use the [Dr.Fone](drfone-feature-inventory.md) and [3uTools](3utools-feature-inventory.md) inventories as workflow references |
| Storage and image coverage | Raw and Android sparse file import, storage labels for SD/microSD/CF, HDD, SSD/NVMe, USB, UFS, eMMC and other readable images, MBR/GPT provenance, and macOS mounted-volume catalogue. Live block acquisition, RAID, E01/EWF, AFF4, DMG, ISO, VMDK, VHD/VHDX, VDI, QCOW2 and Apple Partition Map adapters remain pending |
| Photos, video, audio, documents and other files | 0.3.2 carves contiguous JPEG, PNG, supported MP4/MOV video and PCM WAV from images. It catalogues broader readable formats by name. HEIC, RAW, GIF/WebP, 3GP/MKV/AVI, MP3/AAC/M4A/FLAC/OGG/Opus, document carving, fragmented files, perceptual deduplication, OCR and broader playback/preview remain pending |
| Deleted, lost, active, formatted or flashed data | Current reports distinguish listed partitions, backup GPT candidates, unassigned image bytes and active readable files. Before/after pairs compare hashes. Deleted-entry reconstruction, surviving data after format/flash and original paths/timestamps require filesystem-aware recovery and recoverable source bytes. Encryption, discard, overwrite and missing blocks limit recovery |
| Filesystems and encryption | Detection and geometry hints are available where implemented. Pending metadata parsers cover FAT, exFAT, NTFS, ext, F2FS, APFS and HFS+, with exploratory XFS, Btrfs, ZFS, YAFFS2 and UBIFS support. Authorized key-based adapters are pending for BitLocker, FileVault, LUKS, VeraCrypt and Android FBE/FDE |
| Corrupt-file repair | 0.3.2 handles supported JPEG and PNG damage, particular MP4/MOV size fields, PCM WAV size, byte rate and alignment fields, and certain Office ZIP index damage in DOCX/XLSX/PPTX. Reference-based video reconstruction, media payload repair, PDF and other formats are pending |
| Phone data and private apps | 0.3.1 catalogues readable Android/iOS export filenames for contacts, SMS, calls, mail, chat history, WhatsApp, Messenger, browser and account exports, device info/IMEI files, documents and logs. Structured parsing, authorized database access, SQLite/WAL and app-specific recovery for WhatsApp/Business, Messenger, Telegram, Signal and other apps are pending. Exported browser records are distinct from authenticated sessions |
| Google Drive, Google Photos and WhatsApp | Pending independent workflows for authorized Drive backup import, Google Photos library/export import and provider-supported restore of Trash items, and WhatsApp/Business review of readable exports and authorized supported backups. Track Drive Android backup, Drive WhatsApp backup and Google Photos as different sources. Recovery depends on what the provider or a retained copy makes available. [Dr.Fone cloud and chat reference](drfone-feature-inventory.md) |

## Devices, backup, and service

| Requested capability | Progress and next scope |
| --- | --- |
| Strong device detection and database | macOS USB/disk inventory and local ADB status checks exist. Pending work includes a versioned VID/PID and model/variant database with official provenance, OS/firmware compatibility, multiple-device selection, stable device identity through reconnects, explicit transport state and operator-visible access level. Record USB presence separately from pairing, authorization and acquisition compatibility |
| Built-in device support by platform | Plan macOS and Windows/Linux device adapters for Android, iOS, USB storage and card readers, plus qualified adapters for other devices. Cover OS driver and SDK presence, Windows OEM driver setup, Linux permissions, iOS trust and supported backup/media adapters, and offline distribution of redistributable signed components. Bundle availability and a connected device do not establish support for a live acquisition. [Android driver requirements](https://developer.android.com/studio/run/oem-usb) · [Apple device connection](https://support.apple.com/en-us/108643) |
| Android and iOS acquisition | Pending approved ADB/MTP/PTP logical reads, scoped extended Android data collection, authorized Android partition acquisition, iOS backup parsing/acquisition and media access. Android companion restores ordinary files. Android OTG/SD recovery and an iOS report companion remain pending |
| First capture, later capture and full clone | 0.3.1 shows explicit Before and After workflow and supports case-tagged forensic scans with byte offset, length and media filter. It verifies a copy of an already acquired image to a different physical disk on macOS. Live disk/device/partition cloning, sector retry maps, write-blocking, resumable imaging and full device recovery remain pending |
| Restore saved backups and snapshots | The Android companion restores encrypted `.omnibackup` file archives created from PC folders or readable phone exports. Before/After snapshots are comparison manifests. Pending work includes a backup library, restore preview, target identity and compatibility checks, selective or full restore of supported data, and verified restoration from actual retained images or backups |
| Firmware flash | Pending separate device-service workflow for supported signed firmware. Track model/variant, current and target OS, package hash/signature, allowed recovery mode, effect on user data, and before/after logs. Treat firmware installation, app-level file restore and raw cloning as separate operations. Preserve the source before firmware changes and apply the original authorization requirements |

## Forensics, AI, and enterprise deployment

| Requested capability | Progress and next scope |
| --- | --- |
| Forensic metrics, reports and logs | 0.3.1 has typed range scans, two-pass verification, SHA-256, scope, timing, finding counts, JSON exports, macOS image-copy readback, local activity logs, export, soft delete and restore. Signed chain of custody, case vault, timelines, EXIF/GPS, keyword/regex on raw ranges, file slack, SQLite artefacts, hash sets, multiple digest algorithms, PDF/HTML/CSV/UCO/CASE/VERIS and third-party forensic reporting are pending |
| Malware and sample submission | Local ClamAV inspection and text log indicators are available with a supported scanner installation. Third-party sample/hash lookup, VirusTotal-style submission and segregated handling by case policy are pending |
| Secure wipe | Current Sanitize workflow produces a review plan. Qualified media-specific clear/purge/destroy execution, evidence-free target verification, dual approval, and readback or controller verification remain pending |
| Local, NAS, remote and cloud export | JSON and archives can go to selected local, mounted network or synced paths. Verified media copies on macOS require a separate directly attached physical disk. Managed NAS/object-store connectors, explicit export approvals, classification, retention and recipient enforcement remain pending |
| Local or remote LLM, API and autonomous workflows | 0.3.2 classifies scan results with an offline deterministic algorithm and exposes its counts in the GUI and JSON. The Automation view documents a local read-only CLI JSON-lines interface for scoped filename search and verification. Local or remote models, deeper agent inspection, third-party source adapters, versioned IPC/REST or gRPC integration, job cancellation, quotas and deterministic replay remain pending. Dr.Fone host-process memory inspection was a reference activity. [Automation design](automation.md) |
| Linux GUI and CLI | Publish native Tauri GUI and standalone CLI packages. Target DEB for Ubuntu, Debian, Kali and Parrot. Target RPM for Fedora and Red Hat-family distributions, then qualify other releases. Evaluate AppImage and Flatpak for portable use. Track x86-64 and ARM64 builds, WebKitGTK requirements, install/upgrade checks, and per-distro USB behavior separately. Ubuntu CI build coverage is the current Linux milestone |
| Server, cloud and VM workstations | Pending Linux headless CLI builds and service packaging for servers, cloud instances and isolated lab runners. Provide a full GUI on supported VM workstations and equivalent CLI operation for scripted jobs. Qualify CPU architectures, storage and USB passthrough separately. Plan multi-user case storage, authenticated job API, VM images or container builds where appropriate, and a centrally managed enterprise distribution. Device access from a VM or cloud host requires a qualified adapter or an imported capture |
| High-assurance profiles | Design documents cover named RBAC and MFA, dual approvals, signed append-only custody, classification and tenant isolation, legal hold and retention, offline/lab profiles, TLS 1.3 and mTLS, AES-256-GCM, validated FIPS 140-3 modules, HSM/TPM/smartcard keys and hybrid post-quantum readiness. Deployment enforcement and independent validation are pending |
| Testing, signing and certification | Synthetic tests, CI, STRIDE model and control mapping are available. Pending work includes device and blocker qualification, fuzzing and sandboxing, benchmark and ground-truth corpora, reproducible signed packages and offline updates, SBOM and SLSA provenance, dependency auditing, hardening, independent penetration tests and formal certification. [Validation plan](validation.md) · [Assurance](assurance.md) · [Roadmap](roadmap.md) |

## Verified milestones

- Public 0.3.0 assets and SHA-256 checksums were verified. Native Windows CI built the GUI and executed the CLI version check. Hands-on Windows GUI testing remains pending.
- The 0.3.1 Mac build passed 25 engine tests and a desktop test for same-drive output rejection. The four-format demo, scoped forensic scan, WAV repair, and export categories were verified in the Mac GUI. Source and destination behavior on physical devices still require hardware qualification.
- The 0.3.2 scanner uses vectorized signature search and hashes complete raw images during the scan pass. Recovery hashes the whole source and collects the selected range from the same read. In three interleaved runs on an Intel Mac and a 384 MiB synthetic raw image, scan median changed from 4.30 to 3.24 seconds, and recovery median changed from 3.16 to 1.61 seconds. Findings, source SHA-256 and recovered fixture bytes matched. These results describe the fixture and host used for the comparison.
- Android companion archive interoperability was verified on the JVM and the debug APK signature was checked. A physical phone restore still needs validation.
- Linux packaging, server deployment, AI providers, a device database, firmware flashing and cloud backup integrations remain pending.
