use std::fs::{self, File, Metadata, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom};
use std::ops::Range;
use std::path::Path;

use serde::Serialize;
use sha2::{Digest, Sha256};

const SPARSE_MAGIC: [u8; 4] = [0x3a, 0xff, 0x26, 0xed];
pub(crate) fn is_sparse_image(file: &mut File) -> io::Result<bool> {
    file.rewind()?;
    let mut magic = Vec::new();
    file.take(4).read_to_end(&mut magic)?;
    Ok(magic == SPARSE_MAGIC)
}

const MAX_SPARSE_CHUNKS: u32 = 1_000_000;

pub(crate) fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

pub(crate) fn open_image(path: &Path) -> io::Result<File> {
    let before = fs::symlink_metadata(path)?;
    if !before.is_file() || is_reparse_point(&before) {
        return Err(invalid("image must be a regular file"));
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_SHARE_READ: u32 = 0x1;
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        options
            .share_mode(FILE_SHARE_READ)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let file = options.open(path)?;
    let opened = file.metadata()?;
    if !opened.is_file() || is_reparse_point(&opened) || !same_file_state(&before, &opened) {
        return Err(invalid("image changed while it was opened"));
    }
    Ok(file)
}

#[cfg(windows)]
pub(crate) fn is_reparse_point(metadata: &Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
pub(crate) fn is_reparse_point(_: &Metadata) -> bool {
    false
}

pub(crate) fn same_file_state(before: &Metadata, after: &Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if (
            before.dev(),
            before.ino(),
            before.ctime(),
            before.ctime_nsec(),
        ) != (after.dev(), after.ino(), after.ctime(), after.ctime_nsec())
        {
            return false;
        }
    }
    before.len() == after.len() && before.modified().ok() == after.modified().ok()
}

pub(crate) fn hash_image(file: &mut File, size: u64) -> io::Result<String> {
    hash_image_with(file, size, |_, _| {})
}

pub(crate) fn hash_image_with(
    file: &mut File,
    size: u64,
    mut observed: impl FnMut(u64, &[u8]),
) -> io::Result<String> {
    file.seek(SeekFrom::Start(0))?;
    let mut reader = file.take(size.saturating_add(1));
    let mut digest = Sha256::new();
    let mut buffer = vec![0; size.clamp(1, 1024 * 1024) as usize];
    let mut read_bytes = 0;
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        let bytes = &buffer[..count];
        digest.update(bytes);
        observed(read_bytes, bytes);
        read_bytes += count as u64;
    }
    if read_bytes != size {
        return Err(invalid("image length changed while reading"));
    }
    Ok(format!("{:x}", digest.finalize()))
}

#[derive(Clone, Copy)]
struct RawExtent {
    start: u64,
    end: u64,
    file_start: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SkippedKind {
    SparseFill,
    SparseDontCare,
}

#[derive(Debug, Eq, PartialEq, Serialize)]
pub struct SkippedRange {
    pub offset: u64,
    pub length: u64,
    pub kind: SkippedKind,
}

pub(crate) struct Layout {
    raw: Vec<RawExtent>,
    pub(crate) container: &'static str,
    pub(crate) logical_bytes: u64,
    pub(crate) declared_crc: bool,
    pub(crate) skipped_ranges: Vec<SkippedRange>,
}

impl Layout {
    pub(crate) fn read(file: &mut File, size: u64) -> io::Result<Self> {
        if size >= 4 && read_at::<4>(file, 0)? == SPARSE_MAGIC {
            return Self::sparse(file, size);
        }
        Ok(Self {
            raw: if size == 0 {
                Vec::new()
            } else {
                vec![RawExtent {
                    start: 0,
                    end: size,
                    file_start: 0,
                }]
            },
            container: "raw",
            logical_bytes: size,
            declared_crc: false,
            skipped_ranges: Vec::new(),
        })
    }

    fn sparse(file: &mut File, size: u64) -> io::Result<Self> {
        let header = read_at::<28>(file, 0)?;
        let file_header = u64::from(le16(&header, 8));
        let chunk_header = u64::from(le16(&header, 10));
        let block_size = u64::from(le32(&header, 12));
        let blocks = u64::from(le32(&header, 16));
        let chunks = le32(&header, 20);
        if le16(&header, 4) != 1 || le16(&header, 6) != 0 {
            return Err(invalid("unsupported Android sparse image version"));
        }
        if !(28..=4096).contains(&file_header)
            || !(12..=4096).contains(&chunk_header)
            || block_size == 0
            || block_size % 4 != 0
            || chunks > MAX_SPARSE_CHUNKS
            || file_header > size
        {
            return Err(invalid(
                "invalid or unsupported Android sparse image geometry",
            ));
        }
        let header_crc = le32(&header, 24);
        let mut layout = Self {
            raw: Vec::new(),
            container: "android_sparse_v1",
            logical_bytes: blocks * block_size,
            declared_crc: header_crc != 0,
            skipped_ranges: Vec::new(),
        };
        let mut crc = crc32fast::Hasher::new();
        let mut crc_buffer = vec![0; 64 * 1024];
        let mut file_offset = file_header;
        let mut logical_offset = 0_u64;
        for _ in 0..chunks {
            let chunk = read_at::<12>(file, file_offset)?;
            let logical_size = u64::from(le32(&chunk, 4)) * block_size;
            let physical_size = u64::from(le32(&chunk, 8));
            let Some(next_file) = file_offset.checked_add(physical_size) else {
                return Err(invalid("sparse chunk offset overflow"));
            };
            let Some(next_logical) = logical_offset.checked_add(logical_size) else {
                return Err(invalid("sparse logical offset overflow"));
            };
            if physical_size < chunk_header
                || next_file > size
                || next_logical > layout.logical_bytes
            {
                return Err(invalid("sparse chunk lies outside the image"));
            }
            let payload = physical_size - chunk_header;
            let payload_offset = file_offset + chunk_header;
            match le16(&chunk, 0) {
                0xcac1 if payload == logical_size && logical_size != 0 => {
                    file.seek(SeekFrom::Start(payload_offset))?;
                    let mut remaining = payload;
                    while remaining != 0 {
                        let count = remaining.min(crc_buffer.len() as u64) as usize;
                        file.read_exact(&mut crc_buffer[..count])?;
                        crc.update(&crc_buffer[..count]);
                        remaining -= count as u64;
                    }
                    layout.raw.push(RawExtent {
                        start: logical_offset,
                        end: next_logical,
                        file_start: payload_offset,
                    });
                }
                0xcac2 if payload == 4 && logical_size != 0 => {
                    layout.record_skipped(logical_offset, logical_size, SkippedKind::SparseFill);
                    append_pattern_crc(
                        &mut crc,
                        read_at::<4>(file, payload_offset)?,
                        logical_size / 4,
                    );
                }
                0xcac3 if payload == 0 && logical_size != 0 => {
                    layout.record_skipped(
                        logical_offset,
                        logical_size,
                        SkippedKind::SparseDontCare,
                    );
                    append_pattern_crc(&mut crc, [0; 4], logical_size / 4);
                }
                0xcac4 if payload == 4 && logical_size == 0 => {
                    layout.declared_crc = true;
                    let expected = u32::from_le_bytes(read_at::<4>(file, payload_offset)?);
                    if crc.clone().finalize() != expected {
                        return Err(invalid("Android sparse CRC chunk does not match"));
                    }
                }
                _ => return Err(invalid("unsupported or inconsistent sparse chunk")),
            }
            logical_offset = next_logical;
            file_offset = next_file;
        }
        if logical_offset != layout.logical_bytes || file_offset != size {
            return Err(invalid("sparse image totals do not match the chunk table"));
        }
        if header_crc != 0 && header_crc != crc.finalize() {
            return Err(invalid("Android sparse header checksum does not match"));
        }
        Ok(layout)
    }

    fn record_skipped(&mut self, offset: u64, length: u64, kind: SkippedKind) {
        match self.skipped_ranges.last_mut() {
            Some(range) if range.kind == kind && range.offset + range.length == offset => {
                range.length += length;
            }
            _ => self.skipped_ranges.push(SkippedRange {
                offset,
                length,
                kind,
            }),
        }
    }

    pub(crate) fn observed_ranges(&self) -> Vec<(u64, u64)> {
        let mut ranges: Vec<(u64, u64)> = Vec::new();
        for extent in &self.raw {
            match ranges.last_mut() {
                Some((_, end)) if *end == extent.start => *end = extent.end,
                _ => ranges.push((extent.start, extent.end)),
            }
        }
        ranges
    }

    pub(crate) fn read_observed(
        &self,
        file: &mut File,
        mut offset: u64,
        mut buffer: &mut [u8],
    ) -> io::Result<()> {
        let first = self.raw.partition_point(|extent| extent.end <= offset);
        for extent in &self.raw[first..] {
            if buffer.is_empty() || offset < extent.start {
                break;
            }
            let count = buffer
                .len()
                .min((extent.end - offset).try_into().unwrap_or(usize::MAX));
            file.seek(SeekFrom::Start(extent.file_start + (offset - extent.start)))?;
            file.read_exact(&mut buffer[..count])?;
            offset += count as u64;
            buffer = &mut buffer[count..];
        }
        if buffer.is_empty() {
            Ok(())
        } else {
            Err(invalid(
                "requested bytes are outside observed image extents",
            ))
        }
    }

    pub(crate) fn physical_ranges(&self, mut offset: u64, end: u64) -> io::Result<Vec<Range<u64>>> {
        let mut ranges = Vec::new();
        let first = self.raw.partition_point(|extent| extent.end <= offset);
        for extent in &self.raw[first..] {
            if offset < extent.start || offset == end {
                break;
            }
            let next = end.min(extent.end);
            let start = extent.file_start + (offset - extent.start);
            ranges.push(start..start + (next - offset));
            offset = next;
        }
        if offset != end {
            return Err(invalid("candidate crosses an unobserved range"));
        }
        Ok(ranges)
    }
}

// Combine CRCs for powers of two to handle large FILL and DONT_CARE regions in bounded space.
fn append_pattern_crc(crc: &mut crc32fast::Hasher, pattern: [u8; 4], mut repeats: u64) {
    let mut block = crc32fast::Hasher::new();
    block.update(&pattern);
    while repeats != 0 {
        if repeats & 1 == 1 {
            crc.combine(&block);
        }
        repeats >>= 1;
        if repeats != 0 {
            block.combine(&block.clone());
        }
    }
}

fn read_at<const N: usize>(file: &mut File, offset: u64) -> io::Result<[u8; N]> {
    let mut bytes = [0; N];
    file.seek(SeekFrom::Start(offset))?;
    file.read_exact(&mut bytes)?;
    Ok(bytes)
}

fn le16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(
        bytes[offset..offset + 2]
            .try_into()
            .expect("fixed header field"),
    )
}

fn le32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("fixed header field"),
    )
}

pub(crate) struct ObservedReader<'a> {
    pub(crate) layout: &'a Layout,
    pub(crate) file: &'a mut File,
    pub(crate) offset: u64,
    pub(crate) end: u64,
}

impl Read for ObservedReader<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let count = buffer
            .len()
            .min((self.end - self.offset).try_into().unwrap_or(usize::MAX));
        self.layout
            .read_observed(self.file, self.offset, &mut buffer[..count])?;
        self.offset += count as u64;
        Ok(count)
    }
}
