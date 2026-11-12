use omnirecovery::{
    MediaType, ScanOptions, preview_repair, recover_candidate, scan, scan_selected_media,
};
use sha2::{Digest, Sha256};

const MP4: &[u8] = include_bytes!("../desktop/assets/demo.mp4");
const WAV: &[u8] = include_bytes!("../desktop/assets/demo.wav");

#[test]
fn recovers_exact_video_and_audio_bytes_and_filters_scan() {
    let directory = tempfile::tempdir().unwrap();
    let image = directory.path().join("media.dd");
    let video_offset = 1024 * 1024 - 4;
    let audio_offset = 2 * 1024 * 1024 - 10;
    let mut bytes = vec![0; video_offset];
    bytes.extend_from_slice(MP4);
    bytes.resize(audio_offset, 0);
    bytes.extend_from_slice(WAV);
    bytes.extend_from_slice(&[0xff; 20]);
    std::fs::write(&image, &bytes).unwrap();

    let report = scan(&image).unwrap();
    assert_eq!(report.findings.len(), 2);
    assert_eq!(report.triage.container_verified, 2);
    for (finding, media, offset, original) in [
        (&report.findings[0], MediaType::Mp4, video_offset, MP4),
        (&report.findings[1], MediaType::Wav, audio_offset, WAV),
    ] {
        assert_eq!(finding.media_type, media);
        assert_eq!(finding.offset, offset as u64);
        assert_eq!(finding.length, original.len() as u64);
        assert_eq!(finding.confidence, 2);
        assert_eq!(
            recover_candidate(&image, &report.source_sha256, finding).unwrap(),
            original
        );
        let filtered = scan_selected_media(&image, ScanOptions::default(), Some(media)).unwrap();
        assert_eq!(filtered.findings.as_slice(), std::slice::from_ref(finding));
    }
    assert_eq!(std::fs::read(image).unwrap(), bytes);
}

#[test]
fn rejects_unobserved_audio_and_out_of_bounds_video_samples() {
    let directory = tempfile::tempdir().unwrap();
    let image = directory.path().join("damaged.dd");
    let mut bytes = MP4.to_vec();
    let offset = bytes.windows(4).position(|part| part == b"stco").unwrap() + 12;
    bytes[offset..offset + 4].copy_from_slice(&u32::MAX.to_be_bytes());
    bytes.extend_from_slice(&WAV[..WAV.len() - 2]);
    std::fs::write(&image, bytes).unwrap();

    let report = scan(&image).unwrap();
    assert!(report.findings.is_empty());
    assert_eq!(report.triage.structure_rejected, 1);
    assert_eq!(report.triage.needs_more_source, 1);
    assert!(
        report
            .rejected
            .iter()
            .any(|item| item.media_type == MediaType::Mp4
                && item.reason == "invalid_or_unsupported_structure_or_decode")
    );
    assert!(
        report
            .rejected
            .iter()
            .any(|item| item.media_type == MediaType::Wav
                && item.reason == "ends_at_unobserved_range_or_container_boundary")
    );
}

#[test]
fn repairs_video_and_audio_size_fields_only_with_a_matching_hash() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("damaged");
    let video_size_field = MP4.windows(4).position(|part| part == b"mdat").unwrap() - 4;
    let mut mov = MP4.to_vec();
    mov[8..12].copy_from_slice(b"qt  ");
    let wav_data_size = WAV.windows(4).position(|part| part == b"data").unwrap() + 4;
    let wav_byte_rate = WAV.windows(4).position(|part| part == b"fmt ").unwrap() + 16;
    for (original, offset, media) in [
        (MP4, video_size_field, MediaType::Mp4),
        (mov.as_slice(), video_size_field, MediaType::Mov),
        (WAV, 4, MediaType::Wav),
        (WAV, wav_data_size, MediaType::Wav),
        (WAV, wav_byte_rate, MediaType::Wav),
    ] {
        let expected = format!("{:x}", Sha256::digest(original));
        let mut damaged = original.to_vec();
        damaged[offset..offset + 4].fill(0xff);
        std::fs::write(&path, &damaged).unwrap();
        assert!(
            preview_repair(&path, None)
                .unwrap()
                .repaired_bytes
                .is_none()
        );
        assert!(
            preview_repair(&path, Some(&"0".repeat(64)))
                .unwrap()
                .repaired_bytes
                .is_none()
        );
        let repaired = preview_repair(&path, Some(&expected)).unwrap();
        assert_eq!(repaired.report.media_type, Some(media));
        assert_eq!(repaired.repaired_bytes.as_deref(), Some(original));
        assert_eq!(std::fs::read(&path).unwrap(), damaged);
    }
}

#[test]
fn identifies_quicktime_across_a_scan_window_without_mislabelling_it_as_mp4() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("camera-card.dd");
    let mut movie = MP4.to_vec();
    movie[8..12].copy_from_slice(b"qt  ");
    let offset = 1024 * 1024 - 1;
    let mut bytes = vec![0; offset];
    bytes.extend_from_slice(&movie);
    std::fs::write(&path, bytes).unwrap();

    let report = scan_selected_media(&path, ScanOptions::default(), Some(MediaType::Mov)).unwrap();
    assert_eq!(report.findings.len(), 1);
    assert_eq!(report.findings[0].offset, offset as u64);
    assert_eq!(
        recover_candidate(&path, &report.source_sha256, &report.findings[0]).unwrap(),
        movie
    );
    let mp4 = scan_selected_media(&path, ScanOptions::default(), Some(MediaType::Mp4)).unwrap();
    assert!(mp4.findings.is_empty() && mp4.rejected.is_empty());
}

#[test]
fn overlapping_markers_do_not_duplicate_rejected_candidates() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("overlapping-markers.dd");
    std::fs::write(&path, b"\xff\xd8\xff\xe1ftyp").unwrap();
    let report = scan(&path).unwrap();
    assert_eq!(report.rejected.len(), 1);
    assert_eq!(report.rejected[0].media_type, MediaType::Jpeg);
    assert_eq!(report.rejected[0].offset, 0);
}

#[test]
fn repairs_office_zip_footer_with_original_hash() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("damaged");
    for (entry, extension) in [
        ("word/document.xml", "docx"),
        ("xl/workbook.xml", "xlsx"),
        ("ppt/presentation.xml", "pptx"),
    ] {
        let original = office_package(entry);
        let expected = format!("{:x}", Sha256::digest(&original));
        let mut damaged = original.clone();
        let footer = damaged.len() - 22;
        damaged[footer + 12..footer + 16].fill(0xff);
        std::fs::write(&path, &damaged).unwrap();
        assert!(
            preview_repair(&path, None)
                .unwrap()
                .repaired_bytes
                .is_none()
        );
        assert!(
            preview_repair(&path, Some(&"0".repeat(64)))
                .unwrap()
                .repaired_bytes
                .is_none()
        );
        let repaired = preview_repair(&path, Some(&expected)).unwrap();
        assert_eq!(repaired.report.document_type, Some(extension));
        assert_eq!(repaired.repaired_bytes, Some(original));
        assert_eq!(std::fs::read(&path).unwrap(), damaged);
    }
}

fn office_package(main_part: &str) -> Vec<u8> {
    let mut archive = Vec::new();
    let mut directory = Vec::new();
    for name in ["[Content_Types].xml", "_rels/.rels", main_part] {
        let payload = b"<fixture/>";
        let crc = crc32fast::hash(payload).to_le_bytes();
        let length = (payload.len() as u32).to_le_bytes();
        let name_length = (name.len() as u16).to_le_bytes();
        let offset = (archive.len() as u32).to_le_bytes();
        let mut local = [0; 30];
        local[..4].copy_from_slice(b"PK\x03\x04");
        local[4] = 20;
        local[14..18].copy_from_slice(&crc);
        local[18..22].copy_from_slice(&length);
        local[22..26].copy_from_slice(&length);
        local[26..28].copy_from_slice(&name_length);
        archive.extend_from_slice(&local);
        archive.extend_from_slice(name.as_bytes());
        archive.extend_from_slice(payload);
        let mut central = [0; 46];
        central[..4].copy_from_slice(b"PK\x01\x02");
        central[4] = 20;
        central[6] = 20;
        central[16..28].copy_from_slice(&local[14..26]);
        central[28..30].copy_from_slice(&name_length);
        central[42..46].copy_from_slice(&offset);
        directory.extend_from_slice(&central);
        directory.extend_from_slice(name.as_bytes());
    }
    let mut footer = [0; 22];
    footer[..4].copy_from_slice(b"PK\x05\x06");
    footer[8] = 3;
    footer[10] = 3;
    footer[12..16].copy_from_slice(&(directory.len() as u32).to_le_bytes());
    footer[16..20].copy_from_slice(&(archive.len() as u32).to_le_bytes());
    archive.extend_from_slice(&directory);
    archive.extend_from_slice(&footer);
    archive
}
