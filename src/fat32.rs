use std::collections::BTreeSet;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::source::{hash_image, invalid, open_image, same_file_state};

const MASK: u32 = 0x0fff_ffff;
const END_CHAIN: u32 = 0x0fff_fff8;
const MAX_DIRECTORY_CLUSTERS: usize = 65_536;
const MAX_CANDIDATES: usize = 20_000;

#[derive(Debug, Serialize)]
pub struct DeletedFatFile {
    pub directory_entry_offset: u64,
    pub observed_short_name: String,
    pub first_cluster: u32,
    pub assumed_data_offset: u64,
    pub length: u64,
}

#[derive(Debug, Serialize)]
pub struct FatUndeleteReport {
    pub volume_offset: u64,
    pub source_sha256: String,
    pub interpretation: &'static str,
    pub candidates: Vec<DeletedFatFile>,
}

#[derive(Debug, Serialize)]
pub struct FatExportReport {
    pub length: u64,
    pub sha256: String,
    pub matches_expected: bool,
}

struct FatVolume {
    offset: u64,
    cluster_bytes: u64,
    data_start: u64,
    clusters: u32,
    root: u32,
    fat: Vec<u8>,
}

fn u16_at(bytes: &[u8], index: usize) -> u16 {
    u16::from_le_bytes(bytes[index..index + 2].try_into().expect("fixed field"))
}

fn u32_at(bytes: &[u8], index: usize) -> u32 {
    u32::from_le_bytes(bytes[index..index + 4].try_into().expect("fixed field"))
}

fn read_at(file: &mut File, offset: u64, buffer: &mut [u8]) -> io::Result<()> {
    file.seek(SeekFrom::Start(offset))?;
    file.read_exact(buffer)
}

impl FatVolume {
    fn read(file: &mut File, size: u64, offset: u64) -> io::Result<Self> {
        let mut boot = [0_u8; 512];
        read_at(file, offset, &mut boot)?;
        let sector = u64::from(u16_at(&boot, 11));
        let per_cluster = u64::from(boot[13]);
        let reserved = u64::from(u16_at(&boot, 14));
        let copies = u64::from(boot[16]);
        let total = u64::from(u32_at(&boot, 32));
        let fat_sectors = u64::from(u32_at(&boot, 36));
        let root = u32_at(&boot, 44);
        if boot[510..] != [0x55, 0xaa]
            || !matches!(boot[0], 0xe9 | 0xeb)
            || !matches!(sector, 512 | 1024 | 2048 | 4096)
            || !per_cluster.is_power_of_two()
            || sector * per_cluster > 65_536
            || reserved == 0
            || !matches!(copies, 1 | 2)
            || u16_at(&boot, 17) != 0
            || u16_at(&boot, 19) != 0
            || u16_at(&boot, 22) != 0
            || u16_at(&boot, 40) & 0x80 != 0
            || u16_at(&boot, 42) != 0
            || fat_sectors == 0
            || offset
                .checked_add(total * sector)
                .is_none_or(|end| end > size)
        {
            return Err(invalid("invalid or unsupported FAT32 volume geometry"));
        }
        let data_sector = reserved + copies * fat_sectors;
        let clusters = total
            .checked_sub(data_sector)
            .ok_or_else(|| invalid("invalid FAT32 data area"))?
            / per_cluster;
        let table_bytes = fat_sectors * sector;
        if !(65_525..u64::from(END_CHAIN - 3)).contains(&clusters)
            || table_bytes < (clusters + 2) * 4
            || table_bytes > 256 * 1024 * 1024
            || !(2..=clusters as u32 + 1).contains(&root)
        {
            return Err(invalid(
                "invalid or unsupported FAT32 allocation table size",
            ));
        }
        let fat_offset = offset + reserved * sector;
        let mut fat = vec![0; table_bytes as usize];
        read_at(file, fat_offset, &mut fat)?;
        if copies == 2 {
            let mut mirror = vec![0; 1024 * 1024];
            file.seek(SeekFrom::Start(fat_offset + table_bytes))?;
            for original in fat.chunks(mirror.len()) {
                file.read_exact(&mut mirror[..original.len()])?;
                if mirror[..original.len()] != *original {
                    return Err(invalid("FAT32 allocation tables disagree"));
                }
            }
        }
        Ok(Self {
            offset,
            cluster_bytes: sector * per_cluster,
            data_start: offset + data_sector * sector,
            clusters: clusters as u32,
            root,
            fat,
        })
    }

    fn valid_cluster(&self, cluster: u32) -> bool {
        (2..=self.clusters + 1).contains(&cluster)
    }

    fn data_offset(&self, cluster: u32) -> u64 {
        self.data_start + u64::from(cluster - 2) * self.cluster_bytes
    }

    fn next(&self, cluster: u32) -> u32 {
        u32_at(&self.fat, cluster as usize * 4) & MASK
    }

    fn deleted_files(&self, file: &mut File) -> io::Result<Vec<DeletedFatFile>> {
        let mut queue = vec![self.root];
        let mut visited = BTreeSet::new();
        let mut live_starts = BTreeSet::new();
        let mut candidates = Vec::new();
        let mut data = vec![0; self.cluster_bytes as usize];
        while let Some(mut cluster) = queue.pop() {
            let mut end_entries = false;
            loop {
                if !self.valid_cluster(cluster)
                    || !visited.insert(cluster)
                    || visited.len() > MAX_DIRECTORY_CLUSTERS
                {
                    return Err(invalid(
                        "invalid, looping or excessive FAT32 directory chain",
                    ));
                }
                let start = self.data_offset(cluster);
                if !end_entries {
                    read_at(file, start, &mut data)?;
                    for (index, entry) in data.as_chunks::<32>().0.iter().enumerate() {
                        if entry[0] == 0 {
                            end_entries = true;
                            break;
                        }
                        let attr = entry[11];
                        if attr & 0x08 != 0 || entry[0] == b'.' {
                            continue;
                        }
                        let first =
                            (u32::from(u16_at(entry, 20)) << 16) | u32::from(u16_at(entry, 26));
                        let length = u64::from(u32_at(entry, 28));
                        if entry[0] == 0xe5 {
                            let count = length.div_ceil(self.cluster_bytes) as u32;
                            if attr & 0x10 == 0
                                && count != 0
                                && self.valid_cluster(first)
                                && first
                                    .checked_add(count - 1)
                                    .is_some_and(|last| self.valid_cluster(last))
                            {
                                if candidates.len() >= MAX_CANDIDATES {
                                    return Err(invalid("FAT32 deleted entry limit exceeded"));
                                }
                                let mut name = entry[..11].to_vec();
                                name[0] = b'_';
                                name.iter_mut().for_each(|b| {
                                    if !b.is_ascii_graphic() && *b != b' ' {
                                        *b = b'_';
                                    }
                                });
                                let base = std::str::from_utf8(&name[..8])
                                    .expect("ASCII short name")
                                    .trim_end();
                                let ext = std::str::from_utf8(&name[8..])
                                    .expect("ASCII short extension")
                                    .trim_end();
                                let observed_short_name = if ext.is_empty() {
                                    base.to_owned()
                                } else {
                                    format!("{base}.{ext}")
                                };
                                candidates.push(DeletedFatFile {
                                    directory_entry_offset: start + index as u64 * 32,
                                    observed_short_name,
                                    first_cluster: first,
                                    assumed_data_offset: self.data_offset(first),
                                    length,
                                });
                            }
                        } else if self.valid_cluster(first) {
                            live_starts.insert(first);
                            if attr & 0x10 != 0 {
                                if queue.len() >= MAX_DIRECTORY_CLUSTERS {
                                    return Err(invalid(
                                        "FAT32 directory traversal limit exceeded",
                                    ));
                                }
                                queue.push(first);
                            }
                        }
                    }
                }
                let next = self.next(cluster);
                if next >= END_CHAIN {
                    break;
                }
                cluster = next;
            }
        }
        // Build the sorted exclusions once. This bounds the work even when many
        // deleted entries refer to the same large range.
        let mut occupied: Vec<u32> = (2..=self.clusters + 1)
            .filter(|cluster| self.next(*cluster) != 0)
            .collect();
        occupied.extend(live_starts);
        occupied.sort_unstable();
        candidates.retain(|file| {
            let end = file.first_cluster + file.length.div_ceil(self.cluster_bytes) as u32;
            let first = occupied.partition_point(|cluster| *cluster < file.first_cluster);
            occupied.get(first).is_none_or(|cluster| *cluster >= end)
        });
        candidates.sort_unstable_by_key(|candidate| candidate.directory_entry_offset);
        Ok(candidates)
    }
}

/// List contiguous hypotheses from deleted FAT32 short entries in reachable directories.
pub fn scan_fat32_deleted(image: &Path, offset: u64) -> io::Result<FatUndeleteReport> {
    let mut file = open_image(image)?;
    let before = file.metadata()?;
    let volume = FatVolume::read(&mut file, before.len(), offset)?;
    let candidates = volume.deleted_files(&mut file)?;
    let source_sha256 = hash_image(&mut file, before.len())?;
    if !same_file_state(&before, &file.metadata()?) {
        return Err(invalid("FAT32 source changed during examination"));
    }
    Ok(FatUndeleteReport {
        volume_offset: volume.offset,
        source_sha256,
        interpretation: "Deleted short entries with unallocated contiguous spans. A span can contain reused data or be wrong for a fragmented file. The first name character was lost.",
        candidates,
    })
}

/// Export one hypothesis only from the recorded source, optionally matching an original digest.
pub fn recover_fat32_deleted(
    image: &Path,
    offset: u64,
    entry_offset: u64,
    source_sha256: &str,
    expected_sha256: Option<&str>,
    destination: &Path,
) -> io::Result<FatExportReport> {
    let mut file = open_image(image)?;
    let before = file.metadata()?;
    let volume = FatVolume::read(&mut file, before.len(), offset)?;
    let candidate = volume
        .deleted_files(&mut file)?
        .into_iter()
        .find(|candidate| candidate.directory_entry_offset == entry_offset)
        .ok_or_else(|| invalid("deleted FAT32 entry is no longer an eligible candidate"))?;
    let parent = destination
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    let start = candidate.assumed_data_offset;
    let end = start + candidate.length;
    let mut digest = Sha256::new();
    let mut write_error = None;
    let actual = crate::source::hash_image_with(&mut file, before.len(), |chunk_start, bytes| {
        let lower = chunk_start.max(start);
        let upper = (chunk_start + bytes.len() as u64).min(end);
        if upper > lower && write_error.is_none() {
            let selected = &bytes[(lower - chunk_start) as usize..(upper - chunk_start) as usize];
            digest.update(selected);
            if let Err(error) = temporary.write_all(selected) {
                write_error = Some(error);
            }
        }
    })?;
    if let Some(error) = write_error {
        return Err(error);
    }
    if !actual.eq_ignore_ascii_case(source_sha256) || !same_file_state(&before, &file.metadata()?) {
        return Err(invalid("source did not match the recorded FAT32 image"));
    }
    let sha256 = format!("{:x}", digest.finalize());
    if expected_sha256.is_some_and(|expected| !sha256.eq_ignore_ascii_case(expected)) {
        return Err(invalid(
            "FAT32 candidate did not match the original file digest",
        ));
    }
    temporary.as_file().sync_all()?;
    temporary
        .persist_noclobber(destination)
        .map_err(|error| error.error)?;
    Ok(FatExportReport {
        length: candidate.length,
        sha256,
        matches_expected: expected_sha256.is_some(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn fat32_recovers_only_free_contiguous_candidates() -> io::Result<()> {
        let dir = tempfile::tempdir()?;
        let image = dir.path().join("fat32.dd");
        let mut output = File::create_new(&image)?;
        let fat_sectors: u32 = 512;
        let data_start = u64::from(fat_sectors + 1) * 512;
        let sectors = fat_sectors + 1 + 65_525;
        output.set_len(u64::from(sectors) * 512)?;
        let mut boot = [0_u8; 512];
        boot[0] = 0xeb;
        boot[11..13].copy_from_slice(&512_u16.to_le_bytes());
        boot[13] = 1;
        boot[14..16].copy_from_slice(&1_u16.to_le_bytes());
        boot[16] = 1;
        boot[32..36].copy_from_slice(&sectors.to_le_bytes());
        boot[36..40].copy_from_slice(&fat_sectors.to_le_bytes());
        boot[44..48].copy_from_slice(&2_u32.to_le_bytes());
        boot[510..].copy_from_slice(&[0x55, 0xaa]);
        output.write_all(&boot)?;
        output.seek(SeekFrom::Start(512 + 2 * 4))?;
        output.write_all(&MASK.to_le_bytes())?;
        output.seek(SeekFrom::Start(512 + 4 * 4))?;
        output.write_all(&MASK.to_le_bytes())?;
        output.seek(SeekFrom::Start(data_start))?;
        let mut entry = [0_u8; 32];
        entry[..11].copy_from_slice(b"TEST    TXT");
        entry[0] = 0xe5;
        entry[26..28].copy_from_slice(&3_u16.to_le_bytes());
        entry[28..].copy_from_slice(&5_u32.to_le_bytes());
        output.write_all(&entry)?;
        entry[26..28].copy_from_slice(&4_u16.to_le_bytes());
        output.write_all(&entry)?;
        output.seek(SeekFrom::Start(data_start + 512))?;
        output.write_all(b"hello")?;
        drop(output);
        let report = scan_fat32_deleted(&image, 0)?;
        assert_eq!(report.candidates.len(), 1);
        assert_eq!(report.candidates[0].observed_short_name, "_EST.TXT");
        let exported = dir.path().join("export.txt");
        let expected = format!("{:x}", Sha256::digest(b"hello"));
        assert!(
            recover_fat32_deleted(
                &image,
                0,
                data_start,
                &report.source_sha256,
                Some("incorrect"),
                &exported
            )
            .is_err()
        );
        assert!(!exported.exists());
        let saved = recover_fat32_deleted(
            &image,
            0,
            data_start,
            &report.source_sha256,
            Some(&expected),
            &exported,
        )?;
        assert!(saved.matches_expected);
        assert_eq!(fs::read(exported)?, b"hello");
        Ok(())
    }
}
