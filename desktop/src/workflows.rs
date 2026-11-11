use super::*;
use omnirecovery::FileCategory;
use serde_json::json;

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
        let current = devices::discover()?;
        let after = devices::volume(&current.disks, &id)
            .filter(|volume| {
                volume.selectable && volume.uuid == before.uuid && volume.mount == before.mount
            })
            .ok_or("The selected volume changed. Refresh Devices & storage and choose it again.")?;
        let selection = set_source(
            &state,
            Source::Volume,
            after.mount.clone().ok_or("Volume is not mounted.")?,
        )?;
        lock(&state)?.active_volume = Some(after.clone());
        Ok(selection)
    })
    .await
}

#[tauri::command]
pub(super) async fn check_adb() -> Result<Vec<devices::AdbDevice>, String> {
    work(devices::adb_devices).await
}

fn verify_mounted(state: &Shared, slot: Source, source: &Path) -> Result<(), String> {
    if slot != Source::Volume {
        return Ok(());
    }
    let volume = lock(state)?
        .active_volume
        .clone()
        .ok_or("Choose the mounted volume from Devices & storage.")?;
    let devices = devices::discover()?;
    if volume.mount.as_deref() != Some(source)
        || !devices::volume(&devices.disks, &volume.id).is_some_and(|current| {
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
    let sha256 = destination::copy_file(source, &path)?;
    Ok(Some(json!({"name":label(&path),"sha256":sha256})))
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
    if case_reference.len() > 128 {
        return Err("Case reference is too long.".into());
    }
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
