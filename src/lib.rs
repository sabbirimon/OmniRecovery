mod backup;
mod catalogue;
mod media;
mod partitions;
mod repair;
mod snapshot;
mod source;

use std::collections::BTreeMap;
use std::fs::{File, Metadata};
use std::io::{self, BufReader};
use std::path::Path;

use media::{MAX_CANDIDATE_BYTES, Rejection, Validated};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use source::{Layout, ObservedReader, hash_image, invalid, open_image, same_file_state};

pub use backup::{BackupReport, backup_files, backup_selected_files};
pub use catalogue::{
    Catalogue, CatalogueEntry, FileCategory, catalogue, catalogue_file, verify_file,
};
pub use media::MediaType;
pub use partitions::{Partition, PartitionMap};
pub use repair::{RepairChange, RepairPreview, RepairReport, RepairResult, preview_repair};
pub use snapshot::{
    ExportComparison, ExportFile, ExportInventory, PathObservation, compare_exports, inventory,
};
pub use source::{SkippedKind, SkippedRange};

const WINDOW_BYTES: usize = 1024 * 1024;
const SIGNATURE_LOOKAHEAD: usize = 7;
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
    pub findings: Vec<Finding>,
    pub rejected: Vec<RejectedCandidate>,
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
        if hash_image(&mut self.file, self.metadata.len())? != self.source_sha256
            || !same_file_state(&self.metadata, &self.file.metadata()?)
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
            confidence: 3,
            reason: "contiguous_observed_bytes_with_valid_structure_and_successful_decode",
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
        findings: Vec::new(),
        rejected: Vec::new(),
    };
    let mut buffer = vec![0; WINDOW_BYTES + SIGNATURE_LOOKAHEAD];
    let mut remaining_candidate_reads = CANDIDATE_READ_BUDGET;
    'ranges: for (start, end) in ranges {
        let mut offset = start;
        while offset < end {
            let read_bytes = (end - offset).min(buffer.len() as u64) as usize;
            let primary_bytes = read_bytes.min(WINDOW_BYTES);
            session
                .layout
                .read_observed(&mut session.file, offset, &mut buffer[..read_bytes])?;
            for index in 0..primary_bytes {
                let Some(media_type) = MediaType::detect(&buffer[index..read_bytes]) else {
                    continue;
                };
                if media_filter.is_some_and(|requested| requested != media_type) {
                    continue;
                }
                let candidate_offset = offset + index as u64;
                let read_bound = (end - candidate_offset)
                    .min((MAX_CANDIDATE_BYTES + CANDIDATE_BUFFER_BYTES) as u64);
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
                    Ok(candidate) => report.findings.push(Finding::from_validated(
                        media_type,
                        candidate_offset,
                        &candidate,
                    )),
                    Err(Rejection::Io(error)) => return Err(error),
                    Err(reason) => report.rejected.push(RejectedCandidate {
                        offset: candidate_offset,
                        media_type,
                        reason: match reason {
                            Rejection::Invalid => "invalid_or_unsupported_structure_or_decode",
                            Rejection::Truncated
                                if end == range_end && range_end < session.layout.logical_bytes =>
                            {
                                "ends_at_selected_range_boundary"
                            }
                            Rejection::Truncated => {
                                "ends_at_unobserved_range_or_container_boundary"
                            }
                            Rejection::Limit => "exceeds_candidate_limits",
                            Rejection::Io(_) => unreachable!("I/O failures propagate"),
                        },
                    }),
                }
            }
            offset += primary_bytes as u64;
            report.observed_bytes_scanned += primary_bytes as u64;
        }
    }
    session.verify()?;
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

/// Compare decoded media by exact content. The caller supplies the capture order.
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
    if finding.length == 0 || finding.length > MAX_CANDIDATE_BYTES as u64 {
        return Err(invalid("candidate size is outside recovery limits"));
    }
    let mut session = ImageSession::open(image)?;
    if session.source_sha256 != source_sha256 {
        return Err(invalid("source SHA-256 does not match the findings report"));
    }
    let end = finding
        .offset
        .checked_add(finding.length)
        .ok_or_else(|| invalid("candidate offset overflow"))?;
    if !session
        .layout
        .observed_ranges()
        .iter()
        .any(|&(start, range_end)| start <= finding.offset && end <= range_end)
    {
        return Err(invalid("candidate crosses an unobserved range"));
    }
    let candidate = session
        .candidate(finding.media_type, finding.offset, end)
        .0
        .map_err(|error| match error {
            Rejection::Io(error) => error,
            _ => invalid("candidate failed independent validation"),
        })?;
    if Finding::from_validated(finding.media_type, finding.offset, &candidate) != *finding {
        return Err(invalid("candidate does not match the findings report"));
    }
    session.verify()?;
    Ok(candidate.bytes)
}
