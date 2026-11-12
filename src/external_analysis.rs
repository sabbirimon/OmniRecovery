use std::fs::{self, File, FileTimes};
use std::io::{self, Write};
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use serde::Serialize;
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::acquisition::{adapter_command, finish, now_ms, run_adapter, stage};
use crate::source::{invalid, open_image};
use crate::{AcquisitionReport, ExportInventory, catalogue_file, inventory, verify_file};

#[derive(Clone, Copy, Debug, clap::ValueEnum)]
pub enum LeappEngine {
    Aleapp,
    Ileapp,
}

impl LeappEngine {
    fn entrypoint(self) -> &'static str {
        match self {
            Self::Aleapp => "aleapp.py",
            Self::Ileapp => "ileapp.py",
        }
    }
}

#[derive(Clone, Copy, Debug, clap::ValueEnum)]
pub enum LeappInput {
    Fs,
    Itunes,
}

fn write_analysis(directory: &Path, details: &impl Serialize) -> io::Result<()> {
    let mut output = File::create_new(directory.join("omnirecovery-analysis.json"))?;
    serde_json::to_writer_pretty(&mut output, details)?;
    output.write_all(b"\n")?;
    output.sync_all()
}

fn copy_export(source: &Path, directory: &Path) -> io::Result<ExportInventory> {
    let expected = inventory(source)?;
    if expected.files.is_empty() || !expected.skipped.is_empty() {
        return Err(invalid(
            "use a nonempty export without symlinks, mounts or special entries",
        ));
    }
    let length = expected.files.values().try_fold(0_u64, |total, item| {
        total
            .checked_add(item.length)
            .filter(|size| *size <= 64 * 1024 * 1024 * 1024)
            .ok_or_else(|| invalid("the selected export exceeds the 64 GiB staging limit"))
    })?;
    if length == 0 {
        return Err(invalid("no source bytes are available for analysis"));
    }
    for (relative, observation) in &expected.files {
        let mut input = open_image(&catalogue_file(source, relative)?)?;
        let metadata = input.metadata()?;
        let path = directory.join(relative);
        fs::create_dir_all(
            path.parent()
                .ok_or_else(|| invalid("invalid staged export path"))?,
        )?;
        let mut output = File::create_new(path)?;
        if io::copy(&mut (&mut input).take(observation.length + 1), &mut output)?
            != observation.length
        {
            return Err(invalid("source changed while copying the export"));
        }
        let mut times = FileTimes::new();
        if let Ok(modified) = metadata.modified() {
            times = times.set_modified(modified);
        }
        output.set_times(times)?;
    }
    let copied = inventory(directory)?;
    if copied.files != expected.files || !copied.skipped.is_empty() {
        return Err(invalid("staged export did not match the source manifest"));
    }
    Ok(expected)
}

use std::io::Read;

/// Run an explicitly installed ALEAPP or iLEAPP checkout against an isolated export copy.
pub fn analyze_leapp(
    engine: LeappEngine,
    format: LeappInput,
    input: &Path,
    checkout: &Path,
    python: &Path,
    expected_script_sha256: &str,
    output_parent: &Path,
) -> io::Result<AcquisitionReport> {
    if matches!((engine, format), (LeappEngine::Aleapp, LeappInput::Itunes)) {
        return Err(invalid("iTunes input requires iLEAPP"));
    }
    if !python.is_absolute() || !python.is_file() {
        return Err(invalid(
            "provide the absolute path of the trusted Python environment",
        ));
    }
    let input = fs::canonicalize(input)?;
    let checkout = fs::canonicalize(checkout)?;
    let output_parent = fs::canonicalize(output_parent)?;
    if output_parent.starts_with(&input) || output_parent.starts_with(&checkout) {
        return Err(invalid(
            "analysis destination must be outside the input and parser checkout",
        ));
    }
    let script = checkout.join(engine.entrypoint());
    let script_digest = verify_file(&script)?.sha256;
    if !script_digest.eq_ignore_ascii_case(expected_script_sha256) {
        return Err(invalid("parser entrypoint digest did not match"));
    }
    let started = now_ms();
    let temp = tempfile::Builder::new()
        .prefix("omnirecovery-analysis-input-")
        .tempdir_in(&output_parent)?;
    let evidence = temp.path().join("source");
    fs::create_dir(&evidence)?;
    let source = copy_export(&input, &evidence)?;
    let source_sha256 = format!("{:x}", Sha256::digest(serde_json::to_vec(&source.files)?));
    let (result, data) = stage(&output_parent)?;
    let format = match format {
        LeappInput::Fs => "fs",
        LeappInput::Itunes => "itunes",
    };
    let mut command = Command::new(python);
    command
        .env_remove("PYTHONPATH")
        .env_remove("PYTHONHOME")
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .current_dir(temp.path())
        .arg("-s")
        .arg(&script)
        .args(["-t", format, "-i"])
        .arg(&evidence)
        .arg("-o")
        .arg(&data);
    run_adapter(&mut command, Duration::from_secs(120))?;
    if verify_file(&script)?.sha256 != script_digest || inventory(&evidence)?.files != source.files
    {
        return Err(invalid(
            "parser entrypoint or staged evidence changed during analysis",
        ));
    }
    if inventory(&data)?.files.is_empty() {
        return Err(invalid("parser produced no report files"));
    }
    write_analysis(
        &data,
        &json!({
            "adapter": engine.entrypoint(),
            "format": format,
            "entrypoint_sha256": script_digest,
            "source_manifest_sha256": source_sha256,
            "source_files": source.files.len(),
        }),
    )?;
    let files = inventory(&data)?;
    finish(
        result,
        files,
        "external_leapp_analysis",
        input.display().to_string(),
        format,
        started,
        "verified_staging_and_hashed_reports",
    )
}

/// List deleted entries using an installed Sleuth Kit fls against a verified raw image copy.
pub fn tsk_deleted(
    image: &Path,
    output_parent: &Path,
    sector_offset: u64,
    sector_size: u32,
) -> io::Result<AcquisitionReport> {
    if !matches!(sector_size, 512 | 4096) {
        return Err(invalid("sector size must be 512 or 4096 bytes"));
    }
    let mut source = open_image(image)?;
    if crate::source::is_sparse_image(&mut source)? {
        return Err(invalid("Sleuth Kit input requires a regular raw image"));
    }
    let source_size = source.metadata()?.len();
    if sector_offset
        .checked_mul(u64::from(sector_size))
        .is_none_or(|offset| offset >= source_size)
    {
        return Err(invalid("the filesystem offset is outside the source image"));
    }
    let started = now_ms();
    let output_parent = fs::canonicalize(output_parent)?;
    let evidence = tempfile::Builder::new()
        .prefix("omnirecovery-tsk-input-")
        .tempdir_in(&output_parent)?;
    let copy = crate::acquire_image(image, evidence.path())?;
    let path = copy.output.join("data/capture.dd");
    let input_sha256 = copy.files.files["capture.dd"].sha256.clone();
    let version = run_adapter(adapter_command("fls")?.arg("-V"), Duration::from_secs(10))?;
    let text = run_adapter(
        adapter_command("fls")?
            .args([
                "-d",
                "-r",
                "-p",
                "-i",
                "raw",
                "-b",
                &sector_size.to_string(),
                "-o",
                &sector_offset.to_string(),
            ])
            .arg(&path),
        Duration::from_secs(120),
    )?;
    if verify_file(&path)?.sha256 != input_sha256 {
        return Err(invalid("Sleuth Kit staging image changed"));
    }
    let (result, data) = stage(&output_parent)?;
    let mut listing = File::create_new(data.join("tsk-deleted.txt"))?;
    listing.write_all(text.as_bytes())?;
    listing.sync_all()?;
    drop(listing);
    write_analysis(
        &data,
        &json!({
            "adapter": "fls", "version": version.trim(), "source_sha256": input_sha256,
            "sector_offset": sector_offset, "sector_size": sector_size,
            "interpretation": "Filesystem entries reported as deleted by the installed Sleuth Kit. Listing an entry does not establish that its original bytes remain."
        }),
    )?;
    let files = inventory(&data)?;
    finish(
        result,
        files,
        "sleuth_kit_deleted_listing",
        image.display().to_string(),
        "raw image",
        started,
        "verified_staging_and_hashed_reports",
    )
}
