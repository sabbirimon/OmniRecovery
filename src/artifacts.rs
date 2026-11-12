use std::io::{self, Read, Write};
use std::path::Path;

use rusqlite::{Connection, OpenFlags, limits::Limit};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::source::{invalid, open_image, same_file_state};

const MAX_DATABASE: u64 = 512 * 1024 * 1024;
const MAX_CHAT: u64 = 8 * 1024 * 1024;
const MAX_ROWS: usize = 1_000;

#[derive(Clone, Copy, Debug, Serialize, clap::ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactKind {
    AndroidSms,
    AndroidCalls,
    IosSms,
    IosCalls,
    AndroidContactNames,
    IosContactNames,
    SqliteSchema,
    WhatsappExport,
}

#[derive(Debug, Serialize)]
pub struct ArtifactReport {
    pub kind: ArtifactKind,
    pub source_sha256: String,
    pub has_more: bool,
    pub time_encoding: &'static str,
    pub records: Vec<Value>,
}

fn query_for(kind: ArtifactKind) -> (&'static str, &'static str, &'static str) {
    match kind {
        ArtifactKind::AndroidSms => (
            "sms",
            "SELECT _id AS id, date AS time, address AS party, type AS direction, substr(body,1,4096) AS text FROM main.sms ORDER BY _id LIMIT ?1",
            "unix_milliseconds",
        ),
        ArtifactKind::AndroidCalls => (
            "calls",
            "SELECT _id AS id, date AS time, number AS party, type AS direction, duration AS duration_seconds FROM main.calls ORDER BY _id LIMIT ?1",
            "unix_milliseconds",
        ),
        ArtifactKind::IosSms => (
            "message",
            "SELECT ROWID AS id, date AS time, handle_id AS party_id, is_from_me AS outgoing, substr(text,1,4096) AS text FROM main.message ORDER BY ROWID LIMIT ?1",
            "apple_epoch_raw_value",
        ),
        ArtifactKind::IosCalls => (
            "ZCALLRECORD",
            "SELECT Z_PK AS id, ZDATE AS time, ZADDRESS AS party, ZORIGINATED AS outgoing, ZDURATION AS duration_seconds FROM main.ZCALLRECORD ORDER BY Z_PK LIMIT ?1",
            "apple_epoch_seconds",
        ),
        ArtifactKind::AndroidContactNames => (
            "raw_contacts",
            "SELECT _id AS id, contact_id, substr(display_name,1,1024) AS name FROM main.raw_contacts ORDER BY _id LIMIT ?1",
            "none",
        ),
        ArtifactKind::IosContactNames => (
            "ABPerson",
            "SELECT ROWID AS id, substr(First,1,1024) AS first_name, substr(Last,1,1024) AS last_name, substr(Organization,1,1024) AS organization FROM main.ABPerson ORDER BY ROWID LIMIT ?1",
            "none",
        ),
        ArtifactKind::SqliteSchema => (
            "",
            "SELECT name, type, ncol AS columns FROM pragma_table_list WHERE schema = 'main' AND name NOT LIKE 'sqlite_%' ORDER BY name LIMIT ?1",
            "none",
        ),
        ArtifactKind::WhatsappExport => unreachable!("text export uses a separate reader"),
    }
}

fn sqlite_records(path: &Path, kind: ArtifactKind, limit: usize) -> io::Result<ArtifactReport> {
    for suffix in ["-wal", "-journal"] {
        let mut sidecar = path.as_os_str().to_owned();
        sidecar.push(suffix);
        match std::fs::symlink_metadata(sidecar) {
            Ok(_) => {
                return Err(invalid(
                    "SQLite sidecar present; provide a standalone database backup",
                ));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    let mut input = open_image(path)?;
    let before = input.metadata()?;
    if !(100..=MAX_DATABASE).contains(&before.len()) {
        return Err(invalid(
            "SQLite database is outside the supported size range",
        ));
    }
    // SQLite only opens the private working copy. Original evidence and its directory
    // remain read-only throughout SQLite recovery or schema inspection.
    let private = tempfile::tempdir()?;
    let copy_path = private.path().join("artifact.sqlite");
    let mut copy = std::fs::File::create_new(&copy_path)?;
    let mut digest = Sha256::new();
    let mut remaining = before.len();
    let mut bytes = [0_u8; 64 * 1024];
    while remaining != 0 {
        let count = remaining.min(bytes.len() as u64) as usize;
        input.read_exact(&mut bytes[..count])?;
        digest.update(&bytes[..count]);
        copy.write_all(&bytes[..count])?;
        remaining -= count as u64;
    }
    drop(copy);
    if !same_file_state(&before, &input.metadata()?) {
        return Err(invalid("database changed while it was being copied"));
    }
    let db = Connection::open_with_flags(
        copy_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|_| invalid("could not open the supported SQLite export"))?;
    db.execute_batch("PRAGMA query_only=ON; PRAGMA trusted_schema=OFF;")
        .map_err(|_| invalid("could not initialize read-only SQLite inspection"))?;
    db.set_limit(Limit::SQLITE_LIMIT_LENGTH, 1_048_576)
        .map_err(|_| invalid("could not limit SQLite values"))?;
    let (table, sql, time_encoding) = query_for(kind);
    if !table.is_empty() {
        let ordinary: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_list WHERE schema='main' AND type='table' AND name=?1 COLLATE NOCASE)",
            [table], |row| row.get(0),
        ).map_err(|_| invalid("could not read SQLite table metadata"))?;
        if !ordinary {
            return Err(invalid(
                "the selected artifact table is missing or unsupported",
            ));
        }
    }
    let mut statement = db
        .prepare(sql)
        .map_err(|_| invalid("SQLite schema is incompatible with the selected artifact"))?;
    let names: Vec<String> = statement
        .column_names()
        .into_iter()
        .map(str::to_owned)
        .collect();
    let rows = statement
        .query_map([(limit + 1) as i64], |row| {
            use rusqlite::types::ValueRef;
            let mut record = serde_json::Map::new();
            for (index, name) in names.iter().enumerate() {
                let value = match row.get_ref(index)? {
                    ValueRef::Null | ValueRef::Blob(_) => Value::Null,
                    ValueRef::Integer(value) => Value::from(value),
                    ValueRef::Real(value) => Value::from(value),
                    ValueRef::Text(value) => {
                        Value::from(String::from_utf8_lossy(value).into_owned())
                    }
                };
                record.insert(name.clone(), value);
            }
            Ok(Value::Object(record))
        })
        .map_err(|_| invalid("could not query SQLite export"))?;
    let mut records = rows
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| invalid("could not decode SQLite records"))?;
    let has_more = records.len() > limit;
    records.truncate(limit);
    Ok(ArtifactReport {
        kind,
        source_sha256: format!("{:x}", digest.finalize()),
        has_more,
        time_encoding,
        records,
    })
}

fn whatsapp_line(line: &str) -> Option<(&str, &str)> {
    let line = line.trim_start_matches(['\u{feff}', '\u{200e}', '\u{200f}']);
    let (timestamp, body) = if let Some(line) = line.strip_prefix('[') {
        line.split_once("] ")?
    } else {
        line.split_once(" - ")?
    };
    let digits = timestamp.bytes().filter(u8::is_ascii_digit).count();
    (digits >= 6
        && timestamp.contains(':')
        && timestamp.contains(['/', '.', '-'])
        && timestamp.len() <= 80)
        .then_some((timestamp, body))
}

fn whatsapp_records(path: &Path, limit: usize) -> io::Result<ArtifactReport> {
    let mut input = open_image(path)?;
    let state = input.metadata()?;
    let mut bytes = Vec::new();
    (&mut input).take(MAX_CHAT + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_CHAT || !same_file_state(&state, &input.metadata()?) {
        return Err(invalid(
            "WhatsApp export exceeded its size limit or changed while reading",
        ));
    }
    let text =
        std::str::from_utf8(&bytes).map_err(|_| invalid("WhatsApp text export must be UTF-8"))?;
    let mut records = Vec::new();
    let mut has_more = false;
    for line in text.lines() {
        if let Some((time, body)) = whatsapp_line(line) {
            if records.len() == limit {
                has_more = true;
                break;
            }
            let (sender, message) = body
                .split_once(": ")
                .map_or((None, body), |(sender, message)| (Some(sender), message));
            records.push(serde_json::json!({
                "time_raw": time,
                "sender": sender.map(|name| name.chars().take(1024).collect::<String>()),
                "text": message.chars().take(4096).collect::<String>(),
            }));
        } else if let Some(record) = records.last_mut() {
            let message = record["text"].as_str().unwrap_or_default();
            if message.len() < 4096 {
                record["text"] = Value::from(format!(
                    "{message}\n{}",
                    line.chars().take(4096 - message.len()).collect::<String>()
                ));
            }
        }
    }
    if records.is_empty() {
        return Err(invalid("no supported WhatsApp text-export records found"));
    }
    Ok(ArtifactReport {
        kind: ArtifactKind::WhatsappExport,
        source_sha256: format!("{:x}", Sha256::digest(&bytes)),
        has_more,
        time_encoding: "export_locale_raw_text",
        records,
    })
}

/// Read supported records from a user-supplied exported SQLite database or WhatsApp text file.
pub fn inspect_artifact(
    path: &Path,
    kind: ArtifactKind,
    limit: usize,
) -> io::Result<ArtifactReport> {
    if !(1..=MAX_ROWS).contains(&limit) {
        return Err(invalid("choose between 1 and 1,000 records"));
    }
    match kind {
        ArtifactKind::WhatsappExport => whatsapp_records(path, limit),
        _ => sqlite_records(path, kind, limit),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sqlite_extracts_bounded_records_and_rejects_views() -> io::Result<()> {
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("sms.sqlite");
        let database = Connection::open(&path).unwrap();
        database.execute_batch("CREATE TABLE sms(_id INTEGER, date INTEGER, address TEXT, type INTEGER, body TEXT);
            INSERT INTO sms VALUES(1,0,'example',1,'hello'),(2,1,'example',2,'reply');").unwrap();
        drop(database);
        let report = inspect_artifact(&path, ArtifactKind::AndroidSms, 1)?;
        assert!(report.has_more);
        assert_eq!(report.records[0]["text"], "hello");
        Connection::open(&path)
            .unwrap()
            .execute_batch("DROP TABLE sms; CREATE VIEW sms AS SELECT 0;")
            .unwrap();
        assert!(inspect_artifact(&path, ArtifactKind::AndroidSms, 1).is_err());
        Ok(())
    }

    #[test]
    fn whatsapp_reads_android_and_ios_text_formats() -> io::Result<()> {
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("_chat.txt");
        std::fs::write(
            &path,
            "12/11/2026, 10:05 - Person: message\ncontinued\n[12/11/2026, 10:07:59] Other: reply\n",
        )?;
        let report = inspect_artifact(&path, ArtifactKind::WhatsappExport, 2)?;
        assert_eq!(report.records[0]["text"], "message\ncontinued");
        assert_eq!(report.records[1]["sender"], "Other");
        assert!(!report.has_more);
        Ok(())
    }
}
