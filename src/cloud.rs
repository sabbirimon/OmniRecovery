use std::io;
use std::path::Path;

use serde::Serialize;
use serde_json::{Value, json};

use crate::http::{curl_json, curl_transfer};
use crate::source::{hash_image, invalid};

const DRIVE: &str = "https://www.googleapis.com/drive/v3";
const PHOTOS: &str = "https://photospicker.googleapis.com/v1";
const MAX_DOWNLOAD: u64 = 512 * 1024 * 1024;

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && !matches!(id, "." | "..")
        && id.len() <= 512
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

/// List the first page of the caller's accessible trashed Google Drive entries.
pub fn google_drive_trash(token: &str, page_token: Option<&str>) -> io::Result<Value> {
    let mut url = format!(
        "{DRIVE}/files?q=trashed%3Dtrue&pageSize=100&fields=nextPageToken%2Cfiles%28id%2Cname%2CmimeType%2Csize%2Csha256Checksum%2CmodifiedTime%29"
    );
    if let Some(page) = page_token {
        if !valid_id(page) {
            return Err(invalid("invalid Google Drive page token"));
        }
        url.push_str("&pageToken=");
        url.push_str(page);
    }
    curl_json(&url, true, Some(token), None)
}

#[derive(Debug, Serialize)]
pub struct CloudDownload {
    pub bytes: u64,
    pub sha256: String,
    pub provider_hash_matched: bool,
}

fn save_download(
    url: &str,
    token: &str,
    destination: &Path,
    expected_hash: Option<&str>,
    expected_size: Option<u64>,
) -> io::Result<CloudDownload> {
    if expected_size.is_some_and(|length| length > MAX_DOWNLOAD) {
        return Err(invalid("provider file exceeds the 512 MiB download limit"));
    }
    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    curl_transfer(url, true, Some(token), None, temp.as_file(), MAX_DOWNLOAD)?;
    temp.as_file().sync_all()?;
    let bytes = temp.as_file().metadata()?.len();
    if expected_size.is_some_and(|length| length != bytes) {
        return Err(invalid(
            "download length did not match the provider metadata",
        ));
    }
    let sha256 = hash_image(temp.as_file_mut(), bytes)?;
    if expected_hash.is_some_and(|hash| !sha256.eq_ignore_ascii_case(hash)) {
        return Err(invalid(
            "download digest did not match the provider metadata",
        ));
    }
    temp.persist_noclobber(destination)
        .map_err(|error| error.error)?;
    Ok(CloudDownload {
        bytes,
        sha256,
        provider_hash_matched: expected_hash.is_some(),
    })
}

/// Save an authorized Drive blob locally without modifying Drive.
pub fn google_drive_download(
    token: &str,
    id: &str,
    destination: &Path,
) -> io::Result<CloudDownload> {
    if !valid_id(id) {
        return Err(invalid("invalid Google Drive file ID"));
    }
    let metadata = curl_json(
        &format!(
            "{DRIVE}/files/{id}?fields=mimeType%2Csize%2Csha256Checksum%2Ccapabilities%28canDownload%29"
        ),
        true,
        Some(token),
        None,
    )?;
    if metadata["mimeType"]
        .as_str()
        .is_none_or(|mime| mime.starts_with("application/vnd.google-apps."))
        || metadata
            .pointer("/capabilities/canDownload")
            .and_then(Value::as_bool)
            != Some(true)
    {
        return Err(invalid(
            "the Drive item does not expose a downloadable blob",
        ));
    }
    let expected_size = metadata["size"]
        .as_str()
        .and_then(|size| size.parse().ok())
        .ok_or_else(|| invalid("missing Drive blob size"))?;
    save_download(
        &format!("{DRIVE}/files/{id}?alt=media"),
        token,
        destination,
        metadata["sha256Checksum"].as_str(),
        Some(expected_size),
    )
}

/// Start a Google Photos selection session for the operator to open in a browser.
pub fn google_photos_session(token: &str) -> io::Result<Value> {
    let response = curl_json(
        &format!("{PHOTOS}/sessions"),
        true,
        Some(token),
        Some(&json!({})),
    )?;
    let id = response["id"]
        .as_str()
        .filter(|id| valid_id(id))
        .ok_or_else(|| invalid("missing Photos session ID"))?;
    let picker = response["pickerUri"]
        .as_str()
        .filter(|url| url.starts_with("https://photos.google.com/"))
        .ok_or_else(|| invalid("unexpected Photos picker origin"))?;
    Ok(json!({"session_id": id, "picker_url": picker}))
}

fn photos_items(token: &str, session: &str, page: Option<&str>) -> io::Result<Value> {
    if !valid_id(session) || page.is_some_and(|value| !valid_id(value)) {
        return Err(invalid("invalid Photos session or page token"));
    }
    let mut url = format!("{PHOTOS}/mediaItems?sessionId={session}&pageSize=100");
    if let Some(page) = page {
        url.push_str("&pageToken=");
        url.push_str(page);
    }
    curl_json(&url, true, Some(token), None)
}

pub fn google_photos_list(token: &str, session: &str, page: Option<&str>) -> io::Result<Value> {
    let mut items = photos_items(token, session, page)?;
    if let Some(items) = items["mediaItems"].as_array_mut() {
        for item in items {
            if let Some(file) = item["mediaFile"].as_object_mut() {
                file.remove("baseUrl");
            }
        }
    }
    Ok(items)
}

fn photos_download_url(base: &str, video: bool) -> io::Result<String> {
    if !base.starts_with("https://lh3.googleusercontent.com/")
        || base.contains(['\r', '\n', '\t', ' ', '\\', '#', '?'])
    {
        return Err(invalid("unexpected Google Photos media origin"));
    }
    Ok(format!("{base}={}", if video { "dv" } else { "d" }))
}

pub fn google_photos_download(
    token: &str,
    session: &str,
    id: &str,
    page: Option<&str>,
    destination: &Path,
) -> io::Result<CloudDownload> {
    let items = photos_items(token, session, page)?;
    let item = items["mediaItems"]
        .as_array()
        .and_then(|items| items.iter().find(|item| item["id"].as_str() == Some(id)))
        .ok_or_else(|| invalid("the Photos item was not found on the selected session page"))?;
    let base = item
        .pointer("/mediaFile/baseUrl")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("the selected Photos item lacks a media URL"))?;
    let url = photos_download_url(base, item["type"].as_str() == Some("VIDEO"))?;
    save_download(&url, token, destination, None, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_media_urls_are_origin_bound() {
        assert!(
            photos_download_url("https://lh3.googleusercontent.com.example.invalid/p", false)
                .is_err()
        );
        assert!(
            photos_download_url(
                "https://lh3.googleusercontent.com@elsewhere.invalid/p",
                false
            )
            .is_err()
        );
        assert_eq!(
            photos_download_url("https://lh3.googleusercontent.com/path", true).unwrap(),
            "https://lh3.googleusercontent.com/path=dv"
        );
        assert!(valid_id("abcd_EFG-12"));
        assert!(!valid_id("../drive"));
    }
}
