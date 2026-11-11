# Capability matrix

Rows below describe engineering targets. The current [Android development CLI](android-recovery.md) analyzes existing raw and Android sparse images, inventories existing logical exports and compares before/after captures. Support claims for devices will include the tested OS version, architecture, device or image format version, acquisition method, and recovery outcome. See the [validation plan](validation.md).

## Image scan options in the development CLI

| `--storage` | Example source of a supported image |
| --- | --- |
| `memory-card` | Raw SD, microSD or CF image |
| `hdd` | Raw hard disk or partition image |
| `ssd` | Raw SATA or NVMe SSD image |
| `ufs`, `emmc` | Existing readable phone partition dump, or a supported Android sparse image |
| `usb-flash` | Raw thumb-drive image |
| `optical` | Sector image containing readable optical-disc bytes |
| `virtual-disk` | Flat raw export of a virtual disk |
| `auto`, `other` | Unspecified storage or an otherwise supported raw image |

These are provenance labels. `auto` leaves the physical medium unspecified. The scanner identifies the input container and applies the same JPEG/PNG validation across profiles. It accepts regular files. Live media access, RAID assembly, E01, AFF4, QCOW2, VHD/VHDX and VMDK parsing are future work. A raw export is scanned according to the bytes it contains.

Use `--offset` and `--length` to limit carving to a known partition or region without making another image copy. The report includes the exact selected range, absolute logical offsets, skipped bytes inside that range and a SHA-256 of the full input. Filesystem signature hints, if present, are relative to the selected range start. Device discard, firmware remapping and missing encryption keys can limit the bytes visible in an acquired image. [Linux discard documentation](https://man7.org/linux/man-pages/man8/fstrim.8.html), [Android metadata encryption](https://source.android.com/docs/security/features/encryption/metadata)

## Hosts and target devices

| Host or target | Storage or source | Acquisition level | Expected outcome and dependencies | Gate |
| --- | --- | --- | --- | --- |
| Linux desktop, x86_64 first, arm64 candidate | Existing raw image, then blocked HDD, SATA/NVMe SSD, USB, SD/CF | Read-only file import and qualified physical imaging | Complete or explicit partial image. Separately qualify bridge, blocker, sector sizes, device permissions and topology | Phase 1 |
| Windows 10/11 and macOS Intel/Apple Silicon | Existing image, then supported removable and raw media | Portable image import first, host-specific privileged adapter later | Release support requires actual per-OS tests for read-only handles, automount, privilege separation and destination ancestry. OS lifecycle and supported editions are part of release qualification | Phases 1–3 |
| Android target via desktop USB | Shared media and documents, SD, accessible thumbnails and trash | Authorized MTP/PTP or USB debugging with scoped ADB reads | Logical files and accessible cached artifacts. Availability varies with OS, OEM, app version and device state. Capture potential mutations and method limits | Phase 4 |
| Android target via desktop USB | MediaStore, shared app media, authorized backups, logs, SQLite/WAL | Extended logical | Accessible rows, candidates from database remnants, and corroborating metadata. Private messaging stores need authorized app/device access or a supplied export. Device lock and app sandbox rules apply | Phase 4 |
| Android target via desktop | SD/OTG media, eMMC/UFS partition image | Reader with write blocker for removable media, or already authorized raw partition access | Preserve accessible block bytes and infer filesystem only where data is available. Credential-encrypted blocks require legitimate decryption access. No bootloader unlock, root installation, pairing bypass or credential attack is part of acquisition | Phases 1, 4 |
| iOS target via desktop | Trusted local backup, media exposed over supported paired interfaces, Photos SQLite and app data included by backup | Logical, including encrypted backups with operator-supplied password | Backup and media artifacts, recoverable SQLite remnants, and supported recently deleted or cached artifacts when present. Scope depends on device state, OS version and backup options. Conventional paired or backup access does not provide physical blocks from a stock modern iPhone | Phase 5 |
| Android companion host | User-selected content, approved OTG devices and removable storage | Scoped logical access and qualified OTG reads, optional already-authorized root profile | Internal deleted-file recovery is generally unavailable to an ordinary scoped-storage app. A Kotlin companion can access approved volumes and feed cases subject to Android's permission model | Phase 7 |
| iOS companion host | User-imported reports, permitted logical backup data | Sandboxed viewing and logical parsing | Swift client for review and approved export; no raw-device access through the companion app | Phase 7 |
| Offline field kit or isolated lab server | Approved sources and deployment storage | Field-kit physical or image import; lab controlled intake and processing | Qualified Linux boot media and hardware profile, or lab tenant-scoped case processing. Capabilities depend on hardware tests and profile acceptance | Phase 8 release |

Standard `adb` uses a local client/server architecture. The strict air-gap profile requires an audited USB acquisition adapter that avoids IP sockets, or separately acquired image and logical imports. Wireless debugging and generic shell execution are outside the planned workflow. [Android Debug Bridge](https://developer.android.com/tools/adb), [Android file-based encryption](https://source.android.com/docs/security/features/encryption/file-based), [Apple encrypted backups](https://support.apple.com/108353), [Apple Data Protection](https://support.apple.com/guide/security/secf6276da8a/web)

## Storage, formats and recovery

| Capability | Initial approach | Expected result and limits | Gate |
| --- | --- | --- | --- |
| Raw/dd, E01/EWF | Stream raw first. Evaluate isolated EWF adapter for segmented images, compression, range maps and verification | Pin tested EWF variants and round-trip with an independent reader. Bit-for-bit clone semantics apply to fully read data | Phase 1 |
| AFF4, DMG, ISO, VMDK, VHD/VHDX, VDI, QCOW2, basic RAID | Adapter-based read-only import, controlled virtual parent chains and RAID geometry | Validate each format and version separately. Missing parent disks, unsupported compression or unknown RAID order produce explicit limitations. No automatic assembly of ambiguous arrays | Phases 5–6 evaluation |
| MBR, GPT, Apple Partition Map | Parse with bounded ranges, CRC checks where applicable and corroborating signatures | Identify partitions and lost partition candidates. Proposed table reconstruction produces a derivative or overlay | MBR/GPT in Phases 1–2, APM in Phase 5 |
| FAT12/16/32, exFAT, NTFS, ext2/3/4 | Read-only metadata parser or qualified adapter. Validate allocation, directory entries, MFT, inodes, journals and extent maps | Recover names, timestamps and data extents when metadata survives. Reused clusters and stale journal or inode records reduce certainty. NTFS logs, USN and shadow copies need separate tests | Phase 2 |
| F2FS, APFS, HFS+ | Versioned metadata adapters with explicit feature flags | F2FS node/segment recovery and APFS snapshot analysis depend on surviving metadata, keys and filesystem variant. Journal and snapshot handling preserve the source | F2FS Phase 4, APFS/HFS+ Phase 5 |
| XFS, Btrfs, ZFS, YAFFS2, UBIFS | Research and evaluate supported read-only adapters and known-answer images | Best-effort candidates with format-specific constraints. Raw NAND often needs device-specific ECC, OOB layout and controller mapping | Phase 6 evaluation |
| BitLocker, FileVault, LUKS, VeraCrypt, Android FBE/FDE | Detection where supported, explicit user-provided credentials and verified read-only decrypt adapters | Some encrypted volumes lack recognizable signatures. Key availability, hardware binding and device state govern what can be read. No password cracking or device lock bypass | Phases 4–5 |
| JPEG, PNG, MP4, MP3 | Bound signature candidates, validate structure and decode with isolated workers | Report byte ranges and truncation. Preserve false-positive and ambiguous outcomes in validation metrics | Phase 2 |
| HEIF/HEIC, camera RAW, GIF/WebP, MOV/3GP/MKV/AVI, AAC/M4A/FLAC/WAV/OGG/Opus | Format-specific validators, fragment reconstruction, isolated decode and optional reference-assisted repair | Keep original fragments and derivative linkage. Any reconstructed bytes or headers must be identified by provenance. Mark uncertain assembly as partial or unverified | Phase 6 |
| SQLite and app artifacts | Preserve database, WAL and journal separately, then parse cloned snapshots, freelists and unallocated pages | Validate page structure, WAL salts/checksums and committed frame boundaries. Candidates from stale or uncommitted bytes carry separate provenance and a lower confidence classification | Phase 4 |
| Timeline, metadata, keyword search, OCR, perceptual hash and hash sets | Derive from approved case bytes with sandboxed parsers, bounded search and optional local inference | Preserve source and time zone for timestamps. Hash matches and model suggestions remain attributable observations. User-supplied hash sets and local model packages are versioned | Phases 3, 6 |

Flash translation, discard/TRIM, garbage collection, wear leveling, overwrite and secure erase can remove access to prior bytes. A zero-filled read alone cannot identify the cause. Images reflect the bytes exposed by the acquisition path. Findings must distinguish unreadable, overwritten when evidenced, encrypted, zeroed, unsupported and ambiguous regions. [SQLite file and WAL format](https://www.sqlite.org/fileformat.html), [NIST disk imaging test methods](https://www.nist.gov/itl/csd/secure-systems-and-applications/computer-forensics-tool-testing-program-cftt/cftt-2)
