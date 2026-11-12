use std::io::{self, BufRead, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use omnirecovery::{MediaType, ScanOptions, StorageProfile};
use zeroize::Zeroizing;

#[derive(Parser)]
#[command(
    version,
    about = "Offline inspection of previously acquired storage images and Android exports"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Scan a regular raw or Android sparse image and emit a JSON findings report.
    Scan {
        image: PathBuf,
        /// Record the original storage type supplied by the operator.
        #[arg(long, value_enum, default_value = "auto")]
        storage: StorageProfile,
        /// Scan only this media type. By default scan all supported formats.
        #[arg(long, value_enum)]
        media: Option<MediaType>,
        /// Start at this zero-based logical byte offset.
        #[arg(long, default_value_t = 0)]
        offset: u64,
        /// Limit carving to this many bytes. By default scan to the end of the image.
        #[arg(long, value_parser = clap::value_parser!(u64).range(1..))]
        length: Option<u64>,
    },
    /// Compare exact validated media hashes in two previously acquired images.
    Compare { before: PathBuf, after: PathBuf },
    /// Hash the accessible files in a previously acquired logical Android export.
    Inventory { directory: PathBuf },
    /// Compare the paths and content hashes in two previously acquired exports.
    CompareExports { before: PathBuf, after: PathBuf },
    /// Preview a bounded repair of a damaged standalone media or Office file.
    Repair {
        file: PathBuf,
        /// Original SHA-256 from a trusted backup or manifest.
        #[arg(long)]
        expected_sha256: Option<String>,
    },
    /// Create an encrypted phone-transfer archive from a directory of existing files.
    BackupFiles {
        directory: PathBuf,
        archive: PathBuf,
    },
    /// Search filenames and categories under a selected local source.
    Catalog {
        source: PathBuf,
        #[arg(long, default_value = "")]
        query: String,
    },
    /// Hash an existing regular file and output JSON verification details.
    Verify { file: PathBuf },
    /// Serve bounded, read-only JSON-line requests under one local source.
    Agent { source: PathBuf },
}

#[derive(serde::Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
enum AgentRequest {
    Search {
        query: String,
        category: Option<omnirecovery::FileCategory>,
    },
    Verify {
        path: String,
    },
}

fn run_agent(source: &Path, mut output: impl Write) -> Result<(), Box<dyn std::error::Error>> {
    let mut input = io::stdin().lock();
    let mut line = Vec::new();
    for _ in 0..1024 {
        line.clear();
        if (&mut input).take(4097).read_until(b'\n', &mut line)? == 0 {
            break;
        }
        if line.len() > 4096 {
            writeln!(output, "{{\"ok\":false,\"error\":\"request_too_large\"}}")?;
            break;
        }
        let response = match serde_json::from_slice::<AgentRequest>(&line) {
            Ok(AgentRequest::Search { query, category }) => {
                omnirecovery::catalogue(source, category, &query)
                    .ok()
                    .and_then(|report| serde_json::to_value(report).ok())
            }
            Ok(AgentRequest::Verify { path }) => omnirecovery::catalogue_file(source, &path)
                .and_then(|file| omnirecovery::verify_file(&file))
                .ok()
                .and_then(|report| serde_json::to_value(report).ok()),
            Err(_) => None,
        };
        let response = response.map_or_else(
            || serde_json::json!({"ok":false,"error":"request_rejected"}),
            |report| serde_json::json!({"ok":true,"report":report}),
        );
        serde_json::to_writer(&mut output, &response)?;
        writeln!(output)?;
        output.flush()?;
    }
    Ok(())
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let command = Cli::parse().command;
    let mut output = io::stdout().lock();
    match command {
        Command::Scan {
            image,
            storage,
            media,
            offset,
            length,
        } => {
            let report = omnirecovery::scan_selected_media(
                &image,
                ScanOptions {
                    storage,
                    offset,
                    length,
                },
                media,
            )?;
            serde_json::to_writer_pretty(&mut output, &report)?;
        }
        Command::Compare { before, after } => {
            let report = omnirecovery::compare(&before, &after)?;
            serde_json::to_writer_pretty(&mut output, &report)?;
        }
        Command::Inventory { directory } => {
            let report = omnirecovery::inventory(&directory)?;
            serde_json::to_writer_pretty(&mut output, &report)?;
        }
        Command::CompareExports { before, after } => {
            let report = omnirecovery::compare_exports(&before, &after)?;
            serde_json::to_writer_pretty(&mut output, &report)?;
        }
        Command::Repair {
            file,
            expected_sha256,
        } => {
            let preview = omnirecovery::preview_repair(&file, expected_sha256.as_deref())?;
            serde_json::to_writer_pretty(&mut output, &preview.report)?;
        }
        Command::BackupFiles { directory, archive } => {
            let password = Zeroizing::new(rpassword::prompt_password("Backup passphrase: ")?);
            let confirmation = Zeroizing::new(rpassword::prompt_password("Repeat passphrase: ")?);
            if *password != *confirmation {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "passphrases do not match",
                )
                .into());
            }
            let report = omnirecovery::backup_files(&directory, &archive, &password)?;
            serde_json::to_writer_pretty(&mut output, &report)?;
        }
        Command::Catalog { source, query } => {
            serde_json::to_writer_pretty(
                &mut output,
                &omnirecovery::catalogue(&source, None, &query)?,
            )?;
        }
        Command::Verify { file } => {
            serde_json::to_writer_pretty(&mut output, &omnirecovery::verify_file(&file)?)?;
        }
        Command::Agent { source } => return run_agent(&source, output),
    }
    writeln!(output)?;
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("omnirecovery: {error}");
            ExitCode::FAILURE
        }
    }
}
