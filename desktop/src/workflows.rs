use super::*;
use omnirecovery::FileCategory;
use serde_json::json;
use std::time::Instant;

pub(super) fn tracked(
    app: &AppHandle,
    operation: &str,
    run: impl FnOnce() -> Result<Value, String>,
) -> Result<Value, String> {
    let started = records::now();
    let mut result = run();
    if records::record(app, operation, started, &result).is_err() {
        match &mut result {
            Ok(value) => value["logging_warning"] = json!("Could not save the local activity log."),
            Err(message) => message.push_str(" The activity log could not be saved."),
        }
    }
    result
}

fn report(state: &Shared, kind: ReportKind, value: Value) -> Result<Value, String> {
    lock(state)?.reports.insert(kind, Arc::new(value.clone()));
    Ok(value)
}

#[tauri::command]
pub(super) async fn discover_devices(state: State<'_, Shared>) -> Result<devices::Devices, String> {
    let state = state.inner().clone();
    work(move || {
        let devices = devices::discover()?;
        lock(&state)?.disks.clone_from(&devices.disks);
        Ok(devices)
    })
    .await
}

#[tauri::command]
pub(super) async fn choose_volume(
    state: State<'_, Shared>,
    id: String,
) -> Result<Selection, String> {
    let before = devices::volume(&lock(&state)?.disks, &id)
        .cloned()
        .filter(|volume| volume.selectable)
        .ok_or("Select a mounted volume from Devices & storage.")?;
    let state = state.inner().clone();
    work(move || {
        let after = devices::recheck_volume(&id)
            .filter(|volume| {
                volume.selectable && volume.uuid == before.uuid && volume.mount == before.mount
            })
            .ok_or("The selected volume changed. Refresh Devices & storage and choose it again.")?;
        let selection = set_source(
            &state,
            Source::Volume,
            after.mount.clone().ok_or("Volume is not mounted.")?,
        )?;
        lock(&state)?.active_volume = Some(after);
        Ok(selection)
    })
    .await
}

#[tauri::command]
pub(super) async fn discover_phones() -> Result<devices::PhoneInventory, String> {
    work(|| Ok(devices::discover_phones())).await
}

fn verify_mounted(state: &Shared, slot: Source, source: &Path) -> Result<(), String> {
    if slot != Source::Volume {
        return Ok(());
    }
    let volume = lock(state)?
        .active_volume
        .clone()
        .ok_or("Choose the mounted volume from Devices & storage.")?;
    if volume.mount.as_deref() != Some(source)
        || !devices::recheck_volume(&volume.id).is_some_and(|current| {
            current.selectable && current.uuid == volume.uuid && current.mount == volume.mount
        })
    {
        return Err(
            "The selected volume changed. Refresh Devices & storage before continuing.".into(),
        );
    }
    Ok(())
}

fn source_kind(slot: Source) -> Result<&'static str, String> {
    match slot {
        Source::Folder => Ok("selected_folder"),
        Source::File => Ok("selected_file"),
        Source::Volume => Ok("active_mounted_volume"),
        Source::AndroidExport | Source::Transfer => Ok("android_export_folder"),
        Source::IosExport => Ok("ios_export_folder"),
        _ => Err("Choose a media or import source.".into()),
    }
}

#[tauri::command]
pub(super) async fn scan_catalogue(
    app: AppHandle,
    state: State<'_, Shared>,
    slot: Source,
    categories: Vec<FileCategory>,
    query: String,
) -> Result<Value, String> {
    let kind = source_kind(slot)?;
    let source = selected(&state, slot)?;
    let state = state.inner().clone();
    work(move || {
        tracked(&app, "media_catalogue", || {
            verify_mounted(&state, slot, &source)?;
            let mut result = omnirecovery::catalogue(&source, None, &query).map_err(error)?;
            if !categories.is_empty() {
                result
                    .entries
                    .retain(|entry| categories.contains(&entry.category));
            }
            let mut value = serde_json::to_value(&result).map_err(error)?;
            value["source_kind"] = json!(kind);
            value["classification"] = json!("filename_based; currently listed files");
            let mut workspace = lock(&state)?;
            workspace.catalogues.insert(
                slot,
                Arc::new(Catalogued {
                    source,
                    report: result,
                }),
            );
            if slot != Source::Transfer {
                workspace
                    .reports
                    .insert(ReportKind::Catalogue, Arc::new(value.clone()));
            }
            Ok(value)
        })
    })
    .await
}

pub(super) fn selected_paths(
    state: &Shared,
    slot: Source,
    indices: &[usize],
) -> Result<Vec<String>, String> {
    let workspace = lock(state)?;
    let job = workspace
        .catalogues
        .get(&slot)
        .ok_or("Scan the selected folder first.")?;
    indices
        .iter()
        .map(|index| {
            job.report
                .entries
                .get(*index)
                .map(|entry| entry.path.clone())
                .ok_or_else(|| "Invalid catalogue selection.".into())
        })
        .collect()
}

fn entry(
    state: &Shared,
    slot: Source,
    index: usize,
) -> Result<(Arc<Catalogued>, String, u64), String> {
    let workspace = lock(state)?;
    let job = workspace
        .catalogues
        .get(&slot)
        .cloned()
        .ok_or("Scan the source first.")?;
    let selected = job
        .report
        .entries
        .get(index)
        .ok_or("Choose an existing catalogue entry.")?;
    Ok((job.clone(), selected.path.clone(), selected.length))
}

#[tauri::command]
pub(super) async fn inspect_catalogue_entry(
    state: State<'_, Shared>,
    slot: Source,
    index: usize,
) -> Result<Value, String> {
    let (job, relative, length) = entry(&state, slot, index)?;
    let state = state.inner().clone();
    work(move || {
        verify_mounted(&state, slot, &job.source)?;
        let path = omnirecovery::catalogue_file(&job.source, &relative).map_err(error)?;
        let file = omnirecovery::verify_file(&path).map_err(error)?;
        if file.length != length {
            return Err("The file length changed. Scan the source again.".into());
        }
        let suffix = path
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let preview =
            if matches!(suffix.as_str(), "jpg" | "jpeg" | "png") && length <= 16 * 1024 * 1024 {
                let report = omnirecovery::scan(&path).map_err(error)?;
                if report.source_sha256 != file.sha256 {
                    return Err("The file changed while preparing its preview.".into());
                }
                if let Some(finding) = report
                    .findings
                    .iter()
                    .find(|finding| finding.offset == 0 && finding.length == length)
                {
                    let bytes = omnirecovery::recover_candidate(&path, &file.sha256, finding)
                        .map_err(error)?;
                    let mime = if finding.media_type == MediaType::Png {
                        "image/png"
                    } else {
                        "image/jpeg"
                    };
                    Some(format!("data:{mime};base64,{}", STANDARD.encode(bytes)))
                } else {
                    None
                }
            } else {
                None
            };
        Ok(json!({"sha256":file.sha256,"bytes":file.length,"preview":preview}))
    })
    .await
}

#[tauri::command]
pub(super) async fn copy_catalogue_entry(
    app: AppHandle,
    state: State<'_, Shared>,
    slot: Source,
    index: usize,
) -> Result<Option<Value>, String> {
    let (job, relative, length) = entry(&state, slot, index)?;
    let state = state.inner().clone();
    work(move || {
        verify_mounted(&state, slot, &job.source)?;
        let source = omnirecovery::catalogue_file(&job.source, &relative).map_err(error)?;
        if source.symlink_metadata().map_err(error)?.len() != length {
            return Err("The file length changed. Scan the source again.".into());
        }
        copy_path(&app, &source, &label(&source))
    })
    .await
}

fn copy_path(app: &AppHandle, source: &Path, filename: &str) -> Result<Option<Value>, String> {
    let path = app
        .dialog()
        .file()
        .set_title("Save verified copy to a different drive")
        .set_file_name(filename)
        .blocking_save_file();
    let Some(path) = picked(path)? else {
        return Ok(None);
    };
    let started = Instant::now();
    let (sha256, bytes) = destination::copy_file(source, &path)?;
    Ok(Some(
        json!({"name":label(&path),"sha256":sha256,"bytes":bytes,"elapsed_ms":started.elapsed().as_millis() as u64}),
    ))
}

#[tauri::command]
pub(super) async fn clone_image(
    app: AppHandle,
    state: State<'_, Shared>,
) -> Result<Option<Value>, String> {
    let source = selected(&state, Source::CaptureImage)?;
    work(move || copy_path(&app, &source, "image-clone.dd")).await
}

#[tauri::command]
pub(super) async fn take_capture(
    app: AppHandle,
    state: State<'_, Shared>,
    stage: records::Stage,
    folder: bool,
) -> Result<Value, String> {
    let source = selected(
        &state,
        if folder {
            Source::CaptureFolder
        } else {
            Source::CaptureImage
        },
    )?;
    work(move || {
        tracked(&app, "capture", || {
            records::capture(&app, &source, stage, folder)
        })
    })
    .await
}

#[tauri::command]
pub(super) async fn capture_status(app: AppHandle) -> Result<Value, String> {
    work(move || records::capture_summary(&app)).await
}

#[tauri::command]
pub(super) async fn compare_saved(
    app: AppHandle,
    state: State<'_, Shared>,
) -> Result<Value, String> {
    let state = state.inner().clone();
    work(move || {
        report(
            &state,
            ReportKind::Compare,
            records::compare_captures(&app)?,
        )
    })
    .await
}

#[tauri::command]
pub(super) async fn scan_evidence(
    app: AppHandle,
    state: State<'_, Shared>,
    case_reference: String,
    offset: u64,
    length: Option<u64>,
    media: Option<MediaType>,
) -> Result<Value, String> {
    validate_case_reference(&case_reference)?;
    let source = selected(&state, Source::Evidence)?;
    let state = state.inner().clone();
    work(move || {
        tracked(&app, "forensic_image_scan", || {
            let started = Instant::now();
            let scan = omnirecovery::scan_selected_media(
                &source,
                ScanOptions {
                    offset,
                    length,
                    ..ScanOptions::default()
                },
                media,
            )
            .map_err(error)?;
            let mut value = serde_json::to_value(scan).map_err(error)?;
            value["kind"] = json!("forensic_image_scan");
            value["case_reference"] = json!(case_reference);
            value["elapsed_ms"] = json!(started.elapsed().as_millis() as u64);
            value["signed"] = json!(false);
            report(&state, ReportKind::ForensicScan, value)
        })
    })
    .await
}

#[tauri::command]
pub(super) async fn clone_evidence(
    app: AppHandle,
    state: State<'_, Shared>,
    case_reference: String,
) -> Result<Value, String> {
    validate_case_reference(&case_reference)?;
    let source = selected(&state, Source::Evidence)?;
    let state = state.inner().clone();
    work(move || {
        tracked(&app, "forensic_image_clone", || {
            let Some(mut value) = copy_path(&app, &source, "forensic-clone.dd")? else {
                return Ok(json!({"status":"cancelled","case_reference":case_reference}));
            };
            value["kind"] = json!("forensic_image_clone");
            value["case_reference"] = json!(case_reference);
            value["readback_verified"] = json!(true);
            value["signed"] = json!(false);
            report(&state, ReportKind::EvidenceClone, value)
        })
    })
    .await
}

fn validate_case_reference(case_reference: &str) -> Result<(), String> {
    if case_reference.len() > 128 || case_reference.chars().any(char::is_control) {
        return Err("Use a case reference of at most 128 bytes without control characters.".into());
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum PhoneAnalysis {
    Android,
    IosFilesystem,
    IosBackup,
}

#[tauri::command]
pub(super) async fn analyze_phone_artifacts(
    app: AppHandle,
    state: State<'_, Shared>,
    mode: PhoneAnalysis,
    script_sha256: String,
) -> Result<Value, String> {
    use omnirecovery::{LeappEngine, LeappInput};
    if script_sha256.len() != 64 || !script_sha256.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("Enter the reviewed parser entrypoint’s SHA-256.".into());
    }
    let (engine, format) = match mode {
        PhoneAnalysis::Android => (LeappEngine::Aleapp, LeappInput::Fs),
        PhoneAnalysis::IosFilesystem => (LeappEngine::Ileapp, LeappInput::Fs),
        PhoneAnalysis::IosBackup => (LeappEngine::Ileapp, LeappInput::Itunes),
    };
    let export = selected(&state, Source::LeappExport)?;
    let checkout = selected(&state, Source::LeappCheckout)?;
    let python = selected(&state, Source::LeappPython)?;
    work(move || {
        let Some(destination) = forensic_report_destination(&app)? else {
            return Ok(Value::Null);
        };
        tracked(&app, "external_phone_artifact_analysis", || {
            let report = omnirecovery::analyze_leapp(
                engine,
                format,
                &export,
                &checkout,
                &python,
                &script_sha256,
                &destination,
            )
            .map_err(error)?;
            Ok(forensic_adapter_summary(report))
        })
    })
    .await
}

#[tauri::command]
pub(super) async fn list_deleted_evidence(
    app: AppHandle,
    state: State<'_, Shared>,
    sector_offset: u64,
    sector_size: u32,
) -> Result<Value, String> {
    let image = selected(&state, Source::Evidence)?;
    work(move || {
        let Some(destination) = forensic_report_destination(&app)? else {
            return Ok(Value::Null);
        };
        tracked(&app, "sleuth_kit_deleted_entries", || {
            let report =
                omnirecovery::tsk_deleted(&image, &destination, sector_offset, sector_size)
                    .map_err(error)?;
            let path = report.output.join("data/tsk-deleted.txt");
            let mut bytes = Vec::new();
            use std::io::Read;
            let preview = std::fs::File::open(path)
                .and_then(|file| file.take(32 * 1024).read_to_end(&mut bytes));
            let mut summary = forensic_adapter_summary(report);
            if preview.is_ok() {
                summary["preview"] = json!(String::from_utf8_lossy(&bytes));
            }
            Ok(summary)
        })
    })
    .await
}

fn forensic_report_destination(app: &AppHandle) -> Result<Option<PathBuf>, String> {
    picked(
        app.dialog()
            .file()
            .set_title("Choose a folder for the forensic report")
            .blocking_pick_folder(),
    )
}

fn forensic_adapter_summary(report: omnirecovery::AcquisitionReport) -> Value {
    json!({"output": report.output, "files": report.files.files.len()})
}

#[tauri::command]
pub(super) async fn verify_evidence(
    app: AppHandle,
    state: State<'_, Shared>,
    expected: String,
    case_reference: String,
) -> Result<Value, String> {
    if !expected.is_empty()
        && (expected.len() != 64 || !expected.bytes().all(|byte| byte.is_ascii_hexdigit()))
    {
        return Err("Expected SHA-256 must contain 64 hexadecimal characters.".into());
    }
    validate_case_reference(&case_reference)?;
    let source = selected(&state, Source::Evidence)?;
    let state = state.inner().clone();
    work(move || tracked(&app, "evidence_verification", || {
        let first = omnirecovery::verify_file(&source).map_err(error)?;
        let second = omnirecovery::verify_file(&source).map_err(error)?;
        if first != second { return Err("The source changed between verification passes.".into()); }
        let reference = (!expected.is_empty()).then(|| expected.to_ascii_lowercase());
        let matches = reference.as_ref().map(|value| value == &first.sha256);
        report(&state, ReportKind::Evidence, json!({"schema_version":1,"kind":"file_verification","case_reference":case_reference,"source":label(&source),"bytes":first.length,"sha256":first.sha256,"reference":reference,"matches_reference":matches,"verified_passes":2,"verified_at_ms":records::now(),"signed":false}))
    })).await
}

#[tauri::command]
pub(super) async fn security_scan(
    app: AppHandle,
    state: State<'_, Shared>,
    log: bool,
) -> Result<Value, String> {
    let source = selected(&state, Source::Security)?;
    let state = state.inner().clone();
    work(move || {
        tracked(
            &app,
            if log {
                "security_log_triage"
            } else {
                "malware_sample_scan"
            },
            || {
                let value =
                    serde_json::to_value(security::analyze(&source, log)?).map_err(error)?;
                report(&state, ReportKind::Security, value)
            },
        )
    })
    .await
}

#[tauri::command]
pub(super) async fn activity_logs(app: AppHandle, trash: bool) -> Result<Vec<Value>, String> {
    work(move || records::logs(&app, trash)).await
}

#[tauri::command]
pub(super) async fn move_activity_log(
    app: AppHandle,
    id: String,
    from_trash: bool,
) -> Result<(), String> {
    work(move || records::move_log(&app, &id, from_trash)).await
}

#[tauri::command]
pub(super) async fn export_activity_log(
    app: AppHandle,
    id: String,
    trash: bool,
) -> Result<Option<String>, String> {
    work(move || {
        let record = records::log(&app, &id, trash)?;
        let path = app
            .dialog()
            .file()
            .set_title("Export scan activity")
            .set_file_name("omnirecovery-activity.json")
            .add_filter("JSON", &["json"])
            .blocking_save_file();
        let Some(path) = picked(path)? else {
            return Ok(None);
        };
        destination::save_report(&path, &serde_json::to_vec_pretty(&record).map_err(error)?)
            .map_err(error)?;
        Ok(Some(label(&path)))
    })
    .await
}

#[tauri::command]
pub(super) async fn sanitize_plan(
    state: State<'_, Shared>,
    id: String,
    goal: String,
) -> Result<Value, String> {
    if !matches!(goal.as_str(), "clear" | "purge" | "destroy") {
        return Err("Choose a valid sanitization goal.".into());
    }
    let state = state.inner().clone();
    work(move || {
        let disk = devices::discover()?.disks.into_iter().find(|disk| disk.id == id && disk.physical && !disk.internal).ok_or("Choose a currently attached external physical disk.")?;
        let route = match (goal.as_str(), disk.solid_state) {
            ("destroy", _) => "Use an approved physical destruction service and record its verification.",
            ("clear", Some(false)) => "Qualify a supported overwrite method and independently verify accessible blocks.",
            ("purge", _) | ("clear", Some(true)) => "Qualify the manufacturer’s device-specific sanitize method and validate controller completion.",
            _ => "Confirm media type and controller capabilities before selecting a method.",
        };
        report(&state, ReportKind::Sanitize, json!({"schema_version":1,"status":"planning_only","device_id":disk.id,"model":disk.name,"size":disk.size,"goal":goal,"method_guidance":route,"standard":"NIST SP 800-88 Rev. 2","verification":"SHA-256 may document readable output for a qualified overwrite. Controller verification applies to purge methods.","approval":"Confirm target identity and evidence-free status. Obtain two authorized approvals before execution."}))
    }).await
}

#[tauri::command]
pub(super) async fn list_acquisition_devices(
    transport: omnirecovery::Transport,
) -> Result<Vec<omnirecovery::DetectedDevice>, String> {
    work(move || omnirecovery::detect_devices(transport).map_err(error)).await
}

#[tauri::command]
pub(super) async fn capture_usb_device(
    app: AppHandle,
    startup: State<'_, super::tool_window::Startup>,
    transport: omnirecovery::Transport,
    id: String,
    scope: omnirecovery::AndroidScope,
) -> Result<Option<Value>, String> {
    startup.check_phone(transport, &id)?;
    work(move || {
        let folder = app
            .dialog()
            .file()
            .set_title("Choose a private destination for the USB capture")
            .blocking_pick_folder();
        let Some(destination) = picked(folder)? else {
            return Ok(None);
        };
        let result =
            omnirecovery::acquire_device(transport, &id, scope, &destination).map_err(error)?;
        Ok(Some(json!({
            "output": result.output,
            "files": result.files.files.len(),
            "bytes": result.bytes,
        })))
    })
    .await
}

#[tauri::command]
pub(super) async fn backup_history(
    state: State<'_, Shared>,
) -> Result<omnirecovery::CaptureHistory, String> {
    let parent = selected(&state, Source::BackupHistory)?;
    work(move || omnirecovery::capture_history(&parent).map_err(error)).await
}

#[tauri::command]
pub(super) async fn verify_history_item(
    state: State<'_, Shared>,
    name: String,
) -> Result<usize, String> {
    let parent = selected(&state, Source::BackupHistory)?;
    work(move || {
        Ok(omnirecovery::verify_capture_in_history(&parent, &name)
            .map_err(error)?
            .files
            .len())
    })
    .await
}

#[tauri::command]
pub(super) async fn fastboot_devices() -> Result<Vec<omnirecovery::DetectedDevice>, String> {
    work(move || omnirecovery::android_fastboot_devices().map_err(error)).await
}

#[tauri::command]
pub(super) async fn ios_recovery_device() -> Result<omnirecovery::IosRecoveryDevice, String> {
    work(move || omnirecovery::ios_recovery_status().map_err(error)).await
}

#[tauri::command]
pub(super) async fn change_ios_recovery(id: String, enter: bool) -> Result<(), String> {
    work(move || {
        if enter {
            omnirecovery::ios_enter_recovery(&id)
        } else {
            omnirecovery::ios_exit_recovery(&id)
        }
        .map_err(error)
    })
    .await
}

#[derive(Default)]
pub(super) struct MirrorSession(Arc<Mutex<Option<std::process::Child>>>);

impl MirrorSession {
    pub(super) fn stop_on_exit(&self) {
        if let Ok(mut child) = self.0.lock()
            && let Some(mut process) = child.take()
        {
            let _ = process.kill();
            let _ = process.wait();
        }
    }
}

#[tauri::command]
pub(super) async fn android_details(
    id: String,
    transport: omnirecovery::Transport,
    startup: State<'_, Startup>,
) -> Result<omnirecovery::AndroidDetails, String> {
    startup.check_phone(transport, &id)?;
    work(move || omnirecovery::inspect_android_details(transport, &id).map_err(error)).await
}

fn mirror_running(child: &mut Option<std::process::Child>) -> Result<bool, String> {
    let Some(process) = child.as_mut() else {
        return Ok(false);
    };
    if process.try_wait().map_err(error)?.is_some() {
        *child = None;
        return Ok(false);
    }
    Ok(true)
}

#[tauri::command]
pub(super) fn mirror_active(state: State<'_, MirrorSession>) -> Result<bool, String> {
    let mut child = state.0.lock().map_err(error)?;
    mirror_running(&mut child)
}

#[tauri::command]
pub(super) async fn stop_mirror(state: State<'_, MirrorSession>) -> Result<(), String> {
    let session = state.0.clone();
    work(move || {
        let mut current = session.lock().map_err(error)?;
        if mirror_running(&mut current)?
            && let Some(mut child) = current.take()
        {
            let _ = child.kill();
            child.wait().map_err(error)?;
        }
        Ok(())
    })
    .await
}

#[tauri::command]
pub(super) async fn mirror_android(
    state: State<'_, MirrorSession>,
    id: String,
    transport: omnirecovery::Transport,
    options: omnirecovery::MirrorOptions,
) -> Result<(), String> {
    let session = state.0.clone();
    work(move || {
        let mut current = session.lock().map_err(error)?;
        if mirror_running(&mut current)? {
            return Err("Stop the active preview before starting another in this window.".into());
        }
        let mut child = omnirecovery::start_android_preview(transport, &id, options).map_err(error)?;
        std::thread::sleep(std::time::Duration::from_millis(650));
        if child.try_wait().map_err(error)?.is_some() {
            return Err("scrcpy closed before the preview started. Check the installed version and device connection.".into());
        }
        *current = Some(child);
        Ok(())
    }).await
}

#[tauri::command]
pub(super) async fn pair_wireless_android(address: String, code: String) -> Result<(), String> {
    let code = Zeroizing::new(code);
    work(move || omnirecovery::pair_android_wifi(&address, &code).map_err(error)).await
}

#[tauri::command]
pub(super) async fn connect_wireless_android(address: String) -> Result<(), String> {
    work(move || omnirecovery::connect_android_wifi(&address).map_err(error)).await
}

#[tauri::command]
pub(super) async fn inspect_export_records(
    state: State<'_, Shared>,
    kind: omnirecovery::ArtifactKind,
) -> Result<omnirecovery::ArtifactReport, String> {
    let source = selected(&state, Source::Artifact)?;
    work(move || omnirecovery::inspect_artifact(&source, kind, 100).map_err(error)).await
}

#[tauri::command]
pub(super) async fn firmware_preflight(
    app: AppHandle,
    state: State<'_, Shared>,
    case: String,
    model: String,
    target: String,
    sha256: String,
) -> Result<Option<String>, String> {
    let source = selected(&state, Source::Firmware)?;
    work(move || {
        let result = app
            .dialog()
            .file()
            .set_title("Save firmware approval plan")
            .set_file_name("omnirecovery-firmware-plan.json")
            .add_filter("JSON", &["json"])
            .blocking_save_file();
        let Some(destination) = picked(result)? else {
            return Ok(None);
        };
        omnirecovery::make_action_plan(
            omnirecovery::SensitiveAction::Firmware,
            &case,
            &model,
            &target,
            Some((&source, &sha256)),
            &destination,
        )
        .map_err(error)?;
        Ok(Some(label(&destination)))
    })
    .await
}

#[tauri::command]
pub(super) async fn phone_wipe_plan(
    app: AppHandle,
    case: String,
    id: String,
) -> Result<Option<String>, String> {
    work(move || {
        let phone = omnirecovery::detect_devices(omnirecovery::Transport::Android)
            .map_err(error)?
            .into_iter()
            .find(|device| device.id == id && device.state == "device")
            .ok_or("Choose a currently authorized Android USB phone.")?;
        let model = phone
            .model
            .as_deref()
            .ok_or("Android did not report a model for this device.")?;
        let result = app
            .dialog()
            .file()
            .set_title("Save Android wipe approval plan")
            .set_file_name("omnirecovery-phone-wipe-plan.json")
            .add_filter("JSON", &["json"])
            .blocking_save_file();
        let Some(destination) = picked(result)? else {
            return Ok(None);
        };
        omnirecovery::make_action_plan(
            omnirecovery::SensitiveAction::Wipe,
            &case,
            model,
            &id,
            None,
            &destination,
        )
        .map_err(error)?;
        Ok(Some(label(&destination)))
    })
    .await
}
