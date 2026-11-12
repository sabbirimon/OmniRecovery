use std::io::{self, Read};
use std::path::Path;

use flate2::{Decompress, FlushDecompress, Status};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::media::{MAX_CANDIDATE_BYTES, MediaType, Validated};
use crate::source::{hash_image, invalid, open_image, same_file_state};

mod office;

const IEND: &[u8] = b"\0\0\0\0IEND\xaeB\x60\x82";
const MAX_BIT_REPAIR_CHUNK: usize = 4 * 1024 * 1024;

pub struct RepairPreview {
    pub report: RepairReport,
    pub repaired_bytes: Option<Vec<u8>>,
}

#[derive(Debug, Serialize)]
pub struct RepairReport {
    pub schema_version: u32,
    pub engine_version: &'static str,
    pub source_sha256: String,
    pub source_bytes: u64,
    pub media_type: Option<MediaType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub document_type: Option<&'static str>,
    pub expected_sha256: Option<String>,
    pub status: &'static str,
    pub reason: &'static str,
    pub result: Option<RepairResult>,
}

#[derive(Debug, Serialize)]
pub struct RepairResult {
    pub sha256: String,
    pub length: u64,
    pub width: u32,
    pub height: u32,
    pub verification: &'static str,
    pub change: Option<RepairChange>,
}

#[derive(Debug, Serialize)]
pub struct RepairChange {
    pub offset: u64,
    pub replaced_bytes: u64,
    pub inserted_bytes: u64,
    pub algorithm: &'static str,
}

/// Preview a repair of a standalone file. Returns a separate derivative in memory.
pub fn preview_repair(path: &Path, expected_sha256: Option<&str>) -> io::Result<RepairPreview> {
    let expected = expected_sha256.map(str::to_ascii_lowercase);
    if expected
        .as_ref()
        .is_some_and(|hash| hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()))
    {
        return Err(invalid(
            "expected SHA-256 must be 64 hexadecimal characters",
        ));
    }
    let mut file = open_image(path)?;
    let state = file.metadata()?;
    if state.len() > MAX_CANDIDATE_BYTES as u64 {
        return Err(invalid("repair input exceeds the 16 MiB limit"));
    }
    let mut input = Vec::with_capacity(state.len() as usize);
    (&mut file).take(state.len() + 1).read_to_end(&mut input)?;
    let mut report = RepairReport {
        schema_version: 1,
        engine_version: env!("CARGO_PKG_VERSION"),
        source_sha256: digest(&input),
        source_bytes: state.len(),
        media_type: MediaType::detect(&input),
        document_type: None,
        expected_sha256: expected,
        status: "unresolved",
        reason: "unsupported_format",
        result: None,
    };
    let repaired_bytes = repair(&input, &mut report);
    if input.len() as u64 != state.len()
        || hash_image(&mut file, state.len())? != report.source_sha256
        || !same_file_state(&state, &file.metadata()?)
    {
        return Err(invalid("source changed during repair preview"));
    }
    Ok(RepairPreview {
        report,
        repaired_bytes,
    })
}

fn repair(input: &[u8], report: &mut RepairReport) -> Option<Vec<u8>> {
    let Some(media) = report.media_type else {
        return repair_office(input, report);
    };
    if let Some(validated) = validate_exact(media, input) {
        if report
            .expected_sha256
            .as_ref()
            .is_some_and(|hash| hash != &report.source_sha256)
        {
            report.reason = "input_does_not_match_reference_hash";
            return None;
        }
        report.status = "already_valid";
        report.reason = if media.is_photo() {
            "input_passed_structure_and_decode_checks"
        } else {
            "input_passed_container_structure_checks"
        };
        report.result = Some(result(
            &validated,
            report.source_sha256.clone(),
            report.expected_sha256.is_some(),
            None,
            media.is_photo(),
        ));
        return None;
    }
    report.reason = "damage_outside_supported_patterns_or_validation_failed";
    let trials = match media {
        MediaType::Png => png_trials(input, report.expected_sha256.is_some()),
        MediaType::Jpeg if report.expected_sha256.is_some() => jpeg_trials(input),
        MediaType::Jpeg => {
            report.reason = "trusted_sha256_required_for_jpeg_repair";
            return None;
        }
        MediaType::Wav if report.expected_sha256.is_some() => wav_trials(input),
        MediaType::Mp4 | MediaType::Mov if report.expected_sha256.is_some() => movie_trials(input),
        MediaType::Mp4 | MediaType::Mov | MediaType::Wav => {
            report.reason = "trusted_sha256_required_for_container_repair";
            return None;
        }
    };
    for trial in trials {
        let sha256 = trial.digest(input);
        if report
            .expected_sha256
            .as_ref()
            .is_some_and(|hash| hash != &sha256)
        {
            report.reason = "no_repair_matches_reference_hash";
            continue;
        }
        if let Some(validated) = validate_exact(media, &trial.pieces(input).concat()) {
            report.status = "repair_available";
            report.reason = "deterministic_repair_passed_validation";
            report.result = Some(result(
                &validated,
                sha256,
                report.expected_sha256.is_some(),
                Some(trial.change),
                media.is_photo(),
            ));
            return Some(validated.bytes);
        }
    }
    None
}

fn result(
    validated: &Validated,
    sha256: String,
    matched: bool,
    change: Option<RepairChange>,
    decoded: bool,
) -> RepairResult {
    RepairResult {
        sha256,
        length: validated.bytes.len() as u64,
        width: validated.width,
        height: validated.height,
        verification: if matched {
            "trusted_sha256_match"
        } else if decoded {
            "structure_and_decode"
        } else {
            "container_structure"
        },
        change,
    }
}

fn repair_office(input: &[u8], report: &mut RepairReport) -> Option<Vec<u8>> {
    let directory = office::inspect(input)?;
    report.document_type = Some(directory.extension);
    let (bytes, change, sha256) = if let Some(fields) = directory.replacement {
        let Some(expected) = &report.expected_sha256 else {
            report.reason = "trusted_sha256_required_for_container_repair";
            return None;
        };
        let trial = replace_field(
            directory.fields,
            &fields,
            "office_restore_zip_directory_bounds",
        );
        let sha256 = trial.digest(input);
        if sha256 != *expected {
            report.reason = "no_repair_matches_reference_hash";
            return None;
        }
        report.status = "repair_available";
        report.reason = "deterministic_repair_passed_validation";
        (
            Some(trial.pieces(input).concat()),
            Some(trial.change),
            sha256,
        )
    } else {
        if report
            .expected_sha256
            .as_ref()
            .is_some_and(|expected| expected != &report.source_sha256)
        {
            report.reason = "input_does_not_match_reference_hash";
            return None;
        }
        report.status = "already_valid";
        report.reason = "input_passed_zip_directory_checks";
        (None, None, report.source_sha256.clone())
    };
    report.result = Some(RepairResult {
        sha256,
        length: input.len() as u64,
        width: 0,
        height: 0,
        verification: if report.expected_sha256.is_some() {
            "trusted_sha256_match"
        } else {
            "zip_directory_structure"
        },
        change,
    });
    bytes
}

struct RepairTrial {
    change: RepairChange,
    replacement: Vec<u8>,
}

impl RepairTrial {
    fn new(offset: usize, replaced: usize, value: &[u8], algorithm: &'static str) -> Self {
        Self {
            change: RepairChange {
                offset: offset as u64,
                replaced_bytes: replaced as u64,
                inserted_bytes: value.len() as u64,
                algorithm,
            },
            replacement: value.to_vec(),
        }
    }

    fn pieces<'a>(&'a self, input: &'a [u8]) -> [&'a [u8]; 3] {
        let start = self.change.offset as usize;
        let end = start + self.change.replaced_bytes as usize;
        [&input[..start], &self.replacement, &input[end..]]
    }

    fn digest(&self, input: &[u8]) -> String {
        let mut digest = Sha256::new();
        for piece in self.pieces(input) {
            digest.update(piece);
        }
        format!("{:x}", digest.finalize())
    }
}

fn replace_field(offset: usize, value: &[u8], algorithm: &'static str) -> RepairTrial {
    RepairTrial::new(offset, value.len(), value, algorithm)
}

fn movie_trials(input: &[u8]) -> Vec<RepairTrial> {
    let mut trials = Vec::new();
    let algorithm = if input.get(8..12) == Some(b"qt  ") {
        "mov_restore_final_atom_size"
    } else {
        "mp4_restore_final_box_size"
    };
    let mut offset = 0;
    for _ in 0..256 {
        let Some(header) = input.get(offset..offset + 8) else {
            break;
        };
        let size = u32::from_be_bytes(header[..4].try_into().expect("box size")) as usize;
        let remaining = input.len() - offset;
        if matches!(&header[4..], b"moov" | b"mdat") && size != remaining && size != 1 {
            trials.push(replace_field(
                offset,
                &(remaining as u32).to_be_bytes(),
                algorithm,
            ));
            if trials.len() == 2 {
                break;
            }
        }
        if !(8..=remaining).contains(&size) {
            break;
        }
        offset += size;
    }
    trials
}

fn wav_trials(input: &[u8]) -> Vec<RepairTrial> {
    let mut trials = Vec::new();
    let riff_size = ((input.len() - 8) as u32).to_le_bytes();
    if input[4..8] != riff_size {
        trials.push(replace_field(4, &riff_size, "wav_restore_riff_size"));
    }
    let mut offset = 12;
    let mut has_format = false;
    for _ in 0..256 {
        let Some(header) = input.get(offset..offset + 8) else {
            break;
        };
        let size = u32::from_le_bytes(header[4..].try_into().expect("chunk size")) as usize;
        let payload = offset + 8;
        if &header[..4] == b"data" && has_format {
            let available = input.len() - payload;
            // The data chunk may end in a single zero byte of RIFF padding.
            for length in [
                Some(available),
                available.checked_sub(1).filter(|_| input.ends_with(&[0])),
            ]
            .into_iter()
            .flatten()
            {
                if length != size && length != 0 {
                    trials.push(replace_field(
                        offset + 4,
                        &(length as u32).to_le_bytes(),
                        "wav_restore_terminal_data_size",
                    ));
                }
            }
            break;
        }
        let Some(next) = payload
            .checked_add(size)
            .and_then(|end| end.checked_add(size % 2))
            .filter(|&end| end <= input.len())
        else {
            break;
        };
        if &header[..4] == b"fmt " && matches!(size, 16 | 18) && !has_format {
            has_format = true;
            let format = &input[payload..payload + size];
            let channels = u16::from_le_bytes([format[2], format[3]]);
            let bits = u16::from_le_bytes([format[14], format[15]]);
            let rate = u32::from_le_bytes(format[4..8].try_into().expect("sample rate"));
            if format[..2] == [1, 0]
                && (1..=32).contains(&channels)
                && matches!(bits, 8 | 16 | 24 | 32)
                && let Some(byte_rate) = rate.checked_mul(u32::from(channels * (bits / 8)))
            {
                let alignment = (channels * (bits / 8)).to_le_bytes();
                let byte_rate = byte_rate.to_le_bytes();
                let mut fields = [0; 6];
                fields[..4].copy_from_slice(&byte_rate);
                fields[4..].copy_from_slice(&alignment);
                if format[8..14] != fields {
                    trials.push(replace_field(
                        payload + 8,
                        &fields,
                        "wav_restore_pcm_rate_and_alignment",
                    ));
                }
            }
        }
        offset = next;
    }
    trials
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn validate_exact(media: MediaType, bytes: &[u8]) -> Option<Validated> {
    if media == MediaType::Png && !valid_png_data(bytes) {
        return None;
    }
    let validated = media.validate(bytes).ok()?;
    (validated.bytes.len() == bytes.len()).then_some(validated)
}

fn replace_suffix(
    input: &[u8],
    start: usize,
    suffix: &[u8],
    algorithm: &'static str,
) -> RepairTrial {
    RepairTrial::new(start, input.len() - start, suffix, algorithm)
}

fn jpeg_trials(input: &[u8]) -> Vec<RepairTrial> {
    let mut trials = Vec::new();
    if input.ends_with(&[0xff]) {
        trials.push(replace_suffix(
            input,
            input.len(),
            &[0xd9],
            "jpeg_complete_end_marker",
        ));
    }
    trials.push(replace_suffix(
        input,
        input.len(),
        &[0xff, 0xd9],
        "jpeg_append_end_marker",
    ));
    trials
}

fn png_trials(input: &[u8], has_reference: bool) -> Vec<RepairTrial> {
    let mut offset = 8;
    let mut bad_idat = None;
    loop {
        let tail = &input[offset..];
        if tail.len() < IEND.len() && IEND.starts_with(tail)
            || (8..=12).contains(&tail.len()) && tail.starts_with(&IEND[..8]) && tail != IEND
        {
            return if bad_idat.is_none() {
                vec![replace_suffix(input, offset, IEND, "png_rebuild_iend")]
            } else {
                Vec::new()
            };
        }
        let Some((kind, data, stored_crc, total)) = chunk(tail) else {
            return Vec::new();
        };
        let actual_crc = crc32fast::hash(&tail[4..8 + data.len()]);
        if actual_crc != stored_crc {
            if !has_reference
                || kind != b"IDAT"
                || data.len() > MAX_BIT_REPAIR_CHUNK
                || bad_idat.is_some()
            {
                return Vec::new();
            }
            bad_idat = Some((offset + 8, data.len(), actual_crc ^ stored_crc));
        }
        if kind == b"IEND" {
            if tail != IEND {
                return Vec::new();
            }
            return bad_idat.map_or_else(Vec::new, |(start, length, syndrome)| {
                bit_trials(input, start, length, syndrome)
            });
        }
        offset += total;
    }
}

fn bit_trials(input: &[u8], start: usize, length: usize, syndrome: u32) -> Vec<RepairTrial> {
    const POLYNOMIAL: u32 = 0xedb8_8320;
    let mut trials = Vec::new();
    let mut effect = POLYNOMIAL;
    for bit in (0..length * 8).rev() {
        if effect == syndrome {
            if !trials.is_empty() {
                return Vec::new();
            }
            trials.push(flip(
                input,
                start + bit / 8,
                1 << (bit % 8),
                "png_idat_crc32_syndrome",
            ));
        }
        effect = (effect >> 1) ^ (POLYNOMIAL & 0_u32.wrapping_sub(effect & 1));
    }
    if syndrome.count_ones() == 1 {
        let position = syndrome.trailing_zeros() as usize;
        trials.push(flip(
            input,
            start + length + 3 - position / 8,
            1 << (position % 8),
            "png_idat_crc_field",
        ));
    }
    trials
}

fn flip(input: &[u8], offset: usize, mask: u8, algorithm: &'static str) -> RepairTrial {
    replace_field(offset, &[input[offset] ^ mask], algorithm)
}

fn chunk(tail: &[u8]) -> Option<(&[u8], &[u8], u32, usize)> {
    let length = u32::from_be_bytes(tail.get(..4)?.try_into().ok()?) as usize;
    let end = 8_usize.checked_add(length)?;
    let total = end.checked_add(4)?;
    let crc = u32::from_be_bytes(tail.get(end..total)?.try_into().ok()?);
    Some((&tail[4..8], &tail[8..end], crc, total))
}

fn valid_png_data(bytes: &[u8]) -> bool {
    let Some(ihdr) = bytes.get(16..29) else {
        return false;
    };
    let width = u64::from(u32::from_be_bytes(
        ihdr[..4].try_into().expect("IHDR width"),
    ));
    let height = u64::from(u32::from_be_bytes(
        ihdr[4..8].try_into().expect("IHDR height"),
    ));
    let channels = match ihdr[9] {
        0 | 3 => 1,
        2 => 3,
        4 => 2,
        6 => 4,
        _ => return false,
    };
    if width > 12_000 || height > 12_000 || width * height > 24_000_000 || ihdr[8] > 16 {
        return false;
    }
    let bits = channels * u64::from(ihdr[8]);
    let filtered = |x: u64, y: u64| {
        if x == 0 || y == 0 {
            0
        } else {
            ((x * bits).div_ceil(8) + 1) * y
        }
    };
    let expected = match ihdr[12] {
        0 => filtered(width, height),
        1 => [
            (0, 0, 8, 8),
            (4, 0, 8, 8),
            (0, 4, 4, 8),
            (2, 0, 4, 4),
            (0, 2, 2, 4),
            (1, 0, 2, 2),
            (0, 1, 1, 2),
        ]
        .into_iter()
        .map(|(x, y, dx, dy)| {
            filtered(
                width.saturating_sub(x).div_ceil(dx),
                height.saturating_sub(y).div_ceil(dy),
            )
        })
        .sum(),
        _ => return false,
    };
    let mut compressed = Vec::new();
    let mut tail = &bytes[8..];
    while let Some((kind, data, _, total)) = chunk(tail) {
        if kind == b"IDAT" {
            compressed.extend_from_slice(data);
        }
        tail = &tail[total..];
    }
    let mut decoder = Decompress::new(true);
    let mut buffer = [0; 16 * 1024];
    loop {
        let before_in = decoder.total_in();
        let before_out = decoder.total_out();
        let Ok(status) = decoder.decompress(
            &compressed[before_in as usize..],
            &mut buffer,
            FlushDecompress::None,
        ) else {
            return false;
        };
        if status == Status::StreamEnd {
            return decoder.total_in() == compressed.len() as u64
                && decoder.total_out() == expected;
        }
        if decoder.total_out() > expected
            || (decoder.total_in(), decoder.total_out()) == (before_in, before_out)
        {
            return false;
        }
    }
}
