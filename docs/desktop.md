# Desktop app

OmniRecovery 0.3.0 is a desktop Tauri app backed by the Rust engine. Its Toolbox opens the scan, device, capture, transfer, verification, and activity workflows. Paths are selected in native dialogs and held by the Rust backend. The interface sends a typed operation and a selected source slot over the embedded WebView bridge.

## macOS

1. Open the Universal DMG in `dist/` and drag **OmniRecovery** to **Applications**.
2. Launch **OmniRecovery** by name. Look for the navy icon with a white ring and teal data tiles.
3. Open **Scan media** and select **Try demo** to scan a synthetic raw image containing one PNG and one JPEG. Choose a finding to switch the preview.

| Workflow | Available actions |
| --- | --- |
| Scan media | Choose a raw or Android sparse image, folder, file, mounted volume, or readable phone export. Set media types and scope, review partition origin or file category, and export the findings |
| Devices & storage | View macOS disk, volume and USB metadata. Check the status of Android USB debugging through a local ADB bridge |
| Capture & clone | Save before/after manifests, compare the saved pair, and copy an existing image to a separate physical disk on macOS |
| Compare captures | Select before and after images or export folders. Review hash observations and save the full report |
| Repair media | Choose a PNG or JPEG and supply the original hash when required. Preview a verified derivative and review the repair reason |
| Phone clone | Select files in an existing export folder and create an encrypted archive for the Android companion |
| Forensics, Security and Scan logs | Verify a reference SHA-256, analyze a local sample or text log, and export or restore activity records |
| Sanitize | Prepare a media-specific plan. Destructive execution requires a qualified, dual-approved workflow |

For **Save image** and **Save repaired image**, choose a different directly attached physical drive. macOS resolves the source and destination to their backing disks, including the physical stores of APFS volumes. If the disks overlap, the backing device is virtual or the topology cannot be verified, the app stops before writing media. It re-verifies the source and creates a new destination file through the selected directory handle. Existing output files are protected from overwrite.

The standalone desktop workflows apply to previously acquired images and personal files. Live USB acquisition, filesystem-level deleted-entry recovery, case custody and dual-approved evidence release remain separate milestones. The app is a development build and has no Developer ID notarization.

## Build from source

Use the pinned Rust toolchain. For native dependencies, follow the applicable Tauri prerequisites. The source builds a macOS, Windows or Linux app using the host's native WebView. Windows and Linux require their own runtime qualification. Physical destination enforcement for media saving is implemented on macOS. [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/)

```sh
cargo build --release --locked --manifest-path desktop/Cargo.toml
```

On macOS, add the two Rust targets and build the Universal app and DMG:

```sh
rustup target add x86_64-apple-darwin aarch64-apple-darwin
bash desktop/package-macos.sh
```

Run `cargo test --release --locked --manifest-path desktop/Cargo.toml` on macOS to check that the media save guard rejects a destination on the source disk before reading or writing output. That test does not qualify every storage topology.
