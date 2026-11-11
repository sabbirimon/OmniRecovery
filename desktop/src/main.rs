#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod destination;
mod devices;
mod records;
mod security;
mod workflows;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use omnirecovery::{Catalogue, MediaType, RepairReport, ScanOptions, ScanReport, StorageProfile};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{AppHandle, State};
use tauri_plugin_dialog::{DialogExt, FilePath};
use zeroize::Zeroizing;

type Shared = Arc<Mutex<Workspace>>;

#[derive(Clone, Copy, Deserialize, Eq, Hash, PartialEq)]
#[serde(rename_all = "snake_case")]
enum Source {
    Image,
    Damaged,
    BeforeImage,
    AfterImage,
    BeforeFolder,
    AfterFolder,
    Transfer,
    Folder,
    File,
    Volume,
    AndroidExport,
    IosExport,
    CaptureFolder,
    CaptureImage,
    Evidence,
    Security,
}

impl Source {
    fn folder(self) -> bool {
        matches!(
            self,
            Self::BeforeFolder
                | Self::AfterFolder
                | Self::Transfer
                | Self::Folder
                | Self::Volume
                | Self::AndroidExport
                | Self::IosExport
                | Self::CaptureFolder
        )
    }
}

#[derive(Clone, Copy, Deserialize, Eq, Hash, PartialEq)]
#[serde(rename_all = "snake_case")]
enum ReportKind {
    Scan,
    Repair,
    Compare,
    Catalogue,
    Evidence,
    Security,
    Sanitize,
}

#[derive(Serialize)]
struct Selection {
    name: String,
    bytes: Option<u64>,
}

struct Scanned {
    source: PathBuf,
    report: ScanReport,
}
struct Repaired {
    source: PathBuf,
    expected: Option<String>,
    report: RepairReport,
}

struct Catalogued {
    source: PathBuf,
    report: Catalogue,
}

struct Workspace {
    paths: HashMap<Source, PathBuf>,
    reports: HashMap<ReportKind, Arc<Value>>,
    scan: Option<Arc<Scanned>>,
    repair: Option<Arc<Repaired>>,
    catalogues: HashMap<Source, Arc<Catalogued>>,
    disks: Vec<devices::Disk>,
    active_volume: Option<devices::Volume>,
    demo: tempfile::TempDir,
}

fn lock(state: &Shared) -> Result<MutexGuard<'_, Workspace>, String> {
    state
        .lock()
        .map_err(|_| "Workspace is unavailable. Reopen OmniRecovery.".into())
}

fn selected(state: &Shared, source: Source) -> Result<PathBuf, String> {
    lock(state)?
        .paths
        .get(&source)
        .cloned()
        .ok_or_else(|| "Choose a source first.".into())
}

fn set_source(state: &Shared, slot: Source, path: PathBuf) -> Result<Selection, String> {
    let metadata = path.symlink_metadata().map_err(error)?;
    if (slot.folder() && !metadata.is_dir()) || (!slot.folder() && !metadata.is_file()) {
        return Err("Choose an ordinary file or directory for this workflow.".into());
    }
    let selection = Selection {
        name: label(&path),
        bytes: metadata.is_file().then_some(metadata.len()),
    };
    let mut workspace = lock(state)?;
    workspace.paths.insert(slot, path);
    workspace.catalogues.remove(&slot);
    match slot {
        Source::Image => {
            workspace.scan = None;
            workspace.reports.remove(&ReportKind::Scan);
        }
        Source::Damaged => {
            workspace.repair = None;
            workspace.reports.remove(&ReportKind::Repair);
        }
        Source::BeforeImage | Source::AfterImage | Source::BeforeFolder | Source::AfterFolder => {
            workspace.reports.remove(&ReportKind::Compare);
        }
        _ => {}
    }
    Ok(selection)
}

async fn work<T: Send + 'static>(
    run: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(run)
        .await
        .map_err(error)?
}

fn label(path: &Path) -> String {
    path.file_name().map_or_else(
        || "Selected location".to_owned(),
        |name| name.to_string_lossy().into_owned(),
    )
}

fn error(value: impl std::fmt::Display) -> String {
    value.to_string()
}

fn picked(path: Option<FilePath>) -> Result<Option<PathBuf>, String> {
    path.map(|path| path.into_path().map_err(error)).transpose()
}

#[tauri::command]
async fn pick_source(
    app: AppHandle,
    state: State<'_, Shared>,
    slot: Source,
) -> Result<Option<Selection>, String> {
    let state = state.inner().clone();
    work(move || {
        let dialog = app.dialog().file().set_title(if slot.folder() {
            "Choose source folder"
        } else {
            "Choose source file"
        });
        let path = if slot.folder() {
            dialog.blocking_pick_folder()
        } else {
            dialog.blocking_pick_file()
        };
        picked(path)?
            .map(|path| set_source(&state, slot, path))
            .transpose()
    })
    .await
}

#[tauri::command]
async fn demo_image(state: State<'_, Shared>) -> Result<Selection, String> {
    let state = state.inner().clone();
    work(move || {
        let path = lock(&state)?.demo.path().join("demo-memory-card.dd");
        if !path.is_file() {
            let mut image = vec![0; 4096];
            image.extend_from_slice(include_bytes!("../assets/demo.png"));
            image.resize(8192, 0);
            image.extend_from_slice(include_bytes!("../assets/demo.jpg"));
            image.resize(16384, 0);
            std::fs::write(&path, image).map_err(error)?;
        }
        set_source(&state, Source::Image, path)
    })
    .await
}

#[tauri::command]
async fn scan_image(
    app: AppHandle,
    state: State<'_, Shared>,
    storage: StorageProfile,
    offset: u64,
    length: Option<u64>,
    media: Option<MediaType>,
) -> Result<Value, String> {
    let source = selected(&state, Source::Image)?;
    let state = state.inner().clone();
    work(move || {
        workflows::tracked(&app, "image_scan", || {
            let report = omnirecovery::scan_selected_media(
                &source,
                ScanOptions {
                    storage,
                    offset,
                    length,
                },
                media,
            )
            .map_err(error)?;
            let json = serde_json::to_value(&report).map_err(error)?;
            let mut workspace = lock(&state)?;
            workspace.scan = Some(Arc::new(Scanned { source, report }));
            workspace
                .reports
                .insert(ReportKind::Scan, Arc::new(json.clone()));
            Ok(json)
        })
    })
    .await
}

#[tauri::command]
async fn repair_file(state: State<'_, Shared>, expected: Option<String>) -> Result<Value, String> {
    let source = selected(&state, Source::Damaged)?;
    let state = state.inner().clone();
    work(move || {
        let report = omnirecovery::preview_repair(&source, expected.as_deref())
            .map_err(error)?
            .report;
        let json = serde_json::to_value(&report).map_err(error)?;
        let mut workspace = lock(&state)?;
        workspace.repair = Some(Arc::new(Repaired {
            source,
            expected,
            report,
        }));
        workspace
            .reports
            .insert(ReportKind::Repair, Arc::new(json.clone()));
        Ok(json)
    })
    .await
}

#[tauri::command]
async fn compare_sources(state: State<'_, Shared>, folders: bool) -> Result<Value, String> {
    let (before, after) = if folders {
        (Source::BeforeFolder, Source::AfterFolder)
    } else {
        (Source::BeforeImage, Source::AfterImage)
    };
    let before = selected(&state, before)?;
    let after = selected(&state, after)?;
    let state = state.inner().clone();
    work(move || {
        let json = if folders {
            serde_json::to_value(omnirecovery::compare_exports(&before, &after).map_err(error)?)
        } else {
            serde_json::to_value(omnirecovery::compare(&before, &after).map_err(error)?)
        }
        .map_err(error)?;
        lock(&state)?
            .reports
            .insert(ReportKind::Compare, Arc::new(json.clone()));
        Ok(json)
    })
    .await
}

#[tauri::command]
async fn create_transfer(
    app: AppHandle,
    state: State<'_, Shared>,
    password: String,
    selection: Option<Vec<usize>>,
) -> Result<Option<omnirecovery::BackupReport>, String> {
    let password = Zeroizing::new(password);
    let source = selected(&state, Source::Transfer)?;
    let paths = selection
        .map(|indices| workflows::selected_paths(&state, Source::Transfer, &indices))
        .transpose()?;
    work(move || {
        let path = app
            .dialog()
            .file()
            .set_title("Save encrypted phone-transfer archive")
            .set_file_name("phone-transfer.omnibackup")
            .add_filter("OmniRecovery archive", &["omnibackup"])
            .blocking_save_file();
        picked(path)?
            .map(|path| {
                match &paths {
                    Some(paths) => {
                        omnirecovery::backup_selected_files(&source, &path, &password, paths)
                    }
                    None => omnirecovery::backup_files(&source, &path, &password),
                }
                .map_err(error)
            })
            .transpose()
    })
    .await
}

enum Media {
    Scan(Arc<Scanned>, usize),
    Repair(Arc<Repaired>),
}

impl Media {
    fn select(state: &Shared, repair: bool, index: usize) -> Result<Self, String> {
        let workspace = lock(state)?;
        if repair {
            return workspace
                .repair
                .clone()
                .filter(|job| job.report.status == "repair_available")
                .map(Self::Repair)
                .ok_or_else(|| "There is no verified repair to preview.".into());
        }
        let job = workspace.scan.clone().ok_or("Scan an image first.")?;
        if index >= job.report.findings.len() {
            return Err("Choose a valid finding.".into());
        }
        Ok(Self::Scan(job, index))
    }

    fn source(&self) -> &Path {
        match self {
            Self::Scan(job, _) => &job.source,
            Self::Repair(job) => &job.source,
        }
    }

    fn media_type(&self) -> MediaType {
        match self {
            Self::Scan(job, index) => job.report.findings[*index].media_type,
            Self::Repair(job) => job
                .report
                .media_type
                .expect("verified repair has a media type"),
        }
    }

    fn read(&self) -> Result<Vec<u8>, String> {
        match self {
            Self::Scan(job, index) => omnirecovery::recover_candidate(
                &job.source,
                &job.report.source_sha256,
                &job.report.findings[*index],
            )
            .map_err(error),
            Self::Repair(job) => {
                let updated = omnirecovery::preview_repair(&job.source, job.expected.as_deref())
                    .map_err(error)?;
                let old_digest = job.report.result.as_ref().map(|result| &result.sha256);
                let new_digest = updated.report.result.as_ref().map(|result| &result.sha256);
                if updated.report.source_sha256 != job.report.source_sha256
                    || old_digest != new_digest
                {
                    return Err("The repair source changed. Run the assessment again.".into());
                }
                updated
                    .repaired_bytes
                    .ok_or_else(|| "The repair could not be re-verified.".into())
            }
        }
    }
}

#[derive(Serialize)]
struct MediaPreview {
    url: String,
    hex: String,
}

#[tauri::command]
async fn preview_media(
    state: State<'_, Shared>,
    repair: bool,
    index: usize,
) -> Result<MediaPreview, String> {
    let media = Media::select(&state, repair, index)?;
    work(move || {
        let bytes = media.read()?;
        let mime = match media.media_type() {
            MediaType::Jpeg => "image/jpeg",
            MediaType::Png => "image/png",
        };
        Ok(MediaPreview {
            url: format!("data:{mime};base64,{}", STANDARD.encode(&bytes)),
            hex: hex_rows(0, &bytes[..bytes.len().min(256)]),
        })
    })
    .await
}

fn hex_rows(start: u64, bytes: &[u8]) -> String {
    bytes
        .chunks(16)
        .enumerate()
        .map(|(index, row)| {
            let offset = start + index as u64 * 16;
            let cells = row
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<Vec<_>>()
                .join(" ");
            format!("{offset:08x}  {cells}")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[tauri::command]
async fn inspect_bytes(
    state: State<'_, Shared>,
    offset: u64,
    length: usize,
) -> Result<String, String> {
    let job = lock(&state)?.scan.clone().ok_or("Scan an image first.")?;
    work(move || {
        let bytes = omnirecovery::inspect_image_bytes(
            &job.source,
            &job.report.source_sha256,
            offset,
            length,
        )
        .map_err(error)?;
        Ok(hex_rows(offset, &bytes))
    })
    .await
}

#[tauri::command]
fn media_saving_available() -> bool {
    cfg!(target_os = "macos")
}

#[tauri::command]
async fn save_media(
    app: AppHandle,
    state: State<'_, Shared>,
    repair: bool,
    index: usize,
) -> Result<Option<String>, String> {
    let media = Media::select(&state, repair, index)?;
    work(move || {
        let extension = match media.media_type() {
            MediaType::Jpeg => "jpg",
            MediaType::Png => "png",
        };
        let path = app
            .dialog()
            .file()
            .set_title("Save to a different physical drive")
            .set_file_name(format!(
                "omnirecovery-{}-{index}.{extension}",
                if repair { "repair" } else { "recovered" }
            ))
            .add_filter("Verified image", &[extension])
            .blocking_save_file();
        let Some(path) = picked(path)? else {
            return Ok(None);
        };
        destination::save_media(media.source(), &path, || media.read())?;
        Ok(Some(label(&path)))
    })
    .await
}

#[tauri::command]
async fn save_report(
    app: AppHandle,
    state: State<'_, Shared>,
    kind: ReportKind,
) -> Result<Option<String>, String> {
    let report = lock(&state)?
        .reports
        .get(&kind)
        .cloned()
        .ok_or("Run the analysis first.")?;
    work(move || {
        let path = app
            .dialog()
            .file()
            .set_title("Save analysis report")
            .set_file_name("omnirecovery-report.json")
            .add_filter("JSON report", &["json"])
            .blocking_save_file();
        let Some(path) = picked(path)? else {
            return Ok(None);
        };
        destination::save_report(
            &path,
            &serde_json::to_vec_pretty(&report.as_ref()).map_err(error)?,
        )
        .map_err(error)?;
        Ok(Some(label(&path)))
    })
    .await
}

fn main() {
    let workspace = Workspace {
        paths: HashMap::new(),
        reports: HashMap::new(),
        scan: None,
        repair: None,
        catalogues: HashMap::new(),
        disks: Vec::new(),
        active_volume: None,
        demo: tempfile::tempdir().expect("temporary directory unavailable"),
    };
    tauri::Builder::default()
        .manage(Arc::new(Mutex::new(workspace)))
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            pick_source,
            demo_image,
            scan_image,
            repair_file,
            compare_sources,
            create_transfer,
            preview_media,
            inspect_bytes,
            media_saving_available,
            workflows::discover_devices,
            workflows::choose_volume,
            workflows::check_adb,
            workflows::scan_catalogue,
            workflows::inspect_catalogue_entry,
            workflows::copy_catalogue_entry,
            workflows::take_capture,
            workflows::capture_status,
            workflows::compare_saved,
            workflows::clone_image,
            workflows::verify_evidence,
            workflows::security_scan,
            workflows::activity_logs,
            workflows::move_activity_log,
            workflows::export_activity_log,
            workflows::sanitize_plan,
            save_media,
            save_report
        ])
        .run(tauri::generate_context!())
        .expect("failed to launch OmniRecovery");
}
