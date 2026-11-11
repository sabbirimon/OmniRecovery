use std::fs;
use std::io::Cursor;

use aes_gcm::{
    Aes256Gcm, KeyInit, Nonce,
    aead::{Aead, Payload},
};
use image::{DynamicImage, ImageFormat, Rgb, RgbImage};
use omnirecovery::{backup_files, preview_repair};
use pbkdf2::pbkdf2_hmac;
use sha2::{Digest, Sha256};
use tempfile::tempdir;

fn fixture(format: ImageFormat) -> Vec<u8> {
    let mut output = Cursor::new(Vec::new());
    DynamicImage::ImageRgb8(RgbImage::from_pixel(7, 4, Rgb([29, 107, 209])))
        .write_to(&mut output, format)
        .unwrap();
    output.into_inner()
}

#[test]
fn repairs_only_png_ending_when_payload_is_complete() {
    let folder = tempdir().unwrap();
    let path = folder.path().join("damaged.png");
    let png = fixture(ImageFormat::Png);
    for missing in [1, 9, 12] {
        let damaged = &png[..png.len() - missing];
        fs::write(&path, damaged).unwrap();
        let preview = preview_repair(&path, None).unwrap();
        assert_eq!(preview.report.status, "repair_available");
        let result = preview.report.result.unwrap();
        assert_eq!(result.verification, "structure_and_decode");
        assert_eq!(result.change.unwrap().algorithm, "png_rebuild_iend");
        assert_eq!(preview.repaired_bytes.unwrap(), png);
        assert_eq!(fs::read(&path).unwrap(), damaged);
    }
    fs::write(&path, &png[..png.len() - 16]).unwrap();
    assert_eq!(
        preview_repair(&path, None).unwrap().report.status,
        "unresolved"
    );
}

#[test]
fn uses_crc_syndrome_to_restore_one_png_bit_with_reference_hash() {
    let folder = tempdir().unwrap();
    let path = folder.path().join("damaged.png");
    let png = fixture(ImageFormat::Png);
    let expected = format!("{:x}", Sha256::digest(&png));
    let idat = png.windows(4).position(|chunk| chunk == b"IDAT").unwrap();
    let offset = idat + 7;
    let mut corrupted = png.clone();
    corrupted[offset] ^= 0x20;
    fs::write(&path, &corrupted).unwrap();
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
    let verified = preview_repair(&path, Some(&expected)).unwrap();
    let result = verified.report.result.unwrap();
    assert_eq!(result.verification, "trusted_sha256_match");
    let patch = result.change.unwrap();
    assert_eq!(patch.offset, offset as u64);
    assert_eq!(patch.algorithm, "png_idat_crc32_syndrome");
    assert_eq!(verified.repaired_bytes.unwrap(), png);
}

#[test]
fn jpeg_end_marker_requires_matching_reference() {
    let folder = tempdir().unwrap();
    let path = folder.path().join("damaged.jpg");
    let jpeg = fixture(ImageFormat::Jpeg);
    let expected = format!("{:x}", Sha256::digest(&jpeg));
    fs::write(&path, &jpeg[..jpeg.len() - 2]).unwrap();
    let preview = preview_repair(&path, None).unwrap();
    assert_eq!(
        preview.report.reason,
        "trusted_sha256_required_for_jpeg_repair"
    );
    let preview = preview_repair(&path, Some(&expected)).unwrap();
    assert_eq!(preview.report.status, "repair_available");
    assert_eq!(preview.repaired_bytes.unwrap(), jpeg);
}

#[test]
fn portable_archive_is_encrypted_verified_and_no_clobber() {
    let folder = tempdir().unwrap();
    let input = folder.path().join("phone-export");
    fs::create_dir_all(input.join("DCIM")).unwrap();
    let data = b"synthetic-content-for-the-transfer-test";
    fs::write(input.join("DCIM/photo.txt"), data).unwrap();
    let archive = folder.path().join("transfer.omnibackup");
    let password = "synthetic passphrase only";
    let report = backup_files(&input, &archive, password).unwrap();
    assert_eq!(report.files, 1);
    assert_eq!(report.source_bytes, data.len() as u64);
    let bytes = fs::read(&archive).unwrap();
    assert!(!bytes.windows(data.len()).any(|window| window == data));
    assert!(!bytes.windows(14).any(|window| window == b"DCIM/photo.txt"));
    let header = &bytes[..24];
    assert_eq!(&header[..8], b"OMNIBK01");
    let mut key = [0; 32];
    pbkdf2_hmac::<Sha256>(password.as_bytes(), &header[8..], 600_000, &mut key);
    let cipher = Aes256Gcm::new_from_slice(&key).unwrap();
    let mut position = 24;
    let mut records = Vec::new();
    for counter in 0..4_u64 {
        let size = &bytes[position..position + 4];
        let length = u32::from_be_bytes(size.try_into().unwrap()) as usize;
        let end = position + 4 + length + 16;
        let nonce = [[0; 4].as_slice(), &counter.to_be_bytes()].concat();
        let aad = [header, &counter.to_be_bytes(), size].concat();
        records.push(
            cipher
                .decrypt(
                    Nonce::from_slice(&nonce),
                    Payload {
                        msg: &bytes[position + 4..end],
                        aad: &aad,
                    },
                )
                .unwrap(),
        );
        position = end;
    }
    assert_eq!(position, bytes.len());
    assert_eq!(records[0], b"\x01DCIM/photo.txt");
    assert_eq!(&records[1][1..], data);
    assert_eq!(&records[2][9..], Sha256::digest(data).as_slice());
    assert_eq!(records[3][0], 4);
    assert!(backup_files(&input, &archive, password).is_err());
    assert!(backup_files(&input, &input.join("bad.omnibackup"), password).is_err());
    assert_eq!(fs::read(archive).unwrap(), bytes);
    assert_eq!(fs::read(input.join("DCIM/photo.txt")).unwrap(), data);
}
