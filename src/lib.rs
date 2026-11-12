mod acquisition;
mod ai;
mod android_wifi;
mod artifacts;
mod backup;
mod catalogue;
mod cloud;
mod custody;
mod device_service;
mod external_analysis;
mod fat32;
mod http;
mod media;
mod partitions;
mod phone_browse;
mod phone_transfer;
mod reconstruct;
mod repair;
mod snapshot;
mod source;

use std::collections::BTreeMap;
use std::fs::{File, Metadata};
use std::io::{self, BufReader};
use std::path::Path;

use media::{Rejection, Validated};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use source::{
    Layout, ObservedReader, hash_image, hash_image_with, invalid, open_image, same_file_state,
};

#[cfg(target_os = "linux")]
pub use acquisition::acquire_disk;
pub use acquisition::{
    AcquisitionReport, AndroidScope, DetectedDevice, Transport, acquire_device, acquire_image,
    detect_devices, verify_acquisition,
};
pub use acquisition::{
    CaptureHistory, CaptureHistoryEntry, capture_history, verify_capture_in_history,
};
pub use ai::{AiTriage, ai_triage};
pub use android_wifi::{connect_android_wifi, pair_android_wifi};
pub use artifacts::{ArtifactKind, ArtifactReport, inspect_artifact};
pub use backup::{
    BackupReport, TransferProtection, backup_files, backup_individual_files, backup_selected_files,
    package_selected_computer_files, package_transfer, restore_files,
};
pub use catalogue::{
    Catalogue, CatalogueEntry, FileCategory, catalogue, catalogue_file, verify_file,
};
pub use cloud::{
    CloudDownload, google_drive_download, google_drive_trash, google_photos_download,
    google_photos_list, google_photos_session,
};
pub use custody::{
    ActionPlan, ApprovedAction, SensitiveAction, make_action_plan, verify_action_approval,
};
pub use custody::{
    CustodyAction, CustodyHead, append_custody, generate_custody_key, verify_custody,
};
pub use device_service::{
    AndroidDetails, AndroidStorage, IosRecoveryDevice, MirrorAudio, MirrorOptions, MirrorProfile,
    MirrorVideo, android_fastboot_devices, inspect_android_details, ios_enter_recovery,
    ios_exit_recovery, ios_recovery_status, start_android_mirror, start_android_preview,
};
pub use external_analysis::{LeappEngine, LeappInput, analyze_leapp, tsk_deleted};
pub use fat32::{
    DeletedFatFile, FatExportReport, FatUndeleteReport, recover_fat32_deleted, scan_fat32_deleted,
};
pub use media::MediaType;
pub use partitions::{Partition, PartitionMap};
pub use phone_browse::{
    LivePhoneFile, LivePhoneFiles, PhoneCategory, browse_android_files, export_android_file,
};
pub use phone_transfer::{
    PhoneBackupReport, PhoneDelivery, PhoneFolder, backup_android_folders, deliver_phone_archive,
    package_android_folders,
};
pub use reconstruct::{ReconstructionReport, reconstruct_file};
pub use repair::{
    RepairChange, RepairPreview, RepairReport, RepairResult, export_repair, preview_repair,
};
pub use snapshot::{
    ExportComparison, ExportFile, ExportInventory, PathObservation, compare_exports, inventory,
};
pub use source::{SkippedKind, SkippedRange};

const WINDOW_BYTES: usize = 1024 * 1024;
const SIGNATURE_LOOKAHEAD: usize = 11;
const MAX_CANDIDATES: usize = 4096;
const CANDIDATE_READ_BUDGET: u64 = 2 * 1024 * 1024 * 1024;
const CANDIDATE_BUFFER_BYTES: usize = 8 * 1024;

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize, clap::ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum StorageProfile {
    #[default]
    Auto,
    MemoryCard,
    Hdd,
    Ssd,
    Ufs,
    Emmc,
    UsbFlash,
    Optical,
    Camera,
    Iot,
    VirtualDisk,
    Other,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ScanOptions {
    pub storage: StorageProfile,
    pub offset: u64,
    pub length: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct SelectedRange {
    pub offset: u64,
    pub length: u64,
}

#[derive(Debug, Serialize)]
pub struct ScanReport {
    pub schema_version: u32,
    pub engine_version: &'static str,
    pub source_sha256: String,
    pub container: &'static str,
    pub container_bytes: u64,
    pub logical_bytes: u64,
    pub storage_profile: StorageProfile,
    pub media_filter: Option<MediaType>,
    pub selected_range: SelectedRange,
    pub fill_bytes_skipped: u64,
    pub unspecified_bytes_skipped: u64,
    pub observed_bytes_scanned: u64,
    pub observed_ranges_fully_scanned: bool,
    pub candidate_limit_reached: bool,
    pub validation_budget_reached: bool,
    pub scan_stopped_at: Option<u64>,
    pub skipped_ranges: Vec<SkippedRange>,
    pub sparse_crc: &'static str,
    pub filesystem_signature: Option<&'static str>,
    pub partition_map: Option<PartitionMap>,
    pub offset_basis: &'static str,
    pub triage: ScanTriage,
    pub findings: Vec<Finding>,
    pub rejected: Vec<RejectedCandidate>,
}

/// Deterministic, local classification of scan results for recovery review.
#[derive(Debug, Default, Serialize)]
pub struct ScanTriage {
    pub decode_verified: usize,
    pub container_verified: usize,
    pub needs_more_source: usize,
    pub structure_rejected: usize,
    pub validation_limited: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Finding {
    pub media_type: MediaType,
    pub offset: u64,
    pub length: u64,
    pub sha256: String,
    pub width: u32,
    pub height: u32,
    pub confidence: u8,
    pub reason: &'static str,
}

#[derive(Debug, Serialize)]
pub struct RejectedCandidate {
    pub offset: u64,
    pub media_type: MediaType,
    pub reason: &'static str,
}

#[derive(Debug, Serialize)]
pub struct ComparisonReport {
    pub schema_version: u32,
    pub before: ScanReport,
    pub after: ScanReport,
    pub changes: Vec<ContentObservation>,
    pub interpretation: &'static str,
}

#[derive(Debug, Serialize)]
pub struct ContentObservation {
    pub sha256: String,
    pub media_type: MediaType,
    pub before_offsets: Vec<u64>,
    pub after_offsets: Vec<u64>,
    pub observation: &'static str,
}

struct ImageSession {
    file: File,
    metadata: Metadata,
    source_sha256: String,
    layout: Layout,
}

impl ImageSession {
    fn open(path: &Path) -> io::Result<Self> {
        let mut file = open_image(path)?;
        let metadata = file.metadata()?;
        let source_sha256 = hash_image(&mut file, metadata.len())?;
        let layout = Layout::read(&mut file, metadata.len())?;
        if !same_file_state(&metadata, &file.metadata()?) {
            return Err(invalid("source image changed during initial verification"));
        }
        Ok(Self {
            file,
            metadata,
            source_sha256,
            layout,
        })
    }

    fn verify(&mut self) -> io::Result<()> {
        let digest = hash_image(&mut self.file, self.metadata.len())?;
        self.verify_digest(&digest)
    }

    fn verify_digest(&self, digest: &str) -> io::Result<()> {
        if digest != self.source_sha256 || !same_file_state(&self.metadata, &self.file.metadata()?)
        {
            return Err(invalid("source image changed during analysis"));
        }
        Ok(())
    }

    fn candidate(
        &mut self,
        media_type: MediaType,
        offset: u64,
        end: u64,
    ) -> (Result<Validated, Rejection>, u64) {
        let mut reader = BufReader::with_capacity(
            CANDIDATE_BUFFER_BYTES,
            ObservedReader {
                layout: &self.layout,
                file: &mut self.file,
                offset,
                end,
            },
        );
        let result = media_type.validate(&mut reader);
        (result, reader.get_ref().offset - offset)
    }

    fn filesystem_signature(
        &mut self,
        base: u64,
        ranges: &[(u64, u64)],
    ) -> io::Result<Option<&'static str>> {
        const SUPERBLOCK_START: u64 = 1024;
        let mut bytes = [0; 58];
        let Some(superblock) = base.checked_add(SUPERBLOCK_START) else {
            return Ok(None);
        };
        if !ranges.iter().any(|&(start, end)| {
            start <= superblock && end.saturating_sub(superblock) >= bytes.len() as u64
        }) {
            return Ok(None);
        }
        self.layout
            .read_observed(&mut self.file, superblock, &mut bytes)?;
        if bytes[..4] == [0x10, 0x20, 0xf5, 0xf2] {
            return Ok(Some("f2fs_magic_at_1024"));
        }
        if bytes[56..58] == [0x53, 0xef] {
            return Ok(Some("ext_family_magic_at_1080"));
        }
        Ok(None)
    }
}

impl Finding {
    fn from_validated(media_type: MediaType, offset: u64, candidate: &Validated) -> Self {
        Self {
            media_type,
            offset,
            length: candidate.bytes.len() as u64,
            sha256: format!("{:x}", Sha256::digest(&candidate.bytes)),
            width: candidate.width,
            height: candidate.height,
            confidence: if media_type.is_photo() { 3 } else { 2 },
            reason: match media_type {
                MediaType::Jpeg | MediaType::Png => {
                    "contiguous_observed_bytes_with_valid_structure_and_successful_decode"
                }
                MediaType::Mp4 => "contiguous_mp4_core_with_sample_extents_inside_media_payload",
                MediaType::Mov => "contiguous_mov_core_with_sample_extents_inside_media_payload",
                MediaType::Wav => {
                    "contiguous_riff_wave_with_consistent_pcm_format_and_aligned_samples"
                }
            },
        }
    }
}

/// Examine a previously acquired immutable regular file without writing evidence or outputs.
pub fn scan(image: &Path) -> io::Result<ScanReport> {
    scan_with_options(image, ScanOptions::default())
}

/// Scan a logical byte range in a previously acquired image with a caller-supplied storage label.
pub fn scan_with_options(image: &Path, options: ScanOptions) -> io::Result<ScanReport> {
    scan_selected_media(image, options, None)
}

/// Apply a single media-type filter during signature detection.
pub fn scan_selected_media(
    image: &Path,
    options: ScanOptions,
    media_filter: Option<MediaType>,
) -> io::Result<ScanReport> {
    let mut session = ImageSession::open(image)?;
    let start = options.offset;
    let available = session
        .layout
        .logical_bytes
        .checked_sub(start)
        .ok_or_else(|| invalid("scan offset lies outside the logical image"))?;
    let length = options.length.unwrap_or(available);
    if length > available || (length == 0 && (start != 0 || options.length.is_some())) {
        return Err(invalid(
            "scan range must contain bytes inside the logical image",
        ));
    }
    let range_end = start + length;
    let ranges: Vec<_> = session
        .layout
        .observed_ranges()
        .into_iter()
        .filter_map(|(first, last)| {
            let low = first.max(start);
            let high = last.min(range_end);
            (low < high).then_some((low, high))
        })
        .collect();
    let filesystem_signature = session.filesystem_signature(start, &ranges)?;
    let partition_map = partitions::inspect(
        &session.layout,
        &mut session.file,
        session.layout.logical_bytes,
    )?;
    let mut report = ScanReport {
        schema_version: 1,
        engine_version: env!("CARGO_PKG_VERSION"),
        source_sha256: session.source_sha256.clone(),
        container: session.layout.container,
        container_bytes: session.metadata.len(),
        logical_bytes: session.layout.logical_bytes,
        storage_profile: options.storage,
        media_filter,
        selected_range: SelectedRange {
            offset: start,
            length,
        },
        fill_bytes_skipped: 0,
        unspecified_bytes_skipped: 0,
        observed_bytes_scanned: 0,
        observed_ranges_fully_scanned: true,
        candidate_limit_reached: false,
        validation_budget_reached: false,
        scan_stopped_at: None,
        skipped_ranges: Vec::new(),
        sparse_crc: if session.layout.declared_crc {
            "verified"
        } else {
            "absent"
        },
        filesystem_signature,
        partition_map,
        offset_basis: "zero_based_logical_image_bytes",
        triage: ScanTriage::default(),
        findings: Vec::new(),
        rejected: Vec::new(),
    };
    let mut buffer = vec![0; WINDOW_BYTES + SIGNATURE_LOOKAHEAD];
    let mut remaining_candidate_reads = CANDIDATE_READ_BUDGET;
    // A complete raw scan already reads the full source. Hash only each primary
    // window, leaving lookahead bytes for the next window's digest update.
    let mut scan_digest =
        (session.layout.container == "raw" && start == 0 && length == available).then(Sha256::new);
    'ranges: for (start, end) in ranges {
        let mut offset = start;
        while offset < end {
            let read_bytes = (end - offset).min(buffer.len() as u64) as usize;
            let primary_bytes = read_bytes.min(WINDOW_BYTES);
            session
                .layout
                .read_observed(&mut session.file, offset, &mut buffer[..read_bytes])?;
            if let Some(digest) = &mut scan_digest {
                digest.update(&buffer[..primary_bytes]);
            }
            for (index, media_type) in
                media::signatures(&buffer[..read_bytes], primary_bytes, media_filter)
            {
                let candidate_offset = offset + index as u64;
                let read_bound = (end - candidate_offset)
                    .min((media_type.max_bytes() + CANDIDATE_BUFFER_BYTES) as u64);
                report.candidate_limit_reached =
                    report.findings.len() + report.rejected.len() == MAX_CANDIDATES;
                report.validation_budget_reached = read_bound > remaining_candidate_reads;
                if report.candidate_limit_reached || report.validation_budget_reached {
                    report.observed_bytes_scanned += index as u64;
                    report.observed_ranges_fully_scanned = false;
                    report.scan_stopped_at = Some(candidate_offset);
                    break 'ranges;
                }
                let (result, read_bytes) = session.candidate(media_type, candidate_offset, end);
                remaining_candidate_reads -= read_bytes;
                match result {
                    Ok(candidate) => {
                        if media_type.is_photo() {
                            report.triage.decode_verified += 1;
                        } else {
                            report.triage.container_verified += 1;
                        }
                        report.findings.push(Finding::from_validated(
                            media_type,
                            candidate_offset,
                            &candidate,
                        ));
                    }
                    Err(Rejection::Io(error)) => return Err(error),
                    Err(rejection) => {
                        let reason = match rejection {
                            Rejection::Invalid => {
                                report.triage.structure_rejected += 1;
                                "invalid_or_unsupported_structure_or_decode"
                            }
                            Rejection::Truncated => {
                                report.triage.needs_more_source += 1;
                                if end == range_end && range_end < session.layout.logical_bytes {
                                    "ends_at_selected_range_boundary"
                                } else {
                                    "ends_at_unobserved_range_or_container_boundary"
                                }
                            }
                            Rejection::Limit => {
                                report.triage.validation_limited += 1;
                                "exceeds_candidate_limits"
                            }
                            Rejection::Io(_) => unreachable!("I/O failures propagate"),
                        };
                        report.rejected.push(RejectedCandidate {
                            offset: candidate_offset,
                            media_type,
                            reason,
                        });
                    }
                }
            }
            offset += primary_bytes as u64;
            report.observed_bytes_scanned += primary_bytes as u64;
        }
    }
    if let Some(digest) = scan_digest.filter(|_| report.observed_ranges_fully_scanned) {
        session.verify_digest(&format!("{:x}", digest.finalize()))?;
    } else {
        session.verify()?;
    }
    for mut skipped in session.layout.skipped_ranges {
        let low = skipped.offset.max(start);
        let high = (skipped.offset + skipped.length).min(range_end);
        if low < high {
            skipped.offset = low;
            skipped.length = high - low;
            match skipped.kind {
                SkippedKind::SparseFill => report.fill_bytes_skipped += skipped.length,
                SkippedKind::SparseDontCare => report.unspecified_bytes_skipped += skipped.length,
            }
            report.skipped_ranges.push(skipped);
        }
    }
    Ok(report)
}

/// Read a short verified logical range for the desktop hex inspector.
pub fn inspect_image_bytes(
    image: &Path,
    source_sha256: &str,
    offset: u64,
    length: usize,
) -> io::Result<Vec<u8>> {
    if !(1..=256).contains(&length) {
        return Err(invalid("hex inspection requires 1–256 bytes"));
    }
    let mut session = ImageSession::open(image)?;
    if session.source_sha256 != source_sha256 {
        return Err(invalid("source SHA-256 does not match the scan report"));
    }
    let mut buffer = vec![0; length];
    session
        .layout
        .read_observed(&mut session.file, offset, &mut buffer)?;
    session.verify()?;
    Ok(buffer)
}

/// Compare validated media by exact content. The caller supplies the capture order.
pub fn compare(before: &Path, after: &Path) -> io::Result<ComparisonReport> {
    let before = scan(before)?;
    let after = scan(after)?;
    let mut hashes = BTreeMap::new();
    for (index, report) in [&before, &after].into_iter().enumerate() {
        for finding in &report.findings {
            let offsets = hashes
                .entry((&finding.sha256, finding.media_type))
                .or_insert_with(|| [Vec::new(), Vec::new()]);
            offsets[index].push(finding.offset);
        }
    }
    let changes = hashes
        .into_iter()
        .map(
            |((sha256, media_type), [before_offsets, after_offsets])| ContentObservation {
                sha256: sha256.clone(),
                media_type,
                observation: match (before_offsets.is_empty(), after_offsets.is_empty()) {
                    (false, false) => "observed_in_both",
                    (false, true) => "observed_before_only",
                    (true, false) => "observed_after_only",
                    (true, true) => unreachable!("each comparison entry has an observation"),
                },
                before_offsets,
                after_offsets,
            },
        )
        .collect();
    Ok(ComparisonReport {
        schema_version: 1,
        before,
        after,
        changes,
        interpretation: "Capture order is supplied by the operator. Absence means the scanner did not observe a matching validated byte sequence in that input; it does not prove deletion or erasure.",
    })
}

/// Recover exact validated bytes into memory for a trusted local Rust caller.
/// Case admission, authorization and persistence belong to the future case service.
pub fn recover_candidate(
    image: &Path,
    source_sha256: &str,
    finding: &Finding,
) -> io::Result<Vec<u8>> {
    let (verified, bytes) = validate_recovery(
        image,
        source_sha256,
        finding.media_type,
        finding.offset,
        finding.length,
    )?;
    if verified != *finding {
        return Err(invalid("candidate does not match the findings report"));
    }
    Ok(bytes)
}

fn validate_recovery(
    image: &Path,
    source_sha256: &str,
    media_type: MediaType,
    offset: u64,
    length: u64,
) -> io::Result<(Finding, Vec<u8>)> {
    if length == 0 || length > media_type.max_bytes() as u64 {
        return Err(invalid("candidate size is outside recovery limits"));
    }
    let end = offset
        .checked_add(length)
        .ok_or_else(|| invalid("candidate offset overflow"))?;
    let mut file = open_image(image)?;
    let state = file.metadata()?;
    let layout = Layout::read(&mut file, state.len())?;
    let ranges = layout.physical_ranges(offset, end)?;
    let mut ranges = ranges.into_iter().peekable();
    let mut bytes = Vec::with_capacity(length as usize);
    // The candidate and the digest come from the same full-source read.
    let digest = hash_image_with(&mut file, state.len(), |offset, observed| {
        let end = offset + observed.len() as u64;
        while let Some(range) = ranges.peek().filter(|range| range.start < end) {
            let low = (range.start.max(offset) - offset) as usize;
            let high = (range.end.min(end) - offset) as usize;
            bytes.extend_from_slice(&observed[low..high]);
            if range.end > end {
                break;
            }
            ranges.next();
        }
    })?;
    if digest != source_sha256 {
        return Err(invalid("source SHA-256 does not match the findings report"));
    }
    let candidate = media_type
        .validate(bytes.as_slice())
        .map_err(|error| match error {
            Rejection::Io(error) => error,
            _ => invalid("candidate failed independent validation"),
        })?;
    if bytes.len() as u64 != length || candidate.bytes.len() as u64 != length {
        return Err(invalid(
            "candidate does not match the findings report length",
        ));
    }
    if !same_file_state(&state, &file.metadata()?) {
        return Err(invalid("source image changed during recovery"));
    }
    Ok((
        Finding::from_validated(media_type, offset, &candidate),
        candidate.bytes,
    ))
}

/// Recover one scan finding into a new file and verify it against both recorded digests.
pub fn export_recovery(
    image: &Path,
    source_sha256: &str,
    media_type: MediaType,
    offset: u64,
    length: u64,
    expected_sha256: &str,
    destination: &Path,
) -> io::Result<Finding> {
    if [source_sha256, expected_sha256]
        .into_iter()
        .any(|digest| digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()))
    {
        return Err(invalid("use SHA-256 digests from the scan report"));
    }
    let (finding, bytes) = validate_recovery(
        image,
        &source_sha256.to_ascii_lowercase(),
        media_type,
        offset,
        length,
    )?;
    if !finding.sha256.eq_ignore_ascii_case(expected_sha256) {
        return Err(invalid(
            "recovered content does not match the findings report digest",
        ));
    }
    write_verified_new(destination, &bytes, &finding.sha256)?;
    Ok(finding)
}

pub(crate) fn write_verified_new(destination: &Path, bytes: &[u8], sha256: &str) -> io::Result<()> {
    use std::io::Write;

    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    temp.write_all(bytes)?;
    temp.as_file().sync_all()?;
    if source::hash_image(temp.as_file_mut(), bytes.len() as u64)? != sha256 {
        return Err(invalid(
            "saved file did not match the verified content digest",
        ));
    }
    temp.persist_noclobber(destination)
        .map_err(|error| error.error)?;
    Ok(())
}
