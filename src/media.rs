use std::io::{self, Cursor, Read};

use image::{ImageFormat, ImageReader, Limits};
use serde::{Deserialize, Serialize};

const PNG_MAGIC: &[u8] = b"\x89PNG\r\n\x1a\n";
const JPEG_MAGIC: &[u8] = b"\xff\xd8\xff";
pub(crate) const MAX_CANDIDATE_BYTES: usize = 16 * 1024 * 1024;
const MAX_PIXELS: u64 = 24_000_000;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaType {
    Jpeg,
    Png,
}

impl MediaType {
    pub(crate) fn detect(bytes: &[u8]) -> Option<Self> {
        match bytes.first() {
            Some(0xff) if bytes.starts_with(JPEG_MAGIC) => Some(Self::Jpeg),
            Some(0x89) if bytes.starts_with(PNG_MAGIC) => Some(Self::Png),
            _ => None,
        }
    }

    pub(crate) fn validate(self, reader: impl Read) -> Result<Validated, Rejection> {
        let mut candidate = Candidate {
            reader,
            bytes: Vec::new(),
        };
        match self {
            Self::Jpeg => jpeg(&mut candidate)?,
            Self::Png => png(&mut candidate)?,
        }
        let image_format = match self {
            Self::Jpeg => ImageFormat::Jpeg,
            Self::Png => ImageFormat::Png,
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
}

impl<R: Read> Candidate<R> {
    fn take(&mut self, length: usize) -> Result<&[u8], Rejection> {
        let start = self.bytes.len();
        if length > MAX_CANDIDATE_BYTES - start {
            return Err(Rejection::Limit);
        }
        self.bytes.resize(start + length, 0);
        let bytes = &mut self.bytes[start..];
        self.reader.read_exact(bytes).map_err(|error| {
            if error.kind() == io::ErrorKind::UnexpectedEof {
                Rejection::Truncated
            } else {
                Rejection::Io(error)
            }
        })?;
        Ok(bytes)
    }

    fn byte(&mut self) -> Result<u8, Rejection> {
        Ok(self.take(1)?[0])
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

fn jpeg(reader: &mut Candidate<impl Read>) -> Result<(), Rejection> {
    if reader.take(2)? != &JPEG_MAGIC[..2] {
        return Err(Rejection::Invalid);
    }
    let mut in_scan = false;
    let mut saw_scan = false;
    loop {
        let byte = reader.byte()?;
        if byte != 0xff {
            if in_scan {
                continue;
            }
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
