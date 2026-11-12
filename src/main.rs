use std::io::{self, BufRead, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use omnirecovery::{AndroidScope, MediaType, ScanOptions, StorageProfile, Transport};
use zeroize::Zeroizing;

#[derive(Parser)]
#[command(
    version,
    about = "Local data recovery, authorized acquisition and forensic inspection"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run an installed ALEAPP or iLEAPP parser against a verified export copy.
    AnalyzeLeapp {
        #[arg(value_enum)]
        engine: omnirecovery::LeappEngine,
        input: PathBuf,
        destination: PathBuf,
        #[arg(long, value_enum, default_value = "fs")]
        format: omnirecovery::LeappInput,
        #[arg(long)]
        checkout: PathBuf,
        #[arg(long)]
        python: PathBuf,
        #[arg(long)]
        script_sha256: String,
    },
    /// List deleted files using the installed Sleuth Kit fls against a raw image copy.
    TskDeleted {
        image: PathBuf,
        destination: PathBuf,
        #[arg(long, default_value_t = 0)]
        sector_offset: u64,
        #[arg(long, default_value_t = 512)]
        sector_size: u32,
    },
    /// Export a scanned finding using its original offset, length and recorded SHA-256 values.
    Recover {
        image: PathBuf,
        destination: PathBuf,
        #[arg(long, value_enum)]
        media: MediaType,
        #[arg(long)]
        offset: u64,
        #[arg(long)]
        length: u64,
        #[arg(long)]
        source_sha256: String,
        #[arg(long)]
        expected_sha256: String,
    },
    /// Pair an Android device using its displayed local Wireless debugging code.
    PairAndroid { address: String },
    /// Connect to the device's advertised Wireless debugging address.
    ConnectAndroid { address: String },
    /// List completed captures in a selected backup folder.
    BackupHistory { directory: PathBuf },
    /// Detect iOS recovery or DFU mode without changing the device.
    IosRecoveryStatus,
    /// Request Recovery Mode on an explicitly selected trusted iOS device.
    IosEnterRecovery {
        id: String,
        #[arg(long)]
        confirm_id: String,
    },
    /// Request normal boot for an explicitly selected iOS Recovery Mode target.
    IosExitRecovery {
        ecid: String,
        #[arg(long)]
        confirm_id: String,
    },
    /// List connected Android fastboot USB targets.
    FastbootDevices,
    /// Open a display-only local Android screen mirror.
    MirrorAndroid {
        id: String,
        #[arg(long, value_enum, default_value = "android")]
        transport: Transport,
        #[arg(long, value_enum, default_value = "balanced")]
        profile: omnirecovery::MirrorProfile,
        #[arg(long, value_enum, default_value = "screen")]
        video: omnirecovery::MirrorVideo,
        #[arg(long, value_enum, default_value = "off")]
        audio: omnirecovery::MirrorAudio,
    },
    /// Inspect reported Android specs and shared storage.
    AndroidDetails {
        id: String,
        #[arg(long, value_enum, default_value = "android")]
        transport: Transport,
    },
    /// Create an expiring, device-bound firmware or sanitization review plan.
    PlanAction {
        #[arg(value_enum)]
        action: omnirecovery::SensitiveAction,
        destination: PathBuf,
        #[arg(long)]
        case: String,
        #[arg(long)]
        model: String,
        #[arg(long)]
        target_id: String,
        #[arg(long, requires = "vendor_sha256")]
        firmware: Option<PathBuf>,
        #[arg(long, requires = "firmware")]
        vendor_sha256: Option<String>,
    },
    /// Verify two distinct signatures for an exact wipe or firmware plan.
    VerifyApproval {
        ledger: PathBuf,
        plan: PathBuf,
        #[arg(long)]
        trust: PathBuf,
        #[arg(long)]
        expected_head: String,
    },
    /// Reassemble recorded raw image ranges and verify the original file digest.
    Reconstruct {
        image: PathBuf,
        manifest: PathBuf,
        destination: PathBuf,
    },
    /// Read from explicitly authorized Google cloud services.
    Cloud {
        #[command(subcommand)]
        command: CloudCommand,
    },
    /// Ask a local model or explicitly authorized HTTPS provider about aggregate scan metrics.
    AiTriage {
        image: PathBuf,
        #[arg(long)]
        model: String,
        #[arg(long, requires = "allow_remote")]
        remote_url: Option<String>,
        #[arg(long, requires = "remote_url")]
        allow_remote: bool,
    },
    /// Read structured records from a supported local export.
    Artifacts {
        source: PathBuf,
        #[arg(value_enum)]
        kind: omnirecovery::ArtifactKind,
        #[arg(long, default_value_t = 100)]
        limit: usize,
    },
    /// Create an operator signing key and print its public key.
    CustodyKeygen { private_key: PathBuf },
    /// Sign and append a case event for an artifact.
    CustodyAppend {
        ledger: PathBuf,
        artifact: PathBuf,
        #[arg(long)]
        trust: PathBuf,
        #[arg(long)]
        private_key: PathBuf,
        #[arg(long)]
        actor: String,
        #[arg(long)]
        case: String,
        #[arg(long, value_enum)]
        action: omnirecovery::CustodyAction,
    },
    /// Verify every custody signature against a separately retained head hash.
    CustodyVerify {
        ledger: PathBuf,
        #[arg(long)]
        trust: PathBuf,
        #[arg(long)]
        expected_head: String,
    },
    /// List deleted FAT32 short entries whose contiguous cluster spans are free.
    UndeleteFat32 {
        image: PathBuf,
        #[arg(long, default_value_t = 0)]
        volume_offset: u64,
    },
    /// Export one candidate from the exact FAT32 image examined earlier.
    RecoverFat32 {
        image: PathBuf,
        entry_offset: u64,
        destination: PathBuf,
        #[arg(long, default_value_t = 0)]
        volume_offset: u64,
        #[arg(long)]
        source_sha256: String,
        #[arg(long)]
        expected_sha256: Option<String>,
    },
    /// List devices exposed by a local USB transport helper.
    Devices { transport: Transport },
    /// Collect an authorized USB logical export into an existing destination directory.
    Acquire {
        transport: Transport,
        id: String,
        destination: PathBuf,
        #[arg(long, value_enum, default_value = "shared")]
        android_scope: AndroidScope,
    },
    /// Copy and verify an existing image into a new capture directory.
    CloneImage {
        image: PathBuf,
        destination: PathBuf,
    },
    /// Read an exclusively opened, already read-only whole Linux block device.
    #[cfg(target_os = "linux")]
    CloneDisk {
        device: PathBuf,
        destination: PathBuf,
    },
    /// Recheck every data file against an acquisition manifest.
    VerifyAcquisition { directory: PathBuf },
    /// Restore a password-protected OmniRecovery file archive to a new directory.
    RestoreFiles {
        archive: PathBuf,
        destination: PathBuf,
    },
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
        /// Save a validated derivative into a new file.
        #[arg(long)]
        output: Option<PathBuf>,
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

#[derive(Subcommand)]
enum CloudCommand {
    /// List accessible items in Google Drive Trash.
    DriveTrash {
        #[arg(long)]
        page_token: Option<String>,
    },
    /// Download a Drive blob to a new local file.
    DriveDownload { id: String, destination: PathBuf },
    /// Create a Google Photos Picker session and print its selection URL.
    PhotosSession,
    /// List photos and videos selected in a Picker session.
    PhotosList {
        session: String,
        #[arg(long)]
        page_token: Option<String>,
    },
    /// Download one explicitly selected Photos item.
    PhotosDownload {
        session: String,
        id: String,
        destination: PathBuf,
        #[arg(long)]
        page_token: Option<String>,
    },
}

#[derive(serde::Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
enum AgentRequest {
    Scan {
        #[serde(default)]
        offset: u64,
        length: u64,
        media: Option<MediaType>,
    },
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
    let mut scan_used = false;
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
            Ok(AgentRequest::Scan {
                offset,
                length,
                media,
            }) if !scan_used && (1..=32 * 1024 * 1024).contains(&length) => {
                scan_used = true;
                omnirecovery::scan_selected_media(
                    source,
                    ScanOptions {
                        offset,
                        length: Some(length),
                        storage: StorageProfile::Auto,
                    },
                    media,
                )
                .ok()
                .map(|mut report| {
                    let findings_returned = report.findings.len().min(200);
                    let findings_total = report.findings.len();
                    report.findings.truncate(findings_returned);
                    serde_json::json!({
                        "source_sha256": report.source_sha256,
                        "range": report.selected_range,
                        "observed_ranges_fully_scanned": report.observed_ranges_fully_scanned,
                        "findings_total": findings_total,
                        "findings": report.findings,
                        "triage": report.triage,
                    })
                })
            }
            _ => None,
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
        Command::AnalyzeLeapp {
            engine,
            input,
            destination,
            format,
            checkout,
            python,
            script_sha256,
        } => {
            let report = omnirecovery::analyze_leapp(
                engine,
                format,
                &input,
                &checkout,
                &python,
                &script_sha256,
                &destination,
            )?;
            serde_json::to_writer_pretty(&mut output, &report)?;
        }
        Command::TskDeleted {
            image,
            destination,
            sector_offset,
            sector_size,
        } => {
            let report =
                omnirecovery::tsk_deleted(&image, &destination, sector_offset, sector_size)?;
            serde_json::to_writer_pretty(&mut output, &report)?;
        }
        Command::Recover {
            image,
            destination,
            media,
            offset,
            length,
            source_sha256,
            expected_sha256,
        } => {
            let finding = omnirecovery::export_recovery(
                &image,
                &source_sha256,
                media,
                offset,
                length,
                &expected_sha256,
                &destination,
            )?;
            serde_json::to_writer_pretty(&mut output, &finding)?;
        }
        Command::PairAndroid { address } => {
            let code = Zeroizing::new(rpassword::prompt_password("Android pairing code: ")?);
            omnirecovery::pair_android_wifi(&address, &code)?;
            serde_json::to_writer_pretty(&mut output, &serde_json::json!({"status": "paired"}))?;
        }
        Command::ConnectAndroid { address } => {
            omnirecovery::connect_android_wifi(&address)?;
            serde_json::to_writer_pretty(&mut output, &serde_json::json!({"status": "connected"}))?;
        }
        Command::BackupHistory { directory } => {
            serde_json::to_writer_pretty(&mut output, &omnirecovery::capture_history(&directory)?)?
        }
        Command::IosRecoveryStatus => {
            serde_json::to_writer_pretty(&mut output, &omnirecovery::ios_recovery_status()?)?
        }
        Command::IosEnterRecovery { id, confirm_id } => {
            if id != confirm_id {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "confirm-id must match the selected device",
                )
                .into());
            }
            omnirecovery::ios_enter_recovery(&id)?;
            serde_json::to_writer_pretty(
                &mut output,
                &serde_json::json!({"status": "recovery_transition_requested"}),
            )?;
        }
        Command::IosExitRecovery { ecid, confirm_id } => {
            if ecid != confirm_id {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "confirm-id must match the selected ECID",
                )
                .into());
            }
            omnirecovery::ios_exit_recovery(&ecid)?;
            serde_json::to_writer_pretty(
                &mut output,
                &serde_json::json!({"status": "normal_boot_requested"}),
            )?;
        }
        Command::FastbootDevices => {
            serde_json::to_writer_pretty(&mut output, &omnirecovery::android_fastboot_devices()?)?
        }
        Command::AndroidDetails { id, transport } => {
            let details = omnirecovery::inspect_android_details(transport, &id)?;
            serde_json::to_writer_pretty(&mut output, &details)?;
        }
        Command::MirrorAndroid {
            id,
            transport,
            profile,
            video,
            audio,
        } => {
            let options = omnirecovery::MirrorOptions {
                profile,
                video,
                audio,
            };
            if !omnirecovery::start_android_preview(transport, &id, options)?
                .wait()?
                .success()
            {
                return Err(io::Error::other("the scrcpy window exited with an error").into());
            }
            serde_json::to_writer_pretty(
                &mut output,
                &serde_json::json!({"status": "mirror_closed"}),
            )?;
        }
        Command::PlanAction {
            action,
            destination,
            case,
            model,
            target_id,
            firmware,
            vendor_sha256,
        } => {
            omnirecovery::make_action_plan(
                action,
                &case,
                &model,
                &target_id,
                firmware.as_deref().zip(vendor_sha256.as_deref()),
                &destination,
            )?;
            serde_json::to_writer_pretty(
                &mut output,
                &serde_json::json!({"plan": destination, "status": "awaiting_signed_request_and_approval"}),
            )?;
        }
        Command::VerifyApproval {
            ledger,
            plan,
            trust,
            expected_head,
        } => {
            let report =
                omnirecovery::verify_action_approval(&ledger, &trust, &expected_head, &plan)?;
            serde_json::to_writer_pretty(&mut output, &report)?;
        }
        Command::Reconstruct {
            image,
            manifest,
            destination,
        } => {
            let report = omnirecovery::reconstruct_file(&image, &manifest, &destination)?;
            serde_json::to_writer_pretty(&mut output, &report)?;
        }
        Command::Cloud { command } => {
            let token = Zeroizing::new(rpassword::prompt_password("Google OAuth access token: ")?);
            match command {
                CloudCommand::DriveTrash { page_token } => serde_json::to_writer_pretty(
                    &mut output,
                    &omnirecovery::google_drive_trash(&token, page_token.as_deref())?,
                )?,
                CloudCommand::DriveDownload { id, destination } => serde_json::to_writer_pretty(
                    &mut output,
                    &omnirecovery::google_drive_download(&token, &id, &destination)?,
                )?,
                CloudCommand::PhotosSession => serde_json::to_writer_pretty(
                    &mut output,
                    &omnirecovery::google_photos_session(&token)?,
                )?,
                CloudCommand::PhotosList {
                    session,
                    page_token,
                } => serde_json::to_writer_pretty(
                    &mut output,
                    &omnirecovery::google_photos_list(&token, &session, page_token.as_deref())?,
                )?,
                CloudCommand::PhotosDownload {
                    session,
                    id,
                    destination,
                    page_token,
                } => serde_json::to_writer_pretty(
                    &mut output,
                    &omnirecovery::google_photos_download(
                        &token,
                        &session,
                        &id,
                        page_token.as_deref(),
                        &destination,
                    )?,
                )?,
            }
        }
        Command::AiTriage {
            image,
            model,
            remote_url,
            allow_remote: _,
        } => {
            let scan = omnirecovery::scan(&image)?;
            let report = omnirecovery::ai_triage(&scan, &model, remote_url.as_deref())?;
            serde_json::to_writer_pretty(&mut output, &report)?;
        }
        Command::Artifacts {
            source,
            kind,
            limit,
        } => {
            let report = omnirecovery::inspect_artifact(&source, kind, limit)?;
            serde_json::to_writer_pretty(&mut output, &report)?;
        }
        Command::CustodyKeygen { private_key } => {
            let public_key = omnirecovery::generate_custody_key(&private_key)?;
            serde_json::to_writer_pretty(
                &mut output,
                &serde_json::json!({ "public_key": public_key }),
            )?;
        }
        Command::CustodyAppend {
            ledger,
            artifact,
            trust,
            private_key,
            actor,
            case,
            action,
        } => {
            let head = omnirecovery::append_custody(
                &ledger,
                &trust,
                &private_key,
                &actor,
                &case,
                action,
                &artifact,
            )?;
            serde_json::to_writer_pretty(&mut output, &head)?;
        }
        Command::CustodyVerify {
            ledger,
            trust,
            expected_head,
        } => {
            let head = omnirecovery::verify_custody(&ledger, &trust, &expected_head)?;
            serde_json::to_writer_pretty(&mut output, &head)?;
        }
        Command::UndeleteFat32 {
            image,
            volume_offset,
        } => {
            let report = omnirecovery::scan_fat32_deleted(&image, volume_offset)?;
            serde_json::to_writer_pretty(&mut output, &report)?;
        }
        Command::RecoverFat32 {
            image,
            entry_offset,
            destination,
            volume_offset,
            source_sha256,
            expected_sha256,
        } => {
            let report = omnirecovery::recover_fat32_deleted(
                &image,
                volume_offset,
                entry_offset,
                &source_sha256,
                expected_sha256.as_deref(),
                &destination,
            )?;
            serde_json::to_writer_pretty(&mut output, &report)?;
        }
        Command::Devices { transport } => {
            serde_json::to_writer_pretty(&mut output, &omnirecovery::detect_devices(transport)?)?;
        }
        Command::Acquire {
            transport,
            id,
            destination,
            android_scope,
        } => {
            let report = omnirecovery::acquire_device(transport, &id, android_scope, &destination)?;
            serde_json::to_writer_pretty(&mut output, &report)?;
        }
        Command::CloneImage { image, destination } => {
            let report = omnirecovery::acquire_image(&image, &destination)?;
            serde_json::to_writer_pretty(&mut output, &report)?;
        }
        #[cfg(target_os = "linux")]
        Command::CloneDisk {
            device,
            destination,
        } => {
            let report = omnirecovery::acquire_disk(&device, &destination)?;
            serde_json::to_writer_pretty(&mut output, &report)?;
        }
        Command::VerifyAcquisition { directory } => {
            let report = omnirecovery::verify_acquisition(&directory)?;
            serde_json::to_writer_pretty(&mut output, &report)?;
        }
        Command::RestoreFiles {
            archive,
            destination,
        } => {
            let password = Zeroizing::new(rpassword::prompt_password("Backup passphrase: ")?);
            let report = omnirecovery::restore_files(&archive, &destination, &password)?;
            serde_json::to_writer_pretty(&mut output, &report)?;
        }
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
            output: destination,
        } => {
            let report = if let Some(destination) = destination {
                omnirecovery::export_repair(&file, expected_sha256.as_deref(), &destination)?
            } else {
                omnirecovery::preview_repair(&file, expected_sha256.as_deref())?.report
            };
            serde_json::to_writer_pretty(&mut output, &report)?;
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
