use std::process::Command;

use serde::Serialize;
use tauri::{App, Manager, State};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub(super) struct ToolStart {
    view: &'static str,
    scan_source: Option<&'static str>,
    #[serde(skip)]
    title: &'static str,
}

#[derive(Clone, Copy)]
pub(super) struct Startup(pub Option<ToolStart>);

fn tool(view: &str, source: Option<&str>) -> Result<ToolStart, &'static str> {
    let (view, title) = match view {
        "my-devices" => ("my-devices", "My devices"),
        "devices" => ("devices", "Devices & storage"),
        "scan" => ("scan", "Scan media"),
        "capture" => ("capture", "Capture & clone"),
        "compare" => ("compare", "Compare captures"),
        "records-review" => ("records-review", "Chats & records"),
        "repair" => ("repair", "Repair files"),
        "transfer" => ("transfer", "Phone clone"),
        "service" => ("service", "Device service"),
        "mirror" => ("mirror", "Screen mirror"),
        "forensics" => ("forensics", "Forensics"),
        "security" => ("security", "Security scan"),
        "logs" => ("logs", "Scan logs"),
        "sanitize" => ("sanitize", "Sanitize plan"),
        "automation" => ("automation", "Automation"),
        _ => return Err("Unknown OmniRecovery tool."),
    };
    let scan_source = match (view, source) {
        (_, None) => None,
        ("scan", Some("android_export")) => Some("android_export"),
        ("scan", Some("ios_export")) => Some("ios_export"),
        ("mirror", Some("android")) => Some("android"),
        ("mirror", Some("android_wireless")) => Some("android_wireless"),
        _ => return Err("The selected tool does not support that initial source."),
    };
    Ok(ToolStart {
        view,
        scan_source,
        title,
    })
}

pub(super) fn startup() -> Startup {
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() != Some("--tool-window") {
        return Startup(None);
    }
    let view = args.next();
    let source = args.next();
    let target = view
        .as_deref()
        .and_then(|view| tool(view, source.as_deref()).ok());
    Startup(target)
}

pub(super) fn set_title(app: &App, startup: Startup) -> tauri::Result<()> {
    if let Some(start) = startup.0
        && let Some(window) = app.get_webview_window("main")
    {
        window.set_title(&format!("OmniRecovery — {}", start.title))?;
    }
    Ok(())
}

#[tauri::command]
pub(super) fn tool_startup(startup: State<'_, Startup>) -> Option<ToolStart> {
    startup.0
}

#[tauri::command]
pub(super) fn open_tool_window(view: String, scan_source: Option<String>) -> Result<(), String> {
    let target = tool(&view, scan_source.as_deref())?;
    let executable = std::env::current_exe().map_err(super::error)?;
    let mut command = Command::new(executable);
    command.args(["--tool-window", target.view]);
    if let Some(source) = target.scan_source {
        command.arg(source);
    }
    // Each tool has its own in-memory workspace and can keep working after the
    // Toolbox closes. Only persistent local records are shared by the app.
    let mut child = command.spawn().map_err(super::error)?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_only_known_local_views_and_supported_source_hints() {
        assert_eq!(
            tool("scan", Some("ios_export")).unwrap().scan_source,
            Some("ios_export")
        );
        assert_eq!(tool("repair", None).unwrap().title, "Repair files");
        assert!(tool("https://example.invalid", None).is_err());
        assert!(tool("repair", Some("ios_export")).is_err());
        assert!(tool("scan", Some("../image.dd")).is_err());
    }
}
