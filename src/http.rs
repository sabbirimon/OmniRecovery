use crate::source::invalid;
use serde_json::Value;
use std::fs::File;
use std::io::{self, Read, Seek, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub(crate) fn curl_json(
    url: &str,
    remote: bool,
    token: Option<&str>,
    body: Option<&Value>,
) -> io::Result<Value> {
    let mut output = tempfile::tempfile()?;
    curl_transfer(url, remote, token, body, &output, 1_048_576)?;
    output.rewind()?;
    Ok(serde_json::from_reader(output.take(1_048_576))?)
}

pub(crate) fn curl_transfer(
    url: &str,
    remote: bool,
    token: Option<&str>,
    body: Option<&Value>,
    output: &File,
    limit: u64,
) -> io::Result<()> {
    if token.is_some_and(|token| {
        token.is_empty()
            || token.len() > 2048
            || !token.bytes().all(|b| {
                b.is_ascii_alphanumeric()
                    || matches!(b, b'.' | b'-' | b'_' | b'~' | b'+' | b'/' | b'=')
            })
    }) {
        return Err(invalid("invalid provider access token format"));
    }
    let limit_arg = limit.to_string();
    let mut body_file = tempfile::NamedTempFile::new()?;
    let mut command = Command::new(crate::acquisition::find_helper("curl")?);
    command.args([
        "--disable",
        "--silent",
        "--fail",
        "--noproxy",
        "*",
        "--max-time",
        "120",
        "--max-filesize",
        &limit_arg,
        "--proto",
        if remote { "=https" } else { "=http" },
        "--globoff",
        "--url",
        url,
    ]);
    if let Some(body) = body {
        serde_json::to_writer(&mut body_file, body)?;
        body_file.flush()?;
        let body_path = format!("@{}", body_file.path().display());
        command.args([
            "--header",
            "Content-Type: application/json",
            "--data-binary",
            &body_path,
        ]);
    }
    if token.is_some() {
        command.args(["--config", "-"]);
    }
    let mut process = command
        .env_remove("OMNIRECOVERY_AI_TOKEN")
        .env_remove("SSLKEYLOGFILE")
        .stdin(if token.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stderr(Stdio::null())
        .stdout(Stdio::from(output.try_clone()?))
        .spawn()?;
    if let Some(token) = token {
        let write_result = process
            .stdin
            .take()
            .ok_or_else(|| io::Error::other("could not open provider input"))
            .and_then(|mut input| writeln!(input, "header = \"Authorization: Bearer {token}\""));
        if let Err(error) = write_result {
            let _ = process.kill();
            let _ = process.wait();
            return Err(error);
        }
    }
    let started = Instant::now();
    loop {
        match process.try_wait() {
            Ok(Some(status)) if status.success() => break,
            Ok(Some(_)) => return Err(invalid("HTTPS provider request failed")),
            Ok(None)
                if started.elapsed() < Duration::from_secs(125)
                    && output.metadata().is_ok_and(|state| state.len() <= limit) =>
            {
                std::thread::sleep(Duration::from_millis(50));
            }
            _ => {
                let _ = process.kill();
                let _ = process.wait();
                return Err(invalid(
                    "HTTPS provider response exceeded a time or size limit",
                ));
            }
        }
    }
    if output.metadata()?.len() > limit {
        return Err(invalid("provider response exceeded its size limit"));
    }
    Ok(())
}
