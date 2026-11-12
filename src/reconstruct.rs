use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::source::{hash_image, invalid, open_image, same_file_state};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Span {
    offset: u64,
    length: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Reconstruction {
    source_sha256: String,
    expected_sha256: String,
    spans: Vec<Span>,
}

#[derive(Debug, Serialize)]
pub struct ReconstructionReport {
    pub source_sha256: String,
    pub output_sha256: String,
    pub spans: usize,
    pub bytes: u64,
}

fn valid_hash(hash: &str) -> bool {
    hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// Rebuild an artifact from ordered non-overlapping raw ranges and publish only a trusted hash match.
pub fn reconstruct_file(
    image: &Path,
    manifest: &Path,
    destination: &Path,
) -> io::Result<ReconstructionReport> {
    let mut manifest_bytes = Vec::new();
    open_image(manifest)?
        .take(262_145)
        .read_to_end(&mut manifest_bytes)?;
    if manifest_bytes.len() > 262_144 {
        return Err(invalid("reconstruction plan exceeds size limit"));
    }
    let plan: Reconstruction = serde_json::from_slice(&manifest_bytes)?;
    if !(1..=4096).contains(&plan.spans.len())
        || !valid_hash(&plan.source_sha256)
        || !valid_hash(&plan.expected_sha256)
    {
        return Err(invalid(
            "reconstruction requires ordered ranges and trusted source and output digests",
        ));
    }
    let mut source = open_image(image)?;
    let state = source.metadata()?;
    if crate::source::is_sparse_image(&mut source)? {
        return Err(invalid("reconstruction accepts raw image offsets only"));
    }
    let mut intervals = Vec::with_capacity(plan.spans.len());
    let mut total = 0_u64;
    for span in &plan.spans {
        let end = span
            .offset
            .checked_add(span.length)
            .filter(|end| *end <= state.len() && span.length > 0)
            .ok_or_else(|| invalid("reconstruction span exceeds source image"))?;
        total = total
            .checked_add(span.length)
            .filter(|bytes| *bytes <= 64 * 1024 * 1024 * 1024)
            .ok_or_else(|| invalid("reconstruction exceeds the 64 GiB output limit"))?;
        intervals.push((span.offset, end));
    }
    intervals.sort_unstable();
    if intervals.windows(2).any(|pair| pair[0].1 > pair[1].0) {
        return Err(invalid("reconstruction spans overlap"));
    }
    let source_sha256 = hash_image(&mut source, state.len())?;
    if !source_sha256.eq_ignore_ascii_case(&plan.source_sha256)
        || !same_file_state(&state, &source.metadata()?)
    {
        return Err(invalid("reconstruction source failed verification"));
    }
    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut output = tempfile::NamedTempFile::new_in(parent)?;
    let mut buffer = vec![0_u8; 1024 * 1024];
    let mut digest = Sha256::new();
    for span in &plan.spans {
        source.seek(SeekFrom::Start(span.offset))?;
        let mut remaining = span.length;
        while remaining != 0 {
            let count = remaining.min(buffer.len() as u64) as usize;
            source.read_exact(&mut buffer[..count])?;
            output.write_all(&buffer[..count])?;
            digest.update(&buffer[..count]);
            remaining -= count as u64;
        }
    }
    let output_sha256 = format!("{:x}", digest.finalize());
    if !output_sha256.eq_ignore_ascii_case(&plan.expected_sha256)
        || !same_file_state(&state, &source.metadata()?)
    {
        return Err(invalid(
            "reconstruction did not match the original digest or the source changed",
        ));
    }
    output.as_file().sync_all()?;
    if hash_image(output.as_file_mut(), total)? != output_sha256 {
        return Err(invalid(
            "reconstruction readback did not match the expected digest",
        ));
    }
    output
        .persist_noclobber(destination)
        .map_err(|error| error.error)?;
    Ok(ReconstructionReport {
        source_sha256,
        output_sha256,
        spans: plan.spans.len(),
        bytes: total,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reconstructs_disjoint_ranges_only_with_original_hash() -> io::Result<()> {
        let temp = tempfile::tempdir()?;
        let image = temp.path().join("image.dd");
        let plan = temp.path().join("extents.json");
        let destination = temp.path().join("output");
        let bytes = b"world---hello ";
        std::fs::write(&image, bytes)?;
        let mut data = serde_json::json!({
            "source_sha256": format!("{:x}", Sha256::digest(bytes)),
            "expected_sha256": format!("{:x}", Sha256::digest(b"hello world")),
            "spans": [{"offset":8,"length":6},{"offset":0,"length":5}]
        });
        std::fs::write(&plan, serde_json::to_vec(&data)?)?;
        assert_eq!(reconstruct_file(&image, &plan, &destination)?.bytes, 11);
        assert_eq!(std::fs::read(&destination)?, b"hello world");
        data["spans"][1]["offset"] = serde_json::json!(9);
        std::fs::write(&plan, serde_json::to_vec(&data)?)?;
        assert!(reconstruct_file(&image, &plan, &temp.path().join("other")).is_err());
        assert!(!temp.path().join("other").exists());
        Ok(())
    }
}
