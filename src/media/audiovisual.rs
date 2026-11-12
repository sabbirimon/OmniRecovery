use std::io::Read;
use std::ops::Range;

use super::{Candidate, Rejection};

const MAX_BOXES: usize = 256;
const MAX_SAMPLES: usize = 1_000_000;

struct BoxContents<'a> {
    kind: [u8; 4],
    payload: &'a [u8],
}

pub(super) fn wav(input: &mut Candidate<impl Read>) -> Result<(), Rejection> {
    let head = input.take(12)?;
    if &head[..4] != b"RIFF" || &head[8..] != b"WAVE" {
        return Err(Rejection::Invalid);
    }
    let size = le32(&head[4..8]) as usize;
    let rest = size
        .checked_sub(4)
        .filter(|_| size.is_multiple_of(2))
        .ok_or(Rejection::Invalid)?;
    let mut chunks = input.take(rest)?;
    let mut alignment = None;
    let mut has_data = false;
    for _ in 0..MAX_BOXES {
        if chunks.is_empty() {
            return if has_data {
                Ok(())
            } else {
                Err(Rejection::Invalid)
            };
        }
        let (header, tail) = chunks.split_at_checked(8).ok_or(Rejection::Invalid)?;
        let size = le32(&header[4..]) as usize;
        let padded = size.checked_add(size % 2).ok_or(Rejection::Invalid)?;
        let (data, tail) = tail.split_at_checked(padded).ok_or(Rejection::Invalid)?;
        match &header[..4] {
            b"fmt " if alignment.is_none() => alignment = Some(pcm_alignment(&data[..size])?),
            b"data" if !has_data => {
                let alignment = alignment.ok_or(Rejection::Invalid)?;
                if size == 0 || !size.is_multiple_of(alignment) {
                    return Err(Rejection::Invalid);
                }
                has_data = true;
            }
            b"fmt " | b"data" => return Err(Rejection::Invalid),
            _ => {}
        }
        chunks = tail;
    }
    Err(Rejection::Limit)
}

fn pcm_alignment(data: &[u8]) -> Result<usize, Rejection> {
    let le16 = |offset| u16::from_le_bytes([data[offset], data[offset + 1]]);
    if !matches!(data.len(), 16 | 18) || le16(0) != 1 || (data.len() == 18 && le16(16) != 0) {
        return Err(Rejection::Invalid);
    }
    let channels = u32::from(le16(2));
    let rate = le32(&data[4..8]);
    let bits = u32::from(le16(14));
    let align = channels * (bits / 8);
    if !(1..=32).contains(&channels)
        || !matches!(bits, 8 | 16 | 24 | 32)
        || !(1..=768_000).contains(&rate)
        || u32::from(le16(12)) != align
        || rate.checked_mul(align) != Some(le32(&data[8..12]))
    {
        return Err(Rejection::Invalid);
    }
    Ok(align as usize)
}

pub(super) fn movie(input: &mut Candidate<impl Read>, quicktime: bool) -> Result<(), Rejection> {
    let first = input.take(8)?;
    let size = be32(&first[..4]) as usize;
    if &first[4..] != b"ftyp" || !(16..=4096).contains(&size) || !size.is_multiple_of(4) {
        return Err(Rejection::Invalid);
    }
    let brands = input.take(size - 8)?;
    let supported_brand = if quicktime {
        &brands[..4] == b"qt  "
    } else {
        matches!(
            &brands[..4],
            b"isom" | b"iso2" | b"mp41" | b"mp42" | b"avc1" | b"M4V " | b"MSNV"
        )
    };
    if !supported_brand {
        return Err(Rejection::Invalid);
    }
    let mut movie = None;
    let mut media = None;
    for _ in 0..MAX_BOXES {
        let start = input.bytes.len();
        let head: [u8; 8] = input.take(8)?.try_into().expect("box header");
        let (size, kind) = box_header(&head, || input.take(8).map(be64))?;
        let payload = input.bytes.len();
        let body = size
            .checked_sub(payload - start)
            .ok_or(Rejection::Invalid)?;
        match &kind {
            b"moov" if movie.is_none() => {
                movie = Some(payload..payload.checked_add(body).ok_or(Rejection::Invalid)?)
            }
            b"mdat" if media.is_none() && body > 0 => {
                media = Some(payload..payload.checked_add(body).ok_or(Rejection::Invalid)?)
            }
            b"free" | b"skip" | b"wide" => {}
            _ => return Err(Rejection::Invalid),
        }
        input.take(body)?;
        if let (Some(movie), Some(media)) = (&movie, &media) {
            return validate_movie(&input.bytes[movie.clone()], media);
        }
    }
    Err(Rejection::Limit)
}

fn box_header(
    head: &[u8; 8],
    extended_size: impl FnOnce() -> Result<u64, Rejection>,
) -> Result<(usize, [u8; 4]), Rejection> {
    let size = match be32(&head[..4]) {
        0 => return Err(Rejection::Invalid),
        1 => extended_size()?,
        size => u64::from(size),
    };
    Ok((
        usize::try_from(size).map_err(|_| Rejection::Limit)?,
        head[4..].try_into().expect("box type"),
    ))
}

fn boxes(data: &[u8]) -> Result<Vec<BoxContents<'_>>, Rejection> {
    let mut rest = data;
    let mut result = Vec::new();
    while !rest.is_empty() {
        if result.len() == MAX_BOXES {
            return Err(Rejection::Limit);
        }
        let header = rest.first_chunk::<8>().ok_or(Rejection::Invalid)?;
        let (size, kind) = box_header(header, || {
            rest.get(8..16).map(be64).ok_or(Rejection::Invalid)
        })?;
        let payload = if be32(&header[..4]) == 1 { 16 } else { 8 };
        result.push(BoxContents {
            kind,
            payload: rest.get(payload..size).ok_or(Rejection::Invalid)?,
        });
        rest = &rest[size..];
    }
    Ok(result)
}

fn required<'a>(boxes: &[BoxContents<'a>], kinds: &[[u8; 4]]) -> Result<&'a [u8], Rejection> {
    let mut found = boxes.iter().filter(|item| kinds.contains(&item.kind));
    let entry = found.next().ok_or(Rejection::Invalid)?;
    if found.next().is_some() {
        return Err(Rejection::Invalid);
    }
    Ok(entry.payload)
}

fn validate_movie(movie: &[u8], media: &Range<usize>) -> Result<(), Rejection> {
    let children = boxes(movie)?;
    if required(&children, &[*b"mvhd"])?.len() < 100
        || children.iter().any(|item| &item.kind == b"mvex")
    {
        return Err(Rejection::Invalid);
    }
    let mut has_video = false;
    let mut samples_left = MAX_SAMPLES;
    for track in children.iter().filter(|item| &item.kind == b"trak") {
        let track = boxes(track.payload)?;
        let track_media = boxes(required(&track, &[*b"mdia"])?)?;
        let handler = required(&track_media, &[*b"hdlr"])?;
        let minf = boxes(required(&track_media, &[*b"minf"])?)?;
        let table = boxes(required(&minf, &[*b"stbl"])?)?;
        validate_samples(&table, media, &mut samples_left)?;
        has_video |= handler.get(8..12) == Some(b"vide");
    }
    if has_video {
        Ok(())
    } else {
        Err(Rejection::Invalid)
    }
}

fn validate_samples(
    table: &[BoxContents<'_>],
    media: &Range<usize>,
    samples_left: &mut usize,
) -> Result<(), Rejection> {
    let sample_types = full_box_entries(required(table, &[*b"stsd"])?)?;
    if sample_types.0 == 0 || boxes(sample_types.1)?.len() != sample_types.0 {
        return Err(Rejection::Invalid);
    }
    let (chunk_count, chunk_data) = full_box_entries(required(table, &[*b"stco", *b"co64"])?)?;
    let chunk_width = if table.iter().any(|item| &item.kind == b"co64") {
        8
    } else {
        4
    };
    if chunk_count == 0 || chunk_count.checked_mul(chunk_width) != Some(chunk_data.len()) {
        return Err(Rejection::Invalid);
    }
    let (runs, run_bytes) = full_box_entries(required(table, &[*b"stsc"])?)?;
    let (runs_data, remainder) = run_bytes.as_chunks::<12>();
    if runs == 0 || runs_data.len() != runs || !remainder.is_empty() {
        return Err(Rejection::Invalid);
    }
    let sizes = required(table, &[*b"stsz"])?;
    if sizes.len() < 12 || sizes[..4] != [0; 4] {
        return Err(Rejection::Invalid);
    }
    let fixed_size = be32(&sizes[4..8]) as usize;
    let sample_count = be32(&sizes[8..12]) as usize;
    if sample_count == 0
        || chunk_count > sample_count
        || sizes.len() - 12
            != if fixed_size == 0 {
                sample_count.saturating_mul(4)
            } else {
                0
            }
    {
        return Err(Rejection::Invalid);
    }
    *samples_left = samples_left
        .checked_sub(sample_count)
        .ok_or(Rejection::Limit)?;
    let mut sample = 0;
    let mut next_run = 0;
    let mut samples_per_chunk = 0;
    let mut previous_end = media.start as u64;
    for (index, offset) in chunk_data.chunks_exact(chunk_width).enumerate() {
        if let Some(run) = runs_data.get(next_run)
            && be32(&run[..4]) as usize == index + 1
        {
            let description = be32(&run[8..]) as usize;
            if !(1..=sample_types.0).contains(&description) {
                return Err(Rejection::Invalid);
            }
            samples_per_chunk = be32(&run[4..8]) as usize;
            next_run += 1;
        }
        if samples_per_chunk == 0 || samples_per_chunk > sample_count - sample {
            return Err(Rejection::Invalid);
        }
        let next = sample + samples_per_chunk;
        let bytes = if fixed_size == 0 {
            let (entries, _) = sizes[12 + sample * 4..12 + next * 4].as_chunks::<4>();
            entries
                .iter()
                .copied()
                .map(u32::from_be_bytes)
                .map(u64::from)
                .sum()
        } else {
            (fixed_size as u64) * (samples_per_chunk as u64)
        };
        sample = next;
        let offset = if chunk_width == 8 {
            be64(offset)
        } else {
            u64::from(be32(offset))
        };
        let end = offset.checked_add(bytes).ok_or(Rejection::Invalid)?;
        if bytes == 0 || offset < previous_end || end > media.end as u64 {
            return Err(Rejection::Invalid);
        }
        previous_end = end;
    }
    if sample == sample_count && next_run == runs {
        Ok(())
    } else {
        Err(Rejection::Invalid)
    }
}

fn full_box_entries(data: &[u8]) -> Result<(usize, &[u8]), Rejection> {
    let header = data.get(..8).ok_or(Rejection::Invalid)?;
    if header[..4] != [0; 4] {
        return Err(Rejection::Invalid);
    }
    Ok((be32(&header[4..]) as usize, &data[8..]))
}

fn be32(data: &[u8]) -> u32 {
    u32::from_be_bytes(data.try_into().expect("four bytes"))
}
fn be64(data: &[u8]) -> u64 {
    u64::from_be_bytes(data.try_into().expect("eight bytes"))
}
fn le32(data: &[u8]) -> u32 {
    u32::from_le_bytes(data.try_into().expect("four bytes"))
}
