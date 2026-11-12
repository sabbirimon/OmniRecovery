use std::io::{self, BufRead, Cursor, Read};

use image::{ImageFormat, ImageReader, Limits};
use memchr::{memchr, memmem};
use serde::{Deserialize, Serialize};

mod audiovisual;

const PNG_MAGIC: &[u8] = b"\x89PNG\r\n\x1a\n";
const JPEG_MAGIC: &[u8] = b"\xff\xd8\xff";
pub(crate) const MAX_CANDIDATE_BYTES: usize = 16 * 1024 * 1024;
const MAX_PIXELS: u64 = 24_000_000;

#[derive(
    Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize, clap::ValueEnum,
)]
#[serde(rename_all = "snake_case")]
pub enum MediaType {
    Jpeg,
    Png,
    Mp4,
    Mov,
    Wav,
}

impl MediaType {
    pub(crate) fn detect(bytes: &[u8]) -> Option<Self> {
        match bytes.first() {
            Some(0xff) if bytes.starts_with(JPEG_MAGIC) => Some(Self::Jpeg),
            Some(0x89) if bytes.starts_with(PNG_MAGIC) => Some(Self::Png),
            Some(0) if bytes.get(4..8) == Some(b"ftyp") => {
                Some(if bytes.get(8..12) == Some(b"qt  ") {
                    Self::Mov
                } else {
                    Self::Mp4
                })
            }
            Some(b'R') if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WAVE") => {
                Some(Self::Wav)
            }
            _ => None,
        }
    }

    pub fn is_photo(self) -> bool {
        matches!(self, Self::Jpeg | Self::Png)
    }

    pub(crate) fn max_bytes(self) -> usize {
        if self.is_photo() {
            MAX_CANDIDATE_BYTES
        } else {
            128 * 1024 * 1024
        }
    }

    pub(crate) fn validate(self, reader: impl BufRead) -> Result<Validated, Rejection> {
        let mut candidate = Candidate {
            reader,
            bytes: Vec::new(),
            limit: self.max_bytes(),
        };
        let image_format = match self {
            Self::Jpeg => {
                jpeg(&mut candidate)?;
                ImageFormat::Jpeg
            }
            Self::Png => {
                png(&mut candidate)?;
                ImageFormat::Png
            }
            Self::Mp4 | Self::Mov | Self::Wav => {
                match self {
                    Self::Mp4 | Self::Mov => audiovisual::movie(&mut candidate, self == Self::Mov)?,
                    Self::Wav => audiovisual::wav(&mut candidate)?,
                    _ => unreachable!(),
                }
                return Ok(Validated {
                    bytes: candidate.bytes,
                    width: 0,
                    height: 0,
                });
            }
        };
        let mut reader = ImageReader::with_format(Cursor::new(&candidate.bytes), image_format);
        let mut limits = Limits::default();
        limits.max_image_width = Some(12_000);
        limits.max_image_height = Some(12_000);
        limits.max_alloc = Some(128 * 1024 * 1024);
        reader.limits(limits);
        let decoder = reader.into_decoder().map_err(decode_rejection)?;
        use image::ImageDecoder;
        let (width, height) = decoder.dimensions();
        if u64::from(width) * u64::from(height) > MAX_PIXELS {
            return Err(Rejection::Limit);
        }
        image::DynamicImage::from_decoder(decoder).map_err(decode_rejection)?;
        Ok(Validated {
            bytes: candidate.bytes,
            width,
            height,
        })
    }
}

/// Search for distinctive markers, then confirm only the matching positions.
pub(crate) fn signatures(
    bytes: &[u8],
    primary: usize,
    filter: Option<MediaType>,
) -> impl Iterator<Item = (usize, MediaType)> {
    use MediaType::{Jpeg, Mov, Mp4, Png, Wav};
    let mut streams = [
        (JPEG_MAGIC, 0, Jpeg, matches!(filter, None | Some(Jpeg))),
        (PNG_MAGIC, 0, Png, matches!(filter, None | Some(Png))),
        (
            b"ftyp".as_slice(),
            4,
            Mp4,
            matches!(filter, None | Some(Mp4 | Mov)),
        ),
        (
            b"RIFF".as_slice(),
            0,
            Wav,
            matches!(filter, None | Some(Wav)),
        ),
    ]
    .map(|(needle, prefix, expected, enabled)| {
        memmem::find_iter(if enabled { bytes } else { &[] }, needle)
            .filter_map(move |position| position.checked_sub(prefix))
            .take_while(move |&position| position < primary)
            .filter_map(move |position| {
                let kind = MediaType::detect(&bytes[position..])?;
                ((kind == expected || (expected == Mp4 && kind == Mov))
                    && filter.is_none_or(|media| media == kind))
                .then_some((position, kind))
            })
            .peekable()
    });
    std::iter::from_fn(move || {
        let (first, _) = streams
            .iter_mut()
            .enumerate()
            .filter_map(|(index, stream)| stream.peek().map(|&(position, _)| (index, position)))
            .min_by_key(|&(_, position)| position)?;
        streams[first].next()
    })
}

fn decode_rejection(error: image::ImageError) -> Rejection {
    match error {
        image::ImageError::Limits(_) => Rejection::Limit,
        _ => Rejection::Invalid,
    }
}

pub(crate) struct Validated {
    pub(crate) bytes: Vec<u8>,
    pub(crate) width: u32,
    pub(crate) height: u32,
}

pub(crate) enum Rejection {
    Invalid,
    Truncated,
    Limit,
    Io(io::Error),
}

struct Candidate<R> {
    reader: R,
    bytes: Vec<u8>,
    limit: usize,
}

impl<R: Read> Candidate<R> {
    fn take(&mut self, length: usize) -> Result<&[u8], Rejection> {
        let start = self.bytes.len();
        if length > self.limit - start {
            return Err(Rejection::Limit);
        }
        // Grow as observed bytes arrive. A hostile size field cannot allocate its full
        // declared payload before the reader has supplied it.
        while self.bytes.len() < start + length {
            let offset = self.bytes.len();
            self.bytes
                .resize((start + length).min(offset + 64 * 1024), 0);
            self.reader
                .read_exact(&mut self.bytes[offset..])
                .map_err(|error| {
                    if error.kind() == io::ErrorKind::UnexpectedEof {
                        Rejection::Truncated
                    } else {
                        Rejection::Io(error)
                    }
                })?;
        }
        Ok(&self.bytes[start..])
    }

    fn byte(&mut self) -> Result<u8, Rejection> {
        Ok(self.take(1)?[0])
    }
}

impl<R: BufRead> Candidate<R> {
    fn jpeg_scan_marker(&mut self) -> Result<(), Rejection> {
        while self.bytes.len() < self.limit {
            let available = self.reader.fill_buf().map_err(Rejection::Io)?;
            let limit = available.len().min(self.limit - self.bytes.len());
            if limit == 0 {
                return Err(Rejection::Truncated);
            }
            let marker = memchr(0xff, &available[..limit]);
            let count = marker.map_or(limit, |position| position + 1);
            self.bytes.extend_from_slice(&available[..count]);
            self.reader.consume(count);
            if marker.is_some() {
                return Ok(());
            }
        }
        Err(Rejection::Limit)
    }
}

fn png(reader: &mut Candidate<impl Read>) -> Result<(), Rejection> {
    if reader.take(8)? != PNG_MAGIC {
        return Err(Rejection::Invalid);
    }
    let mut first = true;
    let mut saw_data = false;
    loop {
        let header: [u8; 8] = reader.take(8)?.try_into().expect("PNG chunk header");
        let size = u32::from_be_bytes(header[..4].try_into().expect("PNG chunk size"));
        let chunk = &header[4..];
        if !chunk.iter().all(u8::is_ascii_alphabetic)
            || (first && (chunk != b"IHDR" || size != 13))
            || (!first && chunk == b"IHDR")
            || matches!(chunk, b"acTL" | b"fcTL" | b"fdAT")
        {
            return Err(Rejection::Invalid);
        }
        first = false;
        let mut crc = crc32fast::Hasher::new();
        crc.update(chunk);
        crc.update(reader.take(size as usize)?);
        let expected = u32::from_be_bytes(reader.take(4)?.try_into().expect("PNG chunk CRC"));
        if crc.finalize() != expected {
            return Err(Rejection::Invalid);
        }
        if chunk == b"IDAT" {
            saw_data = true;
        }
        if chunk == b"IEND" {
            return if size == 0 && saw_data {
                Ok(())
            } else {
                Err(Rejection::Invalid)
            };
        }
    }
}

fn jpeg(reader: &mut Candidate<impl BufRead>) -> Result<(), Rejection> {
    if reader.take(2)? != &JPEG_MAGIC[..2] {
        return Err(Rejection::Invalid);
    }
    let mut in_scan = false;
    let mut saw_scan = false;
    loop {
        if in_scan {
            reader.jpeg_scan_marker()?;
        } else if reader.byte()? != 0xff {
            return Err(Rejection::Invalid);
        }
        let mut marker = reader.byte()?;
        while marker == 0xff {
            marker = reader.byte()?;
        }
        if in_scan && matches!(marker, 0x00 | 0xd0..=0xd7) {
            continue;
        }
        in_scan = false;
        match marker {
            0xd9 if saw_scan => return Ok(()),
            0x00 | 0xd0..=0xd9 => return Err(Rejection::Invalid),
            0x01 => continue,
            _ => {
                let size =
                    u16::from_be_bytes(reader.take(2)?.try_into().expect("JPEG segment size"));
                let payload = size.checked_sub(2).ok_or(Rejection::Invalid)?;
                reader.take(usize::from(payload))?;
                if marker == 0xda {
                    in_scan = true;
                    saw_scan = true;
                }
            }
        }
    }
}
