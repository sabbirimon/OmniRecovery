use std::fs::{self, File, FileTimes};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::acquisition::{adapter_command, finish, now_ms, run_adapter, stage};
use crate::source::{invalid, open_image, same_file_state};
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
    let started = now_ms();
    let (_evidence, path, input_sha256) =
        stage_raw_image(image, output_parent, sector_offset, sector_size)?;
    let version = run_adapter(adapter_command("fls")?.arg("-V"), Duration::from_secs(10))?;
    let text = list_deleted(&path, sector_offset, sector_size)?;
    check_staged_image(&path, &input_sha256)?;
    let (result, data) = stage(output_parent)?;
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

fn stage_raw_image(
    image: &Path,
    output_parent: &Path,
    sector_offset: u64,
    sector_size: u32,
) -> io::Result<(tempfile::TempDir, PathBuf, String)> {
    if !matches!(sector_size, 512 | 4096) {
        return Err(invalid("sector size must be 512 or 4096 bytes"));
    }
    let mut source = open_image(image)?;
    if crate::source::is_sparse_image(&mut source)? {
        return Err(invalid("choose a regular raw image"));
    }
    let source_size = source.metadata()?.len();
    if sector_offset
        .checked_mul(u64::from(sector_size))
        .is_none_or(|offset| offset >= source_size)
    {
        return Err(invalid("the filesystem offset is outside the source image"));
    }
    let output_parent = fs::canonicalize(output_parent)?;
    let evidence = tempfile::Builder::new()
        .prefix("omnirecovery-raw-input-")
        .tempdir_in(&output_parent)?;
    let copy = crate::acquire_image(image, evidence.path())?;
    let path = copy.output.join("data/capture.dd");
    let input_sha256 = copy.files.files["capture.dd"].sha256.clone();
    Ok((evidence, path, input_sha256))
}

fn check_staged_image(path: &Path, sha256: &str) -> io::Result<()> {
    if verify_file(path)?.sha256 != sha256 {
        return Err(invalid("the staged image changed during analysis"));
    }
    Ok(())
}

fn tsk_image_args(command: &mut Command, path: &Path, offset: u64, size: u32) {
    command
        .args([
            "-i",
            "raw",
            "-b",
            &size.to_string(),
            "-o",
            &offset.to_string(),
        ])
        .arg(path);
}

fn list_deleted(path: &Path, offset: u64, size: u32) -> io::Result<String> {
    let mut command = adapter_command("fls")?;
    command.args(["-d", "-r", "-p"]);
    tsk_image_args(&mut command, path, offset, size);
    run_adapter(&mut command, Duration::from_secs(120))
}

#[derive(Serialize)]
pub struct DeletedEntry<'a> {
    pub address: &'a str,
    pub path: &'a str,
}

/// Read ordinary deleted-file addresses from a Sleuth Kit listing.
pub fn tsk_deleted_entries(listing: &str) -> impl Iterator<Item = DeletedEntry<'_>> {
    listing.lines().filter_map(|line| {
        let (header, path) = line.split_once(":\t")?;
        let mut fields = header.split_whitespace();
        let (entry_type, inode_type) = fields.next()?.split_once('/')?;
        if !matches!(entry_type, "r" | "-") || inode_type != "r" || fields.next()? != "*" {
            return None;
        }
        let address = fields.next()?;
        if fields.next().is_some() || !valid_tsk_address(address) || path.is_empty() {
            return None;
        }
        Some(DeletedEntry { address, path })
    })
}

fn valid_tsk_address(address: &str) -> bool {
    address.len() <= 64
        && address.split('-').count() <= 3
        && address
            .split('-')
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
}

/// Extract a deleted-file candidate from the same image and filesystem as the listing.
pub fn tsk_recover(
    image: &Path,
    output_parent: &Path,
    sector_offset: u64,
    sector_size: u32,
    address: &str,
    expected_sha256: &str,
) -> io::Result<AcquisitionReport> {
    if !valid_tsk_address(address) {
        return Err(invalid("choose a valid deleted file address"));
    }
    let started = now_ms();
    let (_evidence, path, input_sha256) =
        stage_raw_image(image, output_parent, sector_offset, sector_size)?;
    if !input_sha256.eq_ignore_ascii_case(expected_sha256) {
        return Err(invalid(
            "the image hash differs from the deleted-entry listing",
        ));
    }
    let listing = list_deleted(&path, sector_offset, sector_size)?;
    let entry = tsk_deleted_entries(&listing)
        .find(|entry| entry.address == address)
        .ok_or_else(|| invalid("the address is not a recoverable deleted file in this listing"))?;
    let (result, data) = stage(output_parent)?;
    let recovered = data.join("recovered.bin");
    let mut command = adapter_command("icat")?;
    command.arg("-r");
    tsk_image_args(&mut command, &path, sector_offset, sector_size);
    command
        .arg(address)
        .stdout(Stdio::from(File::create_new(&recovered)?));
    let limit = fs::metadata(&path)?.len();
    wait_recovery_helper(&mut command, Duration::from_secs(300), || {
        Ok(fs::metadata(&recovered)?.len() <= limit)
    })?;
    check_staged_image(&path, &input_sha256)?;
    let file = verify_file(&recovered)?;
    if file.length == 0 || file.length > limit {
        return Err(invalid("Sleuth Kit did not return a usable file candidate"));
    }
    write_analysis(
        &data,
        &json!({
            "adapter": "icat", "version": run_adapter(adapter_command("icat")?.arg("-V"), Duration::from_secs(10))?.trim(),
            "source_sha256": input_sha256, "filesystem_address": address, "listed_path": entry.path,
            "sector_offset": sector_offset, "sector_size": sector_size,
            "interpretation": "Recovered candidate. Its hash verifies the saved bytes. Deleted extents can contain reused data."
        }),
    )?;
    let files = inventory(&data)?;
    finish(
        result,
        files,
        "sleuth_kit_deleted_recovery",
        image.display().to_string(),
        "raw image",
        started,
        "verified_staging_and_hashed_reports",
    )
}

fn wait_recovery_helper(
    command: &mut Command,
    timeout: Duration,
    within_limit: impl Fn() -> io::Result<bool>,
) -> io::Result<()> {
    let mut child = command.stdin(Stdio::null()).stderr(Stdio::null()).spawn()?;
    let start = Instant::now();
    let outcome = loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break Ok(()),
            Ok(Some(_)) => return Err(invalid("recovery helper did not complete successfully")),
            Err(error) => break Err(error),
            Ok(None) => {}
        }
        match within_limit() {
            Ok(true) if start.elapsed() < timeout => std::thread::sleep(Duration::from_millis(50)),
            Ok(_) => break Err(invalid("recovery helper exceeded its time or output limit")),
            Err(error) => break Err(error),
        }
    };
    if outcome.is_err() {
        let _ = child.kill();
        let _ = child.wait();
    }
    outcome
}

/// Run TestDisk's partition listing on a verified staging copy.
pub fn testdisk_list(image: &Path, output_parent: &Path) -> io::Result<AcquisitionReport> {
    let started = now_ms();
    let mut command = adapter_command(if cfg!(windows) {
        "testdisk_win"
    } else {
        "testdisk"
    })?;
    let (evidence, path, input_sha256) = stage_raw_image(image, output_parent, 0, 512)?;
    let listing = run_adapter(
        command.current_dir(evidence.path()).arg("-l").arg(&path),
        Duration::from_secs(120),
    )?;
    if listing.trim().is_empty() {
        return Err(invalid("TestDisk returned no partition report"));
    }
    check_staged_image(&path, &input_sha256)?;
    let (result, data) = stage(output_parent)?;
    fs::write(data.join("testdisk-partitions.txt"), listing)?;
    write_analysis(
        &data,
        &json!({"adapter": "testdisk -l", "source_sha256": input_sha256}),
    )?;
    let files = inventory(&data)?;
    finish(
        result,
        files,
        "testdisk_partition_listing",
        image.display().to_string(),
        "raw image partition inventory",
        started,
        "verified_staging_and_hashed_reports",
    )
}

/// Carve a raw image copy with a separately installed PhotoRec.
pub fn photorec_carve(image: &Path, output_parent: &Path) -> io::Result<AcquisitionReport> {
    let started = now_ms();
    let mut command = adapter_command(if cfg!(windows) {
        "photorec_win"
    } else {
        "photorec"
    })?;
    let (evidence, path, input_sha256) = stage_raw_image(image, output_parent, 0, 512)?;
    let (result, data) = stage(output_parent)?;
    let stdout = tempfile::tempfile()?;
    command.current_dir(evidence.path())
        .env("TERM", "dumb")
        .args(["/d"]).arg(data.join("carved"))
        .arg("/cmd").arg(&path)
        .arg("partition_none,options,keep_corrupted_file_no,fileopt,everything,enable,wholespace,search")
        .stdout(Stdio::from(stdout.try_clone()?));
    wait_recovery_helper(&mut command, Duration::from_secs(600), || {
        Ok(stdout.metadata()?.len() <= 8 * 1024 * 1024)
    })?;
    check_staged_image(&path, &input_sha256)?;
    let carved = inventory(&data)?;
    if !carved
        .files
        .keys()
        .any(|path| path.starts_with("carved.") && !path.ends_with("/report.xml"))
    {
        return Err(invalid("PhotoRec produced no carved files"));
    }
    write_analysis(
        &data,
        &json!({"adapter": "photorec", "source_sha256": input_sha256,
        "interpretation": "Whole-image carving candidates. Allocation and deletion status are unknown."}),
    )?;
    let files = inventory(&data)?;
    finish(
        result,
        files,
        "photorec_carving",
        image.display().to_string(),
        "whole raw image",
        started,
        "verified_staging_and_hashed_candidates",
    )
}

/// Import a bounded FQLite CASE export as data without resolving JSON-LD contexts.
pub fn import_fqlite_report(path: &Path, output_parent: &Path) -> io::Result<AcquisitionReport> {
    const LIMIT: u64 = 16 * 1024 * 1024;
    let started = now_ms();
    let mut file = open_image(path)?;
    let state = file.metadata()?;
    let mut bytes = Vec::new();
    (&mut file).take(LIMIT + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > LIMIT || !same_file_state(&state, &file.metadata()?) {
        return Err(invalid(
            "FQLite report exceeds 16 MiB or changed while reading",
        ));
    }
    let report: serde_json::Value = serde_json::from_slice(&bytes)?;
    let objects = report["uco-core:object"]
        .as_array()
        .filter(|_| report["@type"] == "uco-core:Bundle")
        .ok_or_else(|| invalid("choose a FQLite CASE JSON-LD export"))?;
    let tool = objects
        .iter()
        .find(|item| item["@type"] == "uco-tool:Tool" && item["uco-core:name"] == "FQLite")
        .ok_or_else(|| invalid("the report does not identify FQLite as its source"))?;
    let source = objects
        .iter()
        .find(|item| item["@type"] == "uco-observable:File" && item["@id"] == "kb:file-source")
        .ok_or_else(|| invalid("the FQLite report is missing its SQLite source metadata"))?;
    let mut count = 0;
    let mut preview = Vec::new();
    for item in objects
        .iter()
        .filter(|item| item["@type"] == "uco-observable:CyberItem")
    {
        count += 1;
        if preview.len() < 100 {
            preview.push(json!({
                "table": item["uco-core:tag"],
                "offset": item["case-investigation:exhibitNumber"],
            }));
        }
    }
    let (staging, data) = stage(output_parent)?;
    fs::write(data.join("fqlite-report.jsonld"), &bytes)?;
    write_analysis(
        &data,
        &json!({
            "adapter": "FQLite CASE import", "version": tool["uco-tool:toolVersion"],
            "report_sha256": format!("{:x}", Sha256::digest(&bytes)),
            "reported_source_name": source["uco-observable:fileName"],
            "reported_source_hashes": source["uco-observable:hash"],
            "reported_rows": count, "row_preview": preview,
        }),
    )?;
    let files = inventory(&data)?;
    finish(
        staging,
        files,
        "fqlite_case_import",
        path.display().to_string(),
        "imported report and claimed SQLite source metadata",
        started,
        "hashed_imported_report",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imports_fqlite_case_rows_and_preserves_original_report_bytes() -> io::Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("case.jsonld");
        let fixture = br#"{
          "@context": "https://example.invalid/context",
          "@type": "uco-core:Bundle",
          "uco-core:object": [
            {"@type": "uco-tool:Tool", "uco-core:name": "FQLite", "uco-tool:toolVersion": "vTest"},
            {"@type": "uco-observable:File", "@id": "kb:file-source", "uco-observable:fileName": "fixture.sqlite"},
            {"@type": "uco-observable:CyberItem", "uco-core:tag": "messages", "case-investigation:exhibitNumber": "4096"}
          ]
        }"#;
        fs::write(&path, fixture)?;
        let report = import_fqlite_report(&path, directory.path())?;
        let data = report.output.join("data");
        assert_eq!(fs::read(data.join("fqlite-report.jsonld"))?, fixture);
        let details: serde_json::Value =
            serde_json::from_reader(File::open(data.join("omnirecovery-analysis.json"))?)?;
        assert_eq!(details["reported_rows"], 1);
        assert_eq!(details["row_preview"][0]["offset"], "4096");
        fs::write(&path, b"{}")?;
        assert!(import_fqlite_report(&path, directory.path()).is_err());
        Ok(())
    }
}
