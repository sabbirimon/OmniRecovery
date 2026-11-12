use std::collections::HashSet;

pub(super) struct Directory {
    pub(super) extension: &'static str,
    pub(super) fields: usize,
    pub(super) replacement: Option<[u8; 8]>,
}

pub(super) fn inspect(bytes: &[u8]) -> Option<Directory> {
    const END_SIZE: usize = 22;
    if !bytes.starts_with(b"PK\x03\x04") || bytes.len() < END_SIZE {
        return None;
    }
    let end = (bytes.len().saturating_sub(END_SIZE + usize::from(u16::MAX))
        ..=bytes.len() - END_SIZE)
        .rev()
        .find(|&offset| {
            &bytes[offset..offset + 4] == b"PK\x05\x06"
                && offset + END_SIZE + word(&bytes[offset..], 20) == bytes.len()
        })?;
    let footer = &bytes[end..];
    let count = word(footer, 10);
    if footer[4..8] != [0; 4] || word(footer, 8) != count || !(1..=2048).contains(&count) {
        return None;
    }
    let offset = dword(footer, 16);
    let size = dword(footer, 12);
    let mut matches = [Some(offset), end.checked_sub(size)]
        .into_iter()
        .flatten()
        .filter_map(|start| {
            inspect_entries(bytes, start, end, count).map(|extension| (start, extension))
        });
    let (start, extension) = matches.next()?;
    if matches.any(|item| item != (start, extension)) {
        return None;
    }
    let mut fields = [0; 8];
    fields[..4].copy_from_slice(&u32::try_from(end - start).ok()?.to_le_bytes());
    fields[4..].copy_from_slice(&u32::try_from(start).ok()?.to_le_bytes());
    Some(Directory {
        extension,
        fields: end + 12,
        replacement: (footer[12..20] != fields).then_some(fields),
    })
}

fn inspect_entries(bytes: &[u8], start: usize, end: usize, count: usize) -> Option<&'static str> {
    let mut index = bytes.get(start..end)?;
    let mut names = HashSet::new();
    let mut document = None;
    for _ in 0..count {
        let header = index.get(..46)?;
        if &header[..4] != b"PK\x01\x02" || word(header, 34) != 0 || word(header, 8) & 1 != 0 {
            return None;
        }
        let name_end = 46 + word(header, 28);
        let name = index.get(46..name_end)?;
        if name.is_empty() || !names.insert(name) || name.ends_with(b"vbaProject.bin") {
            return None;
        }
        let local = bytes.get(dword(header, 42)..start)?;
        if local.len() < 30
            || &local[..4] != b"PK\x03\x04"
            || local[6..10] != header[8..12]
            || local.get(30..30 + word(local, 26))? != name
            || (30 + word(local, 26) + word(local, 28)).checked_add(dword(header, 20))?
                > local.len()
        {
            return None;
        }
        let extension = match name {
            b"word/document.xml" => Some("docx"),
            b"xl/workbook.xml" => Some("xlsx"),
            b"ppt/presentation.xml" => Some("pptx"),
            _ => None,
        };
        if let Some(extension) = extension
            && document.replace(extension).is_some()
        {
            return None;
        }
        index = index.get(name_end + word(header, 30) + word(header, 32)..)?;
    }
    if index.is_empty()
        && names.contains(b"[Content_Types].xml".as_slice())
        && names.contains(b"_rels/.rels".as_slice())
    {
        document
    } else {
        None
    }
}

fn word(bytes: &[u8], offset: usize) -> usize {
    usize::from(u16::from_le_bytes(
        bytes[offset..offset + 2]
            .try_into()
            .expect("ZIP 16-bit field"),
    ))
}

fn dword(bytes: &[u8], offset: usize) -> usize {
    u32::from_le_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("ZIP 32-bit field"),
    ) as usize
}
