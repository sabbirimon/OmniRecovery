use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tauri::{AppHandle, Manager};

#[derive(Debug, Serialize)]
pub struct Activity {
    pub id: String,
    pub operation: String,
    pub started_ms: u64,
    pub finished_ms: u64,
    pub outcome: &'static str,
    pub summary: Value,
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |value| value.as_millis() as u64)
}

pub fn record(
    app: &AppHandle,
    operation: &str,
    started_ms: u64,
    result: &Result<Value, String>,
) -> Result<(), String> {
    let summary = result.as_ref().map_or_else(
        |_| json!({"status":"operation_failed"}),
        |report| {
            let mut summary = serde_json::Map::new();
            for key in [
                "source_sha256",
                "source_kind",
                "sha256",
                "bytes",
                "observed_bytes_scanned",
                "container",
                "storage_profile",
                "selected_range",
                "inspected",
                "skipped",
                "limit_reached",
                "result",
                "engine",
                "matches_reference",
                "stage",
                "kind",
                "entries_count",
            ] {
                if let Some(value) = report.get(key).filter(|value| {
                    !value.is_array() && !value.is_object() || key == "selected_range"
                }) {
                    summary.insert(key.to_owned(), value.clone());
                }
            }
            for key in ["findings", "rejected", "entries", "indicators"] {
                if let Some(items) = report.get(key).and_then(Value::as_array) {
                    summary.insert(format!("{key}_count"), json!(items.len()));
                }
            }
            Value::Object(summary)
        },
    );
    let mut random = [0; 12];
    getrandom::fill(&mut random).map_err(error)?;
    let id: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
    let log = Activity {
        id: id.clone(),
        operation: operation.into(),
        started_ms,
        finished_ms: now(),
        outcome: if result.is_ok() {
            "completed"
        } else {
            "failed"
        },
        summary,
    };
    store(
        &log_dir(app, false)?.join(format!("{id}.json")),
        &log,
        false,
    )
}

pub fn logs(app: &AppHandle, trash: bool) -> Result<Vec<Value>, String> {
    let mut entries = Vec::new();
    for item in fs::read_dir(log_dir(app, trash)?).map_err(error)? {
        let item = item.map_err(error)?;
        let path = item.path();
        if path.extension().and_then(|name| name.to_str()) != Some("json")
            || !path
                .file_stem()
                .and_then(|name| name.to_str())
                .is_some_and(valid_id)
        {
            continue;
        }
        if item.file_type().map_err(error)?.is_file()
            && item.metadata().map_err(error)?.len() <= 64 * 1024
        {
            if let Ok(value) = read(&path) {
                entries.push(value);
            }
        }
    }
    entries
        .sort_unstable_by_key(|entry| std::cmp::Reverse(entry["started_ms"].as_u64().unwrap_or(0)));
    entries.truncate(1000);
    Ok(entries)
}

pub fn log(app: &AppHandle, id: &str, trash: bool) -> Result<Value, String> {
    if !valid_id(id) {
        return Err("Invalid activity log selection.".into());
    }
    read(&log_dir(app, trash)?.join(format!("{id}.json")))
}

pub fn move_log(app: &AppHandle, id: &str, from_trash: bool) -> Result<(), String> {
    if !valid_id(id) {
        return Err("Invalid activity log selection.".into());
    }
    let name = format!("{id}.json");
    let source = log_dir(app, from_trash)?.join(&name);
    let target = log_dir(app, !from_trash)?.join(&name);
    let value = read(&source)?;
    store(&target, &value, false)?;
    fs::remove_file(source).map_err(error)
}

fn valid_id(id: &str) -> bool {
    id.len() == 24 && id.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn log_dir(app: &AppHandle, trash: bool) -> Result<PathBuf, String> {
    directory(
        app,
        if trash {
            "deleted-activity"
        } else {
            "activity"
        },
    )
}

fn directory(app: &AppHandle, name: &str) -> Result<PathBuf, String> {
    let path = app.path().app_local_data_dir().map_err(error)?.join(name);
    fs::create_dir_all(&path).map_err(error)?;
    Ok(path)
}

fn store(path: &Path, value: &impl Serialize, replace: bool) -> Result<(), String> {
    let parent = path.parent().ok_or("Missing record location.")?;
    let mut file = tempfile::NamedTempFile::new_in(parent).map_err(error)?;
    serde_json::to_writer_pretty(&mut file, value).map_err(error)?;
    file.flush().map_err(error)?;
    file.as_file().sync_all().map_err(error)?;
    if replace {
        file.persist(path).map_err(error)?;
    } else {
        file.persist_noclobber(path).map_err(error)?;
    }
    Ok(())
}

fn read(path: &Path) -> Result<Value, String> {
    let metadata = path.symlink_metadata().map_err(error)?;
    if !metadata.is_file() || metadata.len() > 96 * 1024 * 1024 {
        return Err("Invalid record file.".into());
    }
    serde_json::from_reader(fs::File::open(path).map_err(error)?).map_err(error)
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Before,
    After,
}
impl Stage {
    fn file(self) -> &'static str {
        match self {
            Self::Before => "before.json",
            Self::After => "after.json",
        }
    }
}

#[derive(Deserialize, Serialize)]
pub struct Capture {
    pub schema_version: u8,
    pub stage: Stage,
    pub kind: String,
    pub name: String,
    pub captured_ms: u64,
    pub source_sha256: Option<String>,
    pub entries: BTreeMap<String, String>,
    pub incomplete: bool,
}

pub fn capture(
    app: &AppHandle,
    source: &Path,
    stage: Stage,
    folder: bool,
) -> Result<Value, String> {
    let (kind, source_sha256, entries, incomplete) = if folder {
        let inventory = omnirecovery::inventory(source).map_err(error)?;
        (
            "folder",
            None,
            inventory
                .files
                .into_iter()
                .map(|(name, file)| (name, file.sha256))
                .collect(),
            !inventory.skipped.is_empty(),
        )
    } else {
        let report = omnirecovery::scan(source).map_err(error)?;
        (
            "image",
            Some(report.source_sha256),
            report
                .findings
                .into_iter()
                .map(|finding| {
                    (
                        format!("{:?} · {}", finding.media_type, finding.sha256),
                        finding.sha256,
                    )
                })
                .collect(),
            !report.observed_ranges_fully_scanned
                || report.fill_bytes_skipped + report.unspecified_bytes_skipped > 0,
        )
    };
    let snapshot = Capture {
        schema_version: 1,
        stage,
        kind: kind.into(),
        name: super::label(source),
        captured_ms: now(),
        source_sha256,
        entries,
        incomplete,
    };
    store(
        &directory(app, "captures")?.join(stage.file()),
        &snapshot,
        true,
    )?;
    Ok(summary(&snapshot))
}

pub fn capture_summary(app: &AppHandle) -> Result<Value, String> {
    Ok(
        json!({"before": load_capture(app, Stage::Before)?.as_ref().map(summary), "after": load_capture(app, Stage::After)?.as_ref().map(summary)}),
    )
}

fn summary(capture: &Capture) -> Value {
    json!({"stage":capture.stage,"kind":capture.kind,"name":capture.name,"captured_ms":capture.captured_ms,"entries_count":capture.entries.len(),"incomplete":capture.incomplete,"source_sha256":capture.source_sha256})
}

fn load_capture(app: &AppHandle, stage: Stage) -> Result<Option<Capture>, String> {
    let path = directory(app, "captures")?.join(stage.file());
    if !path.try_exists().map_err(error)? {
        return Ok(None);
    }
    serde_json::from_value(read(&path)?)
        .map(Some)
        .map_err(error)
}

pub fn compare_captures(app: &AppHandle) -> Result<Value, String> {
    let before = load_capture(app, Stage::Before)?.ok_or("Take a before capture first.")?;
    let after = load_capture(app, Stage::After)?.ok_or("Take an after capture first.")?;
    if before.kind != after.kind {
        return Err("The saved captures must use the same source type.".into());
    }
    let mut paths = BTreeMap::new();
    for (name, digest) in &before.entries {
        let other = after.entries.get(name);
        let observation = match other {
            Some(other) if digest == other => "same_bytes",
            Some(_) => "changed_bytes",
            None => "observed_before_only",
        };
        paths.insert(
            name,
            json!({"observation":observation,"before_sha256":digest,"after_sha256":other}),
        );
    }
    for (name, digest) in &after.entries {
        paths
            .entry(name)
            .or_insert_with(|| json!({"observation":"observed_after_only","after_sha256":digest}));
    }
    Ok(
        json!({"schema_version":1,"before":summary(&before),"after":summary(&after),"paths":paths,"interpretation":"Capture stage is operator-assigned. Absence from a capture does not prove deletion."}),
    )
}

fn error(error: impl std::fmt::Display) -> String {
    error.to_string()
}
