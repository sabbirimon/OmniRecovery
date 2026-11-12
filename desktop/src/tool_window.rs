use std::process::Command;

use omnirecovery::{Transport, detect_devices};
use serde::Serialize;
use sha2::{Digest, Sha256};
use tauri::{App, Manager, State};

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct PhoneStart {
    transport: Transport,
    id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(super) struct ToolStart {
    view: &'static str,
    scan_source: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    phone: Option<PhoneStart>,
    #[serde(skip)]
    title: &'static str,
}

#[derive(Clone)]
pub(super) struct Startup(Option<ToolStart>);

impl Startup {
    pub(super) fn capture_scope(&self) -> Option<String> {
        let phone = self.0.as_ref()?.phone.as_ref()?;
        let transport = phone_transport(phone.transport).ok()?;
        let mut digest = Sha256::new();
        digest.update(transport);
        digest.update([0]);
        digest.update(&phone.id);
        Some(format!("{:x}", digest.finalize()))
    }

    pub(super) fn check_phone(&self, transport: Transport, id: &str) -> Result<(), &'static str> {
        if let Some(phone) = self.0.as_ref().and_then(|tool| tool.phone.as_ref())
            && (phone.transport != transport || phone.id != id)
        {
            return Err("This workspace is assigned to a different phone.");
        }
        Ok(())
    }
}

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
        phone: None,
        title,
    })
}

fn phone_transport(transport: Transport) -> Result<&'static str, &'static str> {
    match transport {
        Transport::Android => Ok("android"),
        Transport::AndroidWireless => Ok("android_wireless"),
        Transport::Ios => Ok("ios"),
        Transport::Camera => Err("Choose an Android, iPhone or iPad workspace."),
    }
}

fn phone_tool(transport: Transport, id: String) -> Result<ToolStart, &'static str> {
    phone_transport(transport)?;
    if id.is_empty()
        || id.len() > 128
        || !id.as_bytes()[0].is_ascii_alphanumeric()
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
    {
        return Err("Choose a connected phone from the device list.");
    }
    Ok(ToolStart {
        view: "my-devices",
        scan_source: None,
        phone: Some(PhoneStart { transport, id }),
        title: "Phone workspace",
    })
}

pub(super) fn startup() -> Startup {
    let mut args = std::env::args().skip(1);
    let target = match args.next().as_deref() {
        Some("--tool-window") => args
            .next()
            .and_then(|view| tool(&view, args.next().as_deref()).ok()),
        Some("--phone-window") => {
            let transport = match args.next().as_deref() {
                Some("android") => Some(Transport::Android),
                Some("android_wireless") => Some(Transport::AndroidWireless),
                Some("ios") => Some(Transport::Ios),
                _ => None,
            };
            transport
                .zip(args.next())
                .and_then(|(transport, id)| phone_tool(transport, id).ok())
        }
        _ => None,
    };
    Startup(target)
}

pub(super) fn set_title(app: &App, startup: &Startup) -> tauri::Result<()> {
    if let Some(start) = &startup.0
        && let Some(window) = app.get_webview_window("main")
    {
        window.set_title(&format!("OmniRecovery — {}", start.title))?;
    }
    Ok(())
}

#[tauri::command]
pub(super) fn tool_startup(startup: State<'_, Startup>) -> Option<ToolStart> {
    startup.0.clone()
}

fn spawn_window(arguments: &[&str]) -> Result<(), String> {
    let executable = std::env::current_exe().map_err(super::error)?;
    // Each tool keeps its own selections and running jobs, even after the Toolbox closes.
    let mut child = Command::new(executable)
        .args(arguments)
        .spawn()
        .map_err(super::error)?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

#[tauri::command]
pub(super) fn open_tool_window(view: String, scan_source: Option<String>) -> Result<(), String> {
    let target = tool(&view, scan_source.as_deref())?;
    let mut arguments = vec!["--tool-window", target.view];
    arguments.extend(target.scan_source);
    spawn_window(&arguments)
}

#[tauri::command]
pub(super) async fn open_phone_window(transport: Transport, id: String) -> Result<(), String> {
    let target = phone_tool(transport, id)?;
    let phone = target.phone.ok_or("Choose a phone.")?;
    super::work(move || {
        let expected = if phone.transport == Transport::Ios {
            "attached"
        } else {
            "device"
        };
        let count = detect_devices(phone.transport)
            .map_err(super::error)?
            .into_iter()
            .filter(|device| device.id == phone.id && device.state == expected)
            .count();
        if count != 1 {
            return Err("Reconnect the selected phone and approve its connection.".into());
        }
        spawn_window(&[
            "--phone-window",
            phone_transport(phone.transport)?,
            &phone.id,
        ])
    })
    .await
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

    #[test]
    fn phone_workspace_rejects_other_targets_and_accepts_wireless_ids() {
        let target = phone_tool(Transport::AndroidWireless, "192.168.1.5:45454".into()).unwrap();
        let startup = Startup(Some(target));
        assert!(
            startup
                .check_phone(Transport::AndroidWireless, "192.168.1.5:45454")
                .is_ok()
        );
        assert!(
            startup
                .check_phone(Transport::AndroidWireless, "192.168.1.6:45454")
                .is_err()
        );
        assert!(
            startup
                .check_phone(Transport::Android, "192.168.1.5:45454")
                .is_err()
        );
        assert!(phone_tool(Transport::Android, "../path".into()).is_err());
        assert!(phone_tool(Transport::Camera, "camera".into()).is_err());
        let other = Startup(Some(
            phone_tool(Transport::Android, "SECOND-PHONE".into()).unwrap(),
        ));
        assert_ne!(startup.capture_scope(), other.capture_scope());
        assert_eq!(Startup(None).capture_scope(), None);
    }
}
