use std::io;

use serde::Serialize;
use serde_json::{Value, json};
use zeroize::Zeroizing;

use crate::{ScanReport, http::curl_json, source::invalid};

#[derive(Debug, Serialize)]
pub struct AiTriage {
    pub provider: &'static str,
    pub model: String,
    pub scope: &'static str,
    pub advice: String,
}

fn aggregate(report: &ScanReport) -> Value {
    json!({
        "source_container": report.container,
        "filesystem_hint": report.filesystem_signature,
        "scan_bytes": report.observed_bytes_scanned,
        "scan_complete": report.observed_ranges_fully_scanned,
        "candidate_limit": report.candidate_limit_reached,
        "validation_limit": report.validation_budget_reached,
        "validated": report.findings.len(),
        "rejected": report.rejected.len(),
        "triage": report.triage,
    })
}

/// Request advisory text using only aggregate scan counts and format metadata.
pub fn ai_triage(
    report: &ScanReport,
    model: &str,
    remote_url: Option<&str>,
) -> io::Result<AiTriage> {
    if model.is_empty()
        || model.len() > 160
        || !model
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b':' | b'-' | b'.' | b'_' | b'/'))
    {
        return Err(invalid("invalid AI model identifier"));
    }
    let messages = [
        json!({"role": "system", "content": "Explain the supplied OmniRecovery scan metrics. Give at most three concise verification or review steps. Do not infer file contents or declare deleted data recovered. You have no tools."}),
        json!({"role": "user", "content": aggregate(report).to_string()}),
    ];
    let (provider, response) = if let Some(url) = remote_url {
        let authority = url
            .strip_prefix("https://")
            .and_then(|rest| rest.split('/').next());
        if authority.is_none_or(|value| value.is_empty() || value.contains('@'))
            || url.contains(['\r', '\n', '\t', ' ', '\\', '#', '?'])
            || url.len() > 2048
        {
            return Err(invalid(
                "remote AI requires a fixed HTTPS endpoint without embedded credentials or query parameters",
            ));
        }
        let token = Zeroizing::new(std::env::var("OMNIRECOVERY_AI_TOKEN").map_err(|_| {
            invalid("set OMNIRECOVERY_AI_TOKEN for the explicitly selected remote provider")
        })?);
        if token.is_empty()
            || token.len() > 1024
            || !token.bytes().all(|b| {
                b.is_ascii_alphanumeric()
                    || matches!(b, b'.' | b'-' | b'_' | b'~' | b'+' | b'/' | b'=')
            })
        {
            return Err(invalid("invalid remote provider token format"));
        }
        let response = curl_json(
            url,
            true,
            Some(&token),
            Some(&json!({
                "model": model, "messages": messages, "stream": false, "max_tokens": 384
            })),
        )?;
        ("openai_compatible_https", response)
    } else {
        if model.to_ascii_lowercase().contains("cloud") {
            return Err(invalid("select an installed, local-only Ollama model"));
        }
        let installed = curl_json("http://127.0.0.1:11434/api/tags", false, None, None)?;
        let present = installed["models"].as_array().is_some_and(|models| {
            models.iter().any(|entry| {
                entry["name"].as_str() == Some(model)
                    && entry["size"].as_u64().is_some_and(|size| size > 0)
                    && entry.get("remote_host").is_none()
                    && entry.get("remote_model").is_none()
            })
        });
        if !present {
            return Err(invalid(
                "the selected Ollama model is not installed locally",
            ));
        }
        let response = curl_json(
            "http://127.0.0.1:11434/api/chat",
            false,
            None,
            Some(&json!({
                "model": model, "messages": messages, "stream": false, "think": false, "options": { "num_predict": 384 }
            })),
        )?;
        ("ollama_loopback", response)
    };
    let advice = if remote_url.is_some() {
        response.pointer("/choices/0/message/content")
    } else {
        response.pointer("/message/content")
    }
    .and_then(Value::as_str)
    .filter(|text| !text.trim().is_empty() && text.len() <= 32_768)
    .ok_or_else(|| invalid("AI response did not contain bounded advisory text"))?;
    Ok(AiTriage {
        provider,
        model: model.into(),
        scope: "aggregate_scan_counts_and_formats",
        advice: advice.into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sends_aggregate_counts_without_paths_hashes_or_offsets() -> io::Result<()> {
        let tmp = tempfile::tempdir()?;
        let source = tmp.path().join("private-name.dd");
        std::fs::write(&source, include_bytes!("../desktop/assets/demo.jpg"))?;
        let report = crate::scan(&source)?;
        let data = aggregate(&report).to_string();
        assert!(!data.contains("private-name"));
        assert!(!data.contains(&report.source_sha256));
        assert!(!data.contains("offset"));
        assert_eq!(aggregate(&report)["validated"], 1);
        assert!(ai_triage(&report, "model:cloud", None).is_err());
        assert!(ai_triage(&report, "model", Some("http://example.invalid/")).is_err());
        Ok(())
    }
}
