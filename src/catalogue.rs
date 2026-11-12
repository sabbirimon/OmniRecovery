use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::snapshot::{ExportFile, on_root_device};
use crate::source::{hash_image, invalid, is_reparse_point, open_image, same_file_state};

const MAX_ENTRIES: usize = 25_000;
const MAX_DEPTH: usize = 48;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FileCategory {
    Photo,
    Video,
    Audio,
    Document,
    Log,
    Contact,
    Message,
    CallHistory,
    ChatHistory,
    Whatsapp,
    Messenger,
    Mail,
    DeviceInfo,
    Database,
    Account,
    Browser,
    Archive,
    Other,
}

#[derive(Clone, Debug, Serialize)]
pub struct CatalogueEntry {
    pub path: String,
    pub length: u64,
    pub category: FileCategory,
}

#[derive(Debug, Serialize)]
pub struct Catalogue {
    pub schema_version: u32,
    pub engine_version: &'static str,
    pub entries: Vec<CatalogueEntry>,
    pub inspected: usize,
    pub skipped: usize,
    pub limit_reached: bool,
}

/// Catalogue readable directory entries without opening file contents or following links.
pub fn catalogue(
    source: &Path,
    category: Option<FileCategory>,
    query: &str,
) -> io::Result<Catalogue> {
    if query.len() > 256 {
        return Err(invalid("search text exceeds 256 bytes"));
    }
    let query = query.to_lowercase();
    let root = fs::symlink_metadata(source)?;
    if is_reparse_point(&root) || !(root.is_file() || root.is_dir()) {
        return Err(invalid("choose a regular file or directory"));
    }
    let mut report = Catalogue {
        schema_version: 1,
        engine_version: env!("CARGO_PKG_VERSION"),
        entries: Vec::new(),
        inspected: 0,
        skipped: 0,
        limit_reached: false,
    };
    if root.is_file() {
        let name = source
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| invalid("file name is not UTF-8"))?;
        report.inspected = 1;
        include(&mut report, name.to_owned(), root.len(), category, &query);
        return Ok(report);
    }
    let mut pending = vec![(source.to_owned(), String::new(), 0)];
    while let Some((path, relative, depth)) = pending.pop() {
        let result = fs::symlink_metadata(&path).and_then(|metadata| {
            if !metadata.is_dir()
                || is_reparse_point(&metadata)
                || !on_root_device(&root, &metadata)
            {
                return Err(invalid("directory changed during enumeration"));
            }
            fs::read_dir(&path)
        });
        let items = match result {
            Ok(items) => items,
            Err(error) if depth == 0 => return Err(error),
            Err(_) => {
                report.skipped += 1;
                continue;
            }
        };
        for item in items {
            if report.inspected == MAX_ENTRIES {
                report.limit_reached = true;
                break;
            }
            report.inspected += 1;
            let Ok(item) = item else {
                report.skipped += 1;
                continue;
            };
            let name = item.file_name();
            let Some(name) = name.to_str() else {
                report.skipped += 1;
                continue;
            };
            let child = item.path();
            let Ok(metadata) = fs::symlink_metadata(&child) else {
                report.skipped += 1;
                continue;
            };
            if is_reparse_point(&metadata)
                || !on_root_device(&root, &metadata)
                || metadata.file_type().is_symlink()
            {
                report.skipped += 1;
                continue;
            }
            let key = if relative.is_empty() {
                name.to_owned()
            } else {
                format!("{relative}/{name}")
            };
            if key.len() > 4096 {
                report.skipped += 1;
                continue;
            }
            if metadata.is_file() {
                include(&mut report, key, metadata.len(), category, &query);
            } else if metadata.is_dir() && depth < MAX_DEPTH {
                pending.push((child, key, depth + 1));
            } else {
                report.skipped += 1;
            }
        }
        if report.limit_reached {
            break;
        }
    }
    report.entries.sort_unstable_by(|a, b| a.path.cmp(&b.path));
    Ok(report)
}

fn include(
    report: &mut Catalogue,
    path: String,
    length: u64,
    filter: Option<FileCategory>,
    query: &str,
) {
    let category = classify(&path);
    if filter.is_none_or(|requested| {
        requested == category
            || requested == FileCategory::Message
                && matches!(category, FileCategory::CallHistory | FileCategory::Mail)
    }) && (query.is_empty() || path.to_lowercase().contains(query))
    {
        report.entries.push(CatalogueEntry {
            path,
            length,
            category,
        });
    }
}

pub(crate) fn classify(path: &str) -> FileCategory {
    use FileCategory::*;
    let path = path.to_ascii_lowercase();
    let name = path.rsplit('/').next().unwrap_or(&path);
    let (base, extension) = name.rsplit_once('.').unwrap_or((name, ""));
    let text_export = matches!(extension, "json" | "csv" | "xml" | "txt");
    let record_export = text_export
        || matches!(
            extension,
            "html"
                | "htm"
                | "db"
                | "db-wal"
                | "sqlite"
                | "sqlite3"
                | "sqlitedb"
                | "sqlite-wal"
                | "storedata"
                | "plist"
        );
    let starts = |prefixes: &[&str]| prefixes.iter().any(|prefix| base.starts_with(prefix));
    let in_folder = |folders: &[&str]| {
        path.split('/')
            .rev()
            .skip(1)
            .any(|part| folders.contains(&part))
    };
    if (record_export || extension.starts_with("crypt"))
        && (starts(&["whatsapp", "msgstore"]) || in_folder(&["whatsapp", "com.whatsapp"]))
    {
        return Whatsapp;
    }
    if record_export
        && (starts(&["messenger", "facebook-messages", "facebook_messages"])
            || in_folder(&["messenger", "com.facebook.orca"])
            || path.contains("facebook/messages/"))
    {
        return Messenger;
    }
    if record_export
        && starts(&[
            "callhistory",
            "call_history",
            "call-history",
            "calllog",
            "call_log",
            "call-log",
            "calls",
        ])
    {
        return CallHistory;
    }
    if record_export && starts(&["contacts", "addressbook", "address_book"]) {
        return Contact;
    }
    if record_export && starts(&["sms", "mms", "mmssms", "messages"]) {
        return Message;
    }
    if record_export && starts(&["chat_history", "chat-history", "telegram", "signal-chat"]) {
        return ChatHistory;
    }
    if record_export
        && starts(&[
            "imei",
            "device-info",
            "device_info",
            "device-identifiers",
            "device_identifiers",
            "hardware-info",
            "hardware_info",
        ])
    {
        return DeviceInfo;
    }
    if (record_export && starts(&["mail-export", "mail_export", "mail-history", "mail_history"]))
        || matches!(extension, "eml" | "mbox" | "emlx" | "pst" | "ost")
    {
        return Mail;
    }
    if matches!(extension, "json" | "csv" | "xml" | "txt")
        && (base == "accounts"
            || base.starts_with("accounts-")
            || base.starts_with("account_export"))
    {
        return Account;
    }
    if matches!(extension, "json" | "csv" | "html" | "htm")
        && ["bookmark", "history", "tabs", "session"]
            .iter()
            .any(|part| base.contains(part))
    {
        return Browser;
    }
    if matches!(extension, "txt" | "json" | "csv" | "xml")
        && ["logcat", "dumpsys", "syslog", "log-", "security-log"]
            .iter()
            .any(|part| base.starts_with(part))
    {
        return Log;
    }
    match extension {
        "jpg" | "jpeg" | "png" | "heic" | "heif" | "gif" | "webp" | "bmp" | "tif" | "tiff"
        | "dng" | "cr2" | "cr3" | "nef" | "arw" | "avif" => Photo,
        "mp4" | "mov" | "m4v" | "3gp" | "mkv" | "avi" | "webm" | "ts" | "mpeg" | "mpg" => Video,
        "mp3" | "m4a" | "aac" | "wav" | "flac" | "ogg" | "opus" | "amr" | "aiff" | "wma" => Audio,
        "pdf" | "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" | "odt" | "ods" | "txt"
        | "rtf" | "md" | "json" | "csv" | "xml" | "html" => Document,
        "vcf" | "vcard" => Contact,
        "log" | "evtx" | "ips" | "crash" | "trace" => Log,
        "db" | "db-wal" | "sqlite" | "sqlite3" | "sqlite-wal" => Database,
        "zip" | "tar" | "gz" | "7z" | "rar" | "ab" | "apk" | "omnibackup" => Archive,
        _ => Other,
    }
}

/// Resolve a selected catalogue path, rechecking each component before access.
pub fn catalogue_file(root: &Path, relative: &str) -> io::Result<PathBuf> {
    let origin = fs::symlink_metadata(root)?;
    if origin.is_file()
        && !is_reparse_point(&origin)
        && root.file_name().and_then(|name| name.to_str()) == Some(relative)
    {
        return Ok(root.to_owned());
    }
    if !origin.is_dir()
        || is_reparse_point(&origin)
        || relative.is_empty()
        || relative.contains('\\')
    {
        return Err(invalid("invalid catalogue selection"));
    }
    let mut path = root.to_owned();
    let mut components = relative.split('/').peekable();
    while let Some(component) = components.next() {
        if matches!(component, "" | "." | "..")
            || Path::new(component).components().count() != 1
            || !matches!(
                Path::new(component).components().next(),
                Some(std::path::Component::Normal(_))
            )
        {
            return Err(invalid("invalid catalogue path component"));
        }
        path.push(component);
        let metadata = fs::symlink_metadata(&path)?;
        if is_reparse_point(&metadata)
            || !on_root_device(&origin, &metadata)
            || if components.peek().is_some() {
                !metadata.is_dir()
            } else {
                !metadata.is_file()
            }
        {
            return Err(invalid("catalogue entry changed or is outside its source"));
        }
    }
    Ok(path)
}

/// Hash a regular file and verify the opened file did not change during the read.
pub fn verify_file(path: &Path) -> io::Result<ExportFile> {
    let mut file = open_image(path)?;
    let before = file.metadata()?;
    let sha256 = hash_image(&mut file, before.len())?;
    if !same_file_state(&before, &file.metadata()?) {
        return Err(invalid("file changed while hashing"));
    }
    Ok(ExportFile {
        length: before.len(),
        sha256,
    })
}
