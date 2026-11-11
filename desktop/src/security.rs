use serde::Serialize;
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};

#[derive(Serialize)]
pub struct SecurityReport {
    pub sha256: String,
    pub bytes: usize,
    pub engine: &'static str,
    pub result: &'static str,
    pub indicators: Vec<Indicator>,
}

#[derive(Serialize)]
pub struct Indicator {
    pub line: usize,
    pub kind: &'static str,
}

pub fn analyze(path: &Path, log: bool) -> Result<SecurityReport, String> {
    let limit = if log {
        8 * 1024 * 1024
    } else {
        32 * 1024 * 1024
    };
    let metadata = fs::symlink_metadata(path).map_err(error)?;
    if !metadata.is_file() || metadata.len() > limit {
        return Err("Select a regular file within the analysis size limit.".into());
    }
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(error)?
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(error)?;
    let sha256 = format!("{:x}", Sha256::digest(&bytes));
    if omnirecovery::verify_file(path).map_err(error)?.sha256 != sha256 {
        return Err("The selected file changed during analysis.".into());
    }
    let (engine, result, indicators) = if log {
        let source = std::str::from_utf8(&bytes)
            .map_err(|_| "Export binary event logs as UTF-8 text, CSV or XML first.")?;
        let indicators = log_indicators(source);
        (
            "local_log_rules",
            if indicators.is_empty() {
                "no_matching_indicators"
            } else {
                "review_indicators"
            },
            indicators,
        )
    } else {
        ("ClamAV", scan_clamav(&bytes)?, Vec::new())
    };
    Ok(SecurityReport {
        sha256,
        bytes: bytes.len(),
        engine,
        result,
        indicators,
    })
}

fn log_indicators(log: &str) -> Vec<Indicator> {
    log.lines()
        .enumerate()
        .filter_map(|(index, line)| {
            let line = line.to_ascii_lowercase();
            let kind = if line.contains("malware detected")
                || line.contains("threat detected")
                || line.contains("<eventid>1116</eventid>")
            {
                "reported_threat"
            } else if line.contains("quarantin") || line.contains("<eventid>1117</eventid>") {
                "possible_remediation_event"
            } else if (line.contains("clamav") || line.contains("clamscan"))
                && line.contains(" found")
            {
                "reported_clamav_detection"
            } else if line.contains("xprotect")
                && (line.contains("blocked") || line.contains("malware"))
            {
                "reported_xprotect_event"
            } else if line.contains("failed password") || line.contains("failed login") {
                "failed_authentication"
            } else {
                return None;
            };
            Some(Indicator {
                line: index + 1,
                kind,
            })
        })
        .take(1000)
        .collect()
}

fn scan_clamav(bytes: &[u8]) -> Result<&'static str, String> {
    let Some(engine) = [
        "/opt/homebrew/bin/clamscan",
        "/usr/local/bin/clamscan",
        "/usr/bin/clamscan",
    ]
    .into_iter()
    .find(|path| Path::new(path).is_file()) else {
        return Ok("engine_unavailable");
    };
    let mut child = Command::new(engine)
        .args([
            "--no-summary",
            "--infected",
            "--max-filesize=32M",
            "--max-scansize=64M",
            "--alert-exceeds-max=yes",
            "-",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(error)?;
    let mut input = child
        .stdin
        .take()
        .ok_or("Could not open the local scanner input.")?;
    let sent = input.write_all(bytes);
    drop(input);
    let exit = child.wait().map_err(error)?;
    Ok(match (exit.code(), sent.is_ok()) {
        (Some(0), true) => "no_detection",
        (Some(1), _) => "detected_or_scanner_limit_reached",
        _ => "scan_unavailable_or_database_error",
    })
}

fn error(error: impl std::fmt::Display) -> String {
    error.to_string()
}
