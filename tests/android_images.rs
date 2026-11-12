use std::fs;
use std::io::Cursor;
use std::process::Command;

use image::{DynamicImage, ImageFormat, Rgb, RgbImage};
use omnirecovery::{
    MediaType, ScanOptions, SkippedKind, SkippedRange, StorageProfile, compare, compare_exports,
    inspect_image_bytes, inventory, recover_candidate, scan, scan_selected_media,
    scan_with_options,
};
use sha2::{Digest, Sha256};
use tempfile::tempdir;

fn photo(format: ImageFormat, color: [u8; 3]) -> Vec<u8> {
    let mut encoded = Cursor::new(Vec::new());
    DynamicImage::ImageRgb8(RgbImage::from_pixel(4, 3, Rgb(color)))
        .write_to(&mut encoded, format)
        .unwrap();
    encoded.into_inner()
}

fn sparse_header(block_size: u32, blocks: u32, chunks: u32, checksum: u32) -> Vec<u8> {
    let mut bytes = vec![0x3a, 0xff, 0x26, 0xed, 1, 0, 0, 0, 28, 0, 12, 0];
    for field in [block_size, blocks, chunks, checksum] {
        bytes.extend(field.to_le_bytes());
    }
    bytes
}

fn sparse_chunk(bytes: &mut Vec<u8>, kind: u16, blocks: u32, payload: &[u8]) {
    bytes.extend(kind.to_le_bytes());
    bytes.extend(0_u16.to_le_bytes());
    bytes.extend(blocks.to_le_bytes());
    bytes.extend((12 + payload.len() as u32).to_le_bytes());
    bytes.extend(payload);
}

#[test]
fn scans_raw_across_window_and_recovers_exact_bytes() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("userdata.raw");
    let jpeg = photo(ImageFormat::Jpeg, [16, 40, 200]);
    let png = photo(ImageFormat::Png, [120, 70, 30]);
    let png_at = 1024 * 1024 - 4;
    let jpeg_at = png_at + png.len() + 3;
    let mut raw = vec![0; png_at];
    raw[1080..1082].copy_from_slice(&[0x53, 0xef]);
    raw.extend(&png);
    raw.extend([1, 2, 3]);
    raw.extend(&jpeg);
    fs::write(&path, &raw).unwrap();

    let report = scan(&path).unwrap();
    assert_eq!(report.source_sha256, format!("{:x}", Sha256::digest(&raw)));
    assert_eq!(
        report.filesystem_signature,
        Some("ext_family_magic_at_1080")
    );
    assert_eq!(report.observed_bytes_scanned, raw.len() as u64);
    assert!(report.observed_ranges_fully_scanned);
    assert_eq!(report.findings.len(), 2);
    assert_eq!(report.findings[0].offset, png_at as u64);
    assert_eq!(report.findings[0].media_type, MediaType::Png);
    assert_eq!(report.findings[1].offset, jpeg_at as u64);
    assert_eq!(report.findings[1].media_type, MediaType::Jpeg);
    for (finding, original) in report.findings.iter().zip([png, jpeg]) {
        assert_eq!(
            (finding.width, finding.height, finding.confidence),
            (4, 3, 3)
        );
        assert_eq!(finding.sha256, format!("{:x}", Sha256::digest(&original)));
        assert_eq!(
            recover_candidate(&path, &report.source_sha256, finding).unwrap(),
            original
        );
    }
    let png_only =
        scan_selected_media(&path, ScanOptions::default(), Some(MediaType::Png)).unwrap();
    assert_eq!(png_only.media_filter, Some(MediaType::Png));
    assert_eq!(png_only.findings, vec![report.findings[0].clone()]);
    assert_eq!(
        inspect_image_bytes(&path, &report.source_sha256, png_at as u64, 8).unwrap(),
        b"\x89PNG\r\n\x1a\n"
    );
    assert_eq!(fs::read(path).unwrap(), raw);
}

#[test]
fn sparse_chunks_preserve_logical_offsets_and_never_bridge_a_hole() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("userdata.sparse");
    let png = photo(ImageFormat::Png, [15, 25, 35]);
    let mut first = vec![0; 512 - 5];
    first.extend_from_slice(&png[..5]);
    let mut second = png[5..].to_vec();
    second.resize(512 - 12, 0);
    second.extend_from_slice(&png[..12]);
    let mut last = png[12..].to_vec();
    last.resize(512, 0);
    let fill_pattern = [0x89, 0x50, 0x4e, 0x47];
    let expanded = [
        [0; 512].as_slice(),
        &first,
        &second,
        &fill_pattern.repeat(128),
        &last,
    ]
    .concat();
    let crc = crc32fast::hash(&expanded);
    let mut bytes = sparse_header(512, 5, 6, crc);
    sparse_chunk(&mut bytes, 0xcac3, 1, &[]);
    sparse_chunk(&mut bytes, 0xcac1, 1, &first);
    sparse_chunk(&mut bytes, 0xcac1, 1, &second);
    sparse_chunk(&mut bytes, 0xcac2, 1, &fill_pattern);
    sparse_chunk(&mut bytes, 0xcac1, 1, &last);
    sparse_chunk(&mut bytes, 0xcac4, 0, &crc.to_le_bytes());
    fs::write(&path, &bytes).unwrap();

    let report = scan(&path).unwrap();
    assert_eq!(report.container, "android_sparse_v1");
    assert_eq!(report.logical_bytes, 2560);
    assert_eq!(report.observed_bytes_scanned, 1536);
    assert_eq!(report.fill_bytes_skipped, 512);
    assert_eq!(report.unspecified_bytes_skipped, 512);
    assert_eq!(
        report.skipped_ranges,
        [
            SkippedRange {
                offset: 0,
                length: 512,
                kind: SkippedKind::SparseDontCare
            },
            SkippedRange {
                offset: 1536,
                length: 512,
                kind: SkippedKind::SparseFill
            },
        ]
    );
    assert_eq!(report.sparse_crc, "verified");
    assert!(inspect_image_bytes(&path, &report.source_sha256, 0, 16).is_err());
    assert_eq!(report.findings.len(), 1);
    assert_eq!(report.findings[0].offset, 1019);
    assert_eq!(report.rejected.len(), 1);
    assert_eq!(
        report.rejected[0].reason,
        "ends_at_unobserved_range_or_container_boundary"
    );
    assert_eq!(
        recover_candidate(&path, &report.source_sha256, &report.findings[0]).unwrap(),
        png
    );
    let selected = scan_with_options(
        &path,
        ScanOptions {
            storage: StorageProfile::Ufs,
            offset: 256,
            length: Some(1536),
        },
    )
    .unwrap();
    assert_eq!(selected.storage_profile, StorageProfile::Ufs);
    assert_eq!(selected.observed_bytes_scanned, 1024);
    assert_eq!(selected.unspecified_bytes_skipped, 256);
    assert_eq!(selected.fill_bytes_skipped, 256);
    assert_eq!(selected.skipped_ranges[0].offset, 256);
    assert_eq!(selected.skipped_ranges[1].offset, 1536);
    assert_eq!(selected.findings, report.findings);
    assert_eq!(selected.source_sha256, report.source_sha256);
    let checksum_byte = bytes.len() - 1;
    bytes[checksum_byte] ^= 1;
    fs::write(&path, bytes).unwrap();
    assert!(scan(&path).unwrap_err().to_string().contains("CRC chunk"));
}

#[test]
fn storage_scan_limits_carving_to_requested_bytes() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("card.img");
    let png = photo(ImageFormat::Png, [1, 9, 7]);
    let mut bytes = png.clone();
    bytes.resize(2048, 0);
    bytes.extend_from_slice(&png);
    fs::write(&path, &bytes).unwrap();
    let exe = env!("CARGO_BIN_EXE_omnirecovery");
    let scan = Command::new(exe)
        .arg("scan")
        .arg(&path)
        .args([
            "--storage",
            "memory-card",
            "--offset",
            "2048",
            "--length",
            "20",
        ])
        .output()
        .unwrap();
    assert!(scan.status.success(), "scan process failed: {scan:?}");
    let report: serde_json::Value = serde_json::from_slice(&scan.stdout).unwrap();
    assert_eq!(report["storage_profile"], "memory-card");
    assert_eq!(
        report["selected_range"],
        serde_json::json!({"offset": 2048, "length": 20})
    );
    assert_eq!(report["observed_bytes_scanned"], 20);
    assert_eq!(
        report["source_sha256"],
        format!("{:x}", Sha256::digest(&bytes))
    );
    assert_eq!(report["findings"], serde_json::json!([]));
    assert_eq!(
        report["rejected"][0]["reason"],
        "ends_at_selected_range_boundary"
    );
    assert_eq!(report["rejected"][0]["offset"], 2048);
    assert!(
        scan_with_options(
            &path,
            ScanOptions {
                offset: 2048,
                length: Some(u64::MAX),
                ..Default::default()
            }
        )
        .is_err()
    );
}

#[test]
fn does_not_expand_multiterabyte_sparse_gaps() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("holes.sparse");
    let mut bytes = sparse_header(4096, 1_000_000_000, 1, 0);
    sparse_chunk(&mut bytes, 0xcac3, 1_000_000_000, &[]);
    fs::write(&path, bytes).unwrap();
    let report = scan(&path).unwrap();
    assert_eq!(report.container_bytes, 40);
    assert_eq!(report.unspecified_bytes_skipped, 4_096_000_000_000);
    assert_eq!(report.observed_bytes_scanned, 0);
    assert!(report.findings.is_empty());
}

#[test]
fn rejects_malformed_sparse_input_and_does_not_fallback_to_raw() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("broken.img");
    let mut bytes = sparse_header(512, 1, 1, 0);
    sparse_chunk(&mut bytes, 0xcac1, 1, &[0; 24]);
    fs::write(&path, bytes).unwrap();
    assert!(scan(&path).is_err());
}

#[test]
fn rejects_damaged_png_crc_and_truncated_jpeg() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("damage.img");
    let mut png = photo(ImageFormat::Png, [3, 2, 1]);
    let last = png.len() - 1;
    png[last] ^= 1;
    let mut jpeg = photo(ImageFormat::Jpeg, [4, 5, 6]);
    jpeg.truncate(jpeg.len() - 2);
    png.extend(jpeg);
    fs::write(&path, png).unwrap();
    let report = scan(&path).unwrap();
    assert!(report.findings.is_empty());
    assert_eq!(report.rejected.len(), 2);
    assert_eq!(
        report.rejected[0].reason,
        "invalid_or_unsupported_structure_or_decode"
    );
    assert_eq!(
        report.rejected[1].reason,
        "ends_at_unobserved_range_or_container_boundary"
    );
}

#[test]
fn discloses_signature_budget_exhaustion() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("false-positives.img");
    fs::write(&path, [0xff, 0xd8, 0xff, 0xd9].repeat(4097)).unwrap();
    let report = scan(&path).unwrap();
    assert!(report.candidate_limit_reached);
    assert!(!report.observed_ranges_fully_scanned);
    assert_eq!(report.observed_bytes_scanned, 4096 * 4);
    assert_eq!(report.scan_stopped_at, Some(4096 * 4));
    assert_eq!(report.rejected.len(), 4096);
}

#[test]
fn rejects_recovery_when_source_or_finding_changes() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("capture.img");
    let mut png = photo(ImageFormat::Png, [7, 8, 9]);
    fs::write(&path, &png).unwrap();
    let report = scan(&path).unwrap();
    let mut forged = report.findings[0].clone();
    forged.sha256 = "0".repeat(64);
    assert!(recover_candidate(&path, &report.source_sha256, &forged).is_err());
    png.push(0);
    fs::write(&path, &png).unwrap();
    assert!(recover_candidate(&path, &report.source_sha256, &report.findings[0]).is_err());
}

#[test]
fn compares_pre_and_post_exact_media_and_keeps_duplicate_offsets() {
    let dir = tempdir().unwrap();
    let before = dir.path().join("before.img");
    let after = dir.path().join("after.img");
    let shared = photo(ImageFormat::Png, [1, 2, 3]);
    let old = photo(ImageFormat::Png, [10, 20, 30]);
    let new = photo(ImageFormat::Png, [100, 200, 30]);
    fs::write(&before, [&old[..], &shared, &shared].concat()).unwrap();
    fs::write(&after, [&shared[..], &new].concat()).unwrap();
    let report = compare(&before, &after).unwrap();
    assert_eq!(report.changes.len(), 3);
    let same = report
        .changes
        .iter()
        .find(|item| item.observation == "observed_in_both")
        .unwrap();
    assert_eq!(
        same.before_offsets,
        vec![old.len() as u64, (old.len() + shared.len()) as u64]
    );
    assert_eq!(same.after_offsets, vec![0]);
    assert!(
        report
            .changes
            .iter()
            .any(|item| item.observation == "observed_before_only")
    );
    assert!(
        report
            .changes
            .iter()
            .any(|item| item.observation == "observed_after_only")
    );
}

#[test]
fn hashes_and_compares_accessible_logical_export_files() {
    let dir = tempdir().unwrap();
    let before = dir.path().join("before");
    let after = dir.path().join("after");
    for root in [&before, &after] {
        fs::create_dir_all(root.join("DCIM")).unwrap();
        fs::write(root.join("DCIM/photo.jpg"), b"same bytes").unwrap();
    }
    fs::write(before.join("messages.db"), b"pre-event").unwrap();
    fs::write(after.join("messages.db"), b"post-event").unwrap();
    fs::write(before.join("old.mp4"), b"old").unwrap();
    fs::write(after.join("new.mp4"), b"new").unwrap();
    let snapshot = inventory(&before).unwrap();
    assert_eq!(
        snapshot.files["messages.db"].sha256,
        format!("{:x}", Sha256::digest(b"pre-event"))
    );
    let changes = compare_exports(&before, &after).unwrap();
    assert_eq!(
        changes.paths["DCIM/photo.jpg"].observation,
        "same_bytes_at_path"
    );
    assert_eq!(
        changes.paths["messages.db"].observation,
        "different_bytes_at_path"
    );
    assert_eq!(changes.paths["old.mp4"].observation, "observed_before_only");
    assert_eq!(changes.paths["new.mp4"].observation, "observed_after_only");
}

#[cfg(unix)]
#[test]
fn rejects_symlinked_image_and_skips_export_links() {
    use std::os::unix::fs::symlink;

    let dir = tempdir().unwrap();
    let before = dir.path().join("before");
    let after = dir.path().join("after");
    fs::create_dir_all(before.join("private")).unwrap();
    fs::create_dir(&after).unwrap();
    let original = before.join("private/photo.jpg");
    fs::write(&original, b"sample").unwrap();
    let link = dir.path().join("link.img");
    symlink(&original, &link).unwrap();
    assert!(scan(&link).is_err());
    symlink(before.join("private"), after.join("private")).unwrap();
    let report = compare_exports(&before, &after).unwrap();
    assert_eq!(report.after_skipped["private"], "link_or_reparse_point");
    assert_eq!(
        report.paths["private/photo.jpg"].observation,
        "after_path_not_inspected"
    );
}

#[test]
fn cli_emits_deterministic_json_and_returns_errors_before_output() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("sample.img");
    fs::write(&path, photo(ImageFormat::Png, [1, 0, 0])).unwrap();
    let command = env!("CARGO_BIN_EXE_omnirecovery");
    let first = Command::new(command)
        .arg("scan")
        .arg(&path)
        .output()
        .unwrap();
    let second = Command::new(command)
        .arg("scan")
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        first.status.success(),
        "first scan process failed: {first:?}"
    );
    assert!(
        second.status.success(),
        "second scan process failed: {second:?}"
    );
    assert_eq!(first.stdout, second.stdout);
    let report: serde_json::Value = serde_json::from_slice(&first.stdout).unwrap();
    assert_eq!(report["schema_version"], 1);
    assert_eq!(report["findings"][0]["media_type"], "png");
    let error = Command::new(command)
        .arg("scan")
        .arg(dir.path())
        .output()
        .unwrap();
    assert!(!error.status.success());
    assert!(error.stdout.is_empty());
}
