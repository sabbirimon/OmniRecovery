# Device acquisition and recovery

## Collect a device

In the desktop app open **Capture & clone → Acquire from a connected device**. Select Android, iPhone or iPad, or Camera. Choose **Detect devices**, select the authorized target and choose a private destination. The capture is saved in a new directory with a data folder and an `acquisition.json` SHA-256 manifest. Use the captured data folder for the existing before/after comparison.

| Transport | Host requirement | Collection scope |
| --- | --- | --- |
| Android USB | Android SDK Platform-Tools and any OEM USB driver required by the host. Enable USB debugging and approve the host key on the unlocked device | Accessible shared files, with optional camera, downloads, documents, pictures, music or movies scope |
| iPhone or iPad | libimobiledevice tools and working USB pairing. Unlock the device and confirm trust when prompted | The MobileBackup2 service's logical backup scope. The device's backup encryption setting applies |
| PTP camera | gPhoto2 and OS permission to access the USB device | Downloadable camera files exposed by the camera transport |
| USB storage and compatible camera or IoT media | Linux direct block device with read-only protection already active, or an existing regular image file on any platform | Whole observed block stream or whole image bytes, followed by independent output readback |

[Android USB and ADB](https://developer.android.com/tools/adb) · [iOS backup helper](https://github.com/libimobiledevice/libimobiledevice/blob/master/docs/idevicebackup2.1) · [gPhoto2](https://www.gphoto.org/doc/manual/ref-gphoto2-cli.html)

```sh
omnirecovery devices android
omnirecovery acquire android DEVICE_ID /path/to/captures --android-scope camera
omnirecovery devices ios
omnirecovery acquire ios DEVICE_ID /path/to/captures
omnirecovery devices camera
omnirecovery acquire camera usb:001,002 /path/to/captures
omnirecovery clone-image storage.dd /path/to/captures
omnirecovery verify-acquisition /path/to/captures/OmniRecovery-acquisition-EXAMPLE
```

On Linux, `clone-disk /dev/DEVICE /path/to/captures` requires a whole, already read-only block device that the kernel grants the app exclusively. Select the acquisition destination on a separate drive. The app checks capacity and device sequence and refuses short reads. A Linux read-only flag and an independent hardware write blocker serve different purposes. USB acquisition copies active accessible data and verifies the saved bytes twice locally. Live source changes, deleted data, encrypted partitions, and app sandbox access are subject to device and transport limits. Verify acquisition again before an export or handoff.

## Recover and restore

- `undelete-fat32 IMAGE --volume-offset BYTES` inspects a FAT32 volume in a raw image for deleted short entries with an unallocated contiguous span. `recover-fat32` takes the entry offset and source SHA-256 from that report. Supply the original SHA-256 if available to require an exact match. Remaining free bytes can come from a different file, especially after reuse or fragmentation.
- `reconstruct IMAGE PLAN OUTPUT` concatenates operator-supplied raw byte spans in listed order. The plan contains `source_sha256`, `expected_sha256` and a `spans` array with `offset` and `length`. It rejects overlapping or out-of-bounds spans and checks the original file digest before publication.
- `artifacts FILE KIND` reads supported Android and iOS SMS, calls and contact name schemas, SQLite table inventory, or exported WhatsApp text. It accepts standalone SQLite copies and preserves raw timestamp encodings. JSON output can contain private records. SQLite sidecar recovery and native app restore require separate adapters.
- `restore-files ARCHIVE DESTINATION` verifies a password-protected OmniRecovery archive and saves its files in a new capture directory. The Android companion can restore selected files through the platform document picker.

## Cloud and AI

- `cloud drive-trash` lists accessible Drive Trash entries. `cloud drive-download` copies authorized binary Drive files, checking the reported size and SHA-256 when available. Use a token scoped for Drive read access. Drive Android device backups and Google Workspace document export require separate integrations.
- `cloud photos-session` creates a Google Photos Picker session. Open its URL, select the items and use `cloud photos-list` and `cloud photos-download` with the same authorized session. Use a token with the Picker scope. This workflow accesses selected available media. Use the Photos account interface for Photos Trash.
- `ai-triage IMAGE --model MODEL:TAG` connects to an installed model on the local Ollama service. Run Ollama with cloud access disabled for offline use. The provider receives aggregate scan statistics and format hints. For an OpenAI-compatible service supply `--remote-url`, `--allow-remote` and an `OMNIRECOVERY_AI_TOKEN`. Advice is informational. The JSON-line `agent SOURCE` also accepts one `scan` request per invocation with a range up to 32 MiB and returns up to 200 findings. The fixed source is chosen when starting the agent.

[Google Drive API downloads](https://developers.google.com/workspace/drive/api/guides/manage-downloads) · [Google Photos Picker API](https://developers.google.com/photos/picker/reference/rest) · [Ollama FAQ](https://docs.ollama.com/faq)

## Signed custody and approvals

Create individual signing seeds with `custody-keygen`. Protect the seeds and configure a separate JSON object mapping actor labels to their public key hex strings. Use an empty dedicated ledger directory. `custody-append` signs the case, action, artifact length and SHA-256, and previous event digest. Retain the returned head in a separate trusted location. `custody-verify` checks signatures, sequence and the retained head.

`plan-action` creates a firmware or wipe plan that expires after 24 hours. Firmware plans require the image and an independently obtained vendor digest. Use `custody-append` to sign a request and a second identity's approval against the exact plan. `verify-approval` checks both signatures, the plan hash and the ledger checkpoint. Device-level execution and hardware-backed identity remain separate qualification work. [NIST media sanitization guidelines](https://csrc.nist.gov/pubs/sp/800/88/r2/final)

## My devices and Android wireless debugging

Open **My devices** to see detected phones before opening the storage dashboard. A Mac USB tile can appear before Android debugging or iOS trust is available. Select a tile to see the connection state and actions. An authorized Android target offers camera roll, pictures, video, music, documents, downloads, shared files and device-info capture. Message, call, contact, app archive, WhatsApp and calendar shortcuts open an existing export. An authorized iOS target offers a logical backup. Use **Backup history** in **Capture & clone** to list completed acquisition folders and recheck their manifests.

On Android 11 or later, open **Developer options → Wireless debugging → Pair device with pairing code**. In OmniRecovery, choose **My devices → Wireless** and enter the pairing address and temporary code. Then use the different address and port on the main Wireless debugging screen to connect. The host needs recent Platform-Tools, local IPv4 connectivity and mDNS discovery of the phone's TLS service. An offline deployment can use the USB tab and its associated commands.

```sh
omnirecovery pair-android 192.168.1.25:37099
omnirecovery connect-android 192.168.1.25:41673
omnirecovery devices android-wireless
omnirecovery acquire android-wireless DEVICE_ID /path/to/captures --android-scope device-info
```

[Android Wireless debugging](https://developer.android.com/tools/adb#connect-to-a-device-over-wi-fi)

In **Device service**, inspect Android fastboot or iOS Recovery and DFU. The iOS workflow can request entry into Recovery Mode and normal boot from Recovery Mode on a confirmed target. DFU detection is available for diagnostics. Android **Screen mirror** uses the installed scrcpy helper in display-only mode and starts a temporary service on the phone. Use it after forensic collection if the original device state matters.

[libirecovery command reference](https://github.com/libimobiledevice/libirecovery/blob/master/tools/irecovery.c) · [scrcpy control modes](https://github.com/Genymobile/scrcpy/blob/master/doc/control.md)

## Recover and repair directly from the CLI

The desktop phone list refreshes USB presence while **My devices → USB** is open. It retains the current target when the same device is still present.

Use the source SHA-256 from the scan report and the selected finding's media type, offset, length and SHA-256 to write its validated bytes on a headless system.

```sh
omnirecovery recover IMAGE OUTPUT --media jpeg --offset OFFSET --length BYTES \
  --source-sha256 IMAGE_SHA256 --expected-sha256 FINDING_SHA256
omnirecovery repair DAMAGED_FILE --expected-sha256 ORIGINAL_SHA256 --output REPAIRED_FILE
```

The commands preserve the existing source and refuse an existing output path. An unresolved repair produces no derivative.

## Separate tool windows

Click a Toolbox card to open a dedicated desktop window. Scan, repair, record review, acquisition and other tools keep their current sources and results in their own processes. Saved activity and before/after captures remain available across windows.

Open **Chats & records** for a local preview of exported WhatsApp messages or a supported SMS, call or contact database. Use **Phone clone** to package exported chat files and attachments for the Android companion's file restore. Native WhatsApp database merge and app-state restore remain separate work.
