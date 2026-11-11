use std::fs::File;
use std::io;

use serde::Serialize;

use crate::source::Layout;

#[derive(Debug, Serialize)]
pub struct PartitionMap {
    pub scheme: &'static str,
    pub sector_bytes: u64,
    pub primary_status: &'static str,
    pub backup_status: &'static str,
    pub partitions: Vec<Partition>,
}

#[derive(Debug, Serialize)]
pub struct Partition {
    pub offset: u64,
    pub length: u64,
    pub name: String,
    pub provenance: &'static str,
}

pub(crate) fn inspect(
    layout: &Layout,
    file: &mut File,
    size: u64,
) -> io::Result<Option<PartitionMap>> {
    let observed = layout.observed_ranges();
    let read = |file: &mut File, offset: u64, length: usize| -> io::Result<Option<Vec<u8>>> {
        let Some(end) = offset.checked_add(length as u64) else {
            return Ok(None);
        };
        let index = observed.partition_point(|(_, stop)| *stop <= offset);
        if !observed
            .get(index)
            .is_some_and(|&(start, stop)| start <= offset && end <= stop)
        {
            return Ok(None);
        }
        let mut data = vec![0; length];
        layout.read_observed(file, offset, &mut data)?;
        Ok(Some(data))
    };
    for sector in [512, 4096] {
        if !size.is_multiple_of(sector) || size / sector < 6 {
            continue;
        }
        let last = size / sector - 1;
        let primary = gpt(file, &read, sector, 1, last)?;
        let backup = gpt(file, &read, sector, last, 1)?;
        if primary.is_none() && backup.is_none() {
            continue;
        }
        let primary_status = if primary.is_some() {
            "crc_verified"
        } else {
            "unavailable_or_invalid"
        };
        let backup_status = if backup.is_some() {
            "crc_verified"
        } else {
            "unavailable_or_invalid"
        };
        let mut partitions = primary.unwrap_or_default();
        if let Some(backup) = backup {
            let known: std::collections::HashSet<_> =
                partitions.iter().map(|p| (p.offset, p.length)).collect();
            partitions.extend(
                backup
                    .into_iter()
                    .filter(|p| !known.contains(&(p.offset, p.length)))
                    .map(|mut p| {
                        p.provenance = "backup_gpt_candidate";
                        p
                    }),
            );
        }
        return Ok(Some(PartitionMap {
            scheme: "gpt",
            sector_bytes: sector,
            primary_status,
            backup_status,
            partitions,
        }));
    }
    let Some(header) = read(file, 0, 512)? else {
        return Ok(None);
    };
    if header[510..512] != [0x55, 0xaa] {
        return Ok(None);
    }
    let mut partitions = Vec::new();
    let (records, _) = header[446..510].as_chunks::<16>();
    for (index, record) in records.iter().enumerate() {
        let kind = record[4];
        if !matches!(record[0], 0 | 0x80) {
            return Ok(None);
        }
        if matches!(kind, 0 | 0x05 | 0x0f | 0x85 | 0xee) {
            continue;
        }
        let offset = u64::from(le32(record, 8)) * 512;
        let length = u64::from(le32(record, 12)) * 512;
        if offset == 0 || length == 0 || offset + length > size {
            return Ok(None);
        }
        partitions.push(Partition {
            offset,
            length,
            name: format!("Partition {} (type {kind:02x})", index + 1),
            provenance: "listed_partition",
        });
    }
    partitions.sort_unstable_by_key(|p| p.offset);
    if partitions.is_empty()
        || partitions
            .windows(2)
            .any(|pair| pair[0].offset + pair[0].length > pair[1].offset)
    {
        return Ok(None);
    }
    Ok(Some(PartitionMap {
        scheme: "mbr",
        sector_bytes: 512,
        primary_status: "geometry_checked",
        backup_status: "not_applicable",
        partitions,
    }))
}

type ReadExtent<'a> = dyn Fn(&mut File, u64, usize) -> io::Result<Option<Vec<u8>>> + 'a;

fn gpt(
    file: &mut File,
    read: &ReadExtent<'_>,
    sector: u64,
    current: u64,
    alternate: u64,
) -> io::Result<Option<Vec<Partition>>> {
    let Some(mut header) = read(file, current * sector, sector as usize)? else {
        return Ok(None);
    };
    if &header[..8] != b"EFI PART" {
        return Ok(None);
    }
    let bytes = le32(&header, 12) as usize;
    if !(92..=header.len()).contains(&bytes)
        || le64(&header, 24) != current
        || le64(&header, 32) != alternate
    {
        return Ok(None);
    }
    let crc = le32(&header, 16);
    header[16..20].fill(0);
    if crc32fast::hash(&header[..bytes]) != crc {
        return Ok(None);
    }
    let first = le64(&header, 40);
    let last = le64(&header, 48);
    let table_lba = le64(&header, 72);
    let count = le32(&header, 80) as usize;
    let width = le32(&header, 84) as usize;
    if first < 2
        || first > last
        || last >= current.max(alternate)
        || !(1..=4096).contains(&count)
        || !(128..=512).contains(&width)
        || !width.is_multiple_of(8)
    {
        return Ok(None);
    }
    let table_sectors = (count * width).div_ceil(sector as usize) as u64;
    let Some(table_end) = table_lba.checked_add(table_sectors) else {
        return Ok(None);
    };
    if (current == 1 && (table_lba < 2 || table_end > first))
        || (current != 1 && (table_lba <= last || table_end > current))
    {
        return Ok(None);
    }
    let Some(table) = read(file, table_lba * sector, count * width)? else {
        return Ok(None);
    };
    if crc32fast::hash(&table) != le32(&header, 88) {
        return Ok(None);
    }
    let mut partitions = Vec::new();
    for (index, entry) in table.chunks_exact(width).enumerate() {
        if entry[..16].iter().all(|byte| *byte == 0) {
            continue;
        }
        let start = le64(entry, 32);
        let end = le64(entry, 40);
        if start < first || end < start || end > last {
            return Ok(None);
        }
        let (name_bytes, _) = entry[56..128].as_chunks::<2>();
        let name = String::from_utf16_lossy(
            &name_bytes
                .iter()
                .copied()
                .map(u16::from_le_bytes)
                .take_while(|word| *word != 0)
                .collect::<Vec<_>>(),
        );
        partitions.push(Partition {
            offset: start * sector,
            length: (end - start + 1) * sector,
            name: if name.is_empty() {
                format!("Partition {}", index + 1)
            } else {
                name
            },
            provenance: "listed_partition",
        });
    }
    partitions.sort_unstable_by_key(|p| p.offset);
    if partitions
        .windows(2)
        .any(|pair| pair[0].offset + pair[0].length > pair[1].offset)
    {
        return Ok(None);
    }
    Ok(Some(partitions))
}

fn le32(data: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(
        data[offset..offset + 4]
            .try_into()
            .expect("fixed partition field"),
    )
}
fn le64(data: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(
        data[offset..offset + 8]
            .try_into()
            .expect("fixed partition field"),
    )
}
