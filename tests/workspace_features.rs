use std::fs;
use std::io::Write;
use std::process::{Command, Stdio};

use omnirecovery::{FileCategory, backup_selected_files, catalogue, catalogue_file, scan};
use tempfile::tempdir;

#[test]
fn catalogue_categorizes_exports_and_searches_paths() {
    let dir = tempdir().unwrap();
    for file in [
        "clip.mp4",
        "notes.pdf",
        "system.log",
        "accounts.json",
        "saved-tabs.json",
        "contacts.vcf",
        "data.sqlite",
    ] {
        fs::write(dir.path().join(file), b"fixture").unwrap();
    }
    let report = catalogue(dir.path(), None, "").unwrap();
    assert_eq!(report.entries.len(), 7);
    assert_eq!(
        catalogue(dir.path(), Some(FileCategory::Video), "CLIP")
            .unwrap()
            .entries[0]
            .path,
        "clip.mp4"
    );
    assert_eq!(
        catalogue(dir.path(), Some(FileCategory::Browser), "")
            .unwrap()
            .entries[0]
            .path,
        "saved-tabs.json"
    );
    assert!(catalogue_file(dir.path(), "../outside.txt").is_err());
}

#[test]
fn catalogue_distinguishes_readable_phone_export_artifacts() {
    let dir = tempdir().unwrap();
    let cases = [
        ("sms.db", FileCategory::Message),
        ("CallHistory.storedata", FileCategory::CallHistory),
        ("AddressBook.sqlitedb", FileCategory::Contact),
        ("Inbox.mbox", FileCategory::Mail),
        ("chat_history.json", FileCategory::ChatHistory),
        ("WhatsApp/msgstore.db.crypt14", FileCategory::Whatsapp),
        (
            "facebook/messages/inbox/thread/message_1.json",
            FileCategory::Messenger,
        ),
        ("IMEI.json", FileCategory::DeviceInfo),
    ];
    for (name, category) in cases {
        let path = dir.path().join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"fixture").unwrap();
        let report = catalogue(dir.path(), Some(category), name).unwrap();
        assert_eq!(report.entries[0].category, category, "{name}");
        assert_eq!(report.entries[0].path, name);
    }
}

#[test]
fn selected_phone_clone_packages_only_chosen_export_paths() {
    let dir = tempdir().unwrap();
    let source = dir.path().join("export");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("keep.txt"), b"keep").unwrap();
    fs::write(source.join("omit.txt"), b"leave this out").unwrap();
    let target = dir.path().join("selected.omnibackup");
    let result = backup_selected_files(
        &source,
        &target,
        "synthetic passphrase only",
        &["keep.txt".into()],
    )
    .unwrap();
    assert_eq!((result.files, result.source_bytes), (1, 4));
    assert!(target.is_file());
}

#[test]
fn gpt_backup_is_reported_as_a_partition_candidate() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("backup-gpt.dd");
    let mut image = vec![0; 128 * 512];
    let table = &mut image[126 * 512..127 * 512];
    table[0] = 1;
    table[16] = 2;
    table[32..40].copy_from_slice(&40_u64.to_le_bytes());
    table[40..48].copy_from_slice(&80_u64.to_le_bytes());
    let table_crc = crc32fast::hash(table);
    let header = &mut image[127 * 512..];
    header[..8].copy_from_slice(b"EFI PART");
    header[8..12].copy_from_slice(&0x0001_0000_u32.to_le_bytes());
    header[12..16].copy_from_slice(&92_u32.to_le_bytes());
    for (offset, value) in [(24, 127_u64), (32, 1), (40, 34), (48, 94), (72, 126)] {
        header[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
    }
    header[80..84].copy_from_slice(&4_u32.to_le_bytes());
    header[84..88].copy_from_slice(&128_u32.to_le_bytes());
    header[88..92].copy_from_slice(&table_crc.to_le_bytes());
    let crc = crc32fast::hash(&header[..92]);
    header[16..20].copy_from_slice(&crc.to_le_bytes());
    fs::write(&path, &image).unwrap();
    let map = scan(&path).unwrap().partition_map.unwrap();
    assert_eq!(map.primary_status, "unavailable_or_invalid");
    assert_eq!(map.backup_status, "crc_verified");
    assert_eq!(map.partitions[0].provenance, "backup_gpt_candidate");
    assert_eq!(map.partitions[0].offset, 40 * 512);
    image[126 * 512 + 10] ^= 1;
    fs::write(&path, image).unwrap();
    assert!(scan(&path).unwrap().partition_map.is_none());
}

#[test]
fn agent_stdio_searches_and_rejects_outside_paths() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("record.log"), b"sample").unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_omnirecovery"))
        .arg("agent")
        .arg(dir.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    input.write_all(b"{\"operation\":\"search\",\"query\":\"record\",\"category\":\"log\"}\n{\"operation\":\"verify\",\"path\":\"../outside\"}\n").unwrap();
    drop(input);
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    let rows: Vec<serde_json::Value> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(rows[0]["report"]["entries"][0]["path"], "record.log");
    assert_eq!(rows[1]["ok"], false);
}
