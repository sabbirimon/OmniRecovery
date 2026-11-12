use std::io;
use std::net::SocketAddrV4;
use std::process::Command;
use std::time::Duration;

use crate::DetectedDevice;
use crate::acquisition::{adapter_command, run_adapter, run_adapter_input};
use crate::source::invalid;

pub(crate) fn adb_command() -> io::Result<Command> {
    let mut command = adapter_command("adb")?;
    command.args(["-H", "127.0.0.1", "-P", "5037"]);
    Ok(command)
}

fn local_endpoint(address: &str) -> io::Result<SocketAddrV4> {
    address
        .parse::<SocketAddrV4>()
        .ok()
        .filter(|endpoint| {
            endpoint.port() != 0 && (endpoint.ip().is_private() || endpoint.ip().is_link_local())
        })
        .ok_or_else(|| {
            invalid("enter the phone's local IPv4 address and port shown in Wireless debugging")
        })
}

fn tls_services() -> io::Result<Vec<(String, SocketAddrV4)>> {
    let output = run_adapter(
        adb_command()?.args(["mdns", "services"]),
        Duration::from_secs(12),
    )?;
    Ok(output
        .lines()
        .filter_map(|line| {
            let mut columns = line.split_whitespace();
            let name = columns.next()?;
            let kind = columns.next()?.trim_end_matches('.');
            let endpoint = local_endpoint(columns.next()?).ok()?;
            (kind == "_adb-tls-connect._tcp")
                .then(|| (format!("{name}._adb-tls-connect._tcp"), endpoint))
        })
        .collect())
}

/// Pair via the platform's Wireless debugging protocol using the code shown on the phone.
pub fn pair_android_wifi(address: &str, code: &str) -> io::Result<()> {
    let address = local_endpoint(address)?;
    if code.len() != 6 || !code.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(invalid(
            "enter the six-digit code shown by Android Wireless debugging",
        ));
    }
    let mut command = adb_command()?;
    command.args(["pair", &address.to_string()]);
    let mut input = zeroize::Zeroizing::new(code.as_bytes().to_vec());
    input.push(b'\n');
    let response = run_adapter_input(&mut command, Duration::from_secs(25), Some(&input))?;
    if !response
        .lines()
        .any(|line| line.contains("Successfully paired"))
    {
        return Err(invalid(
            "pairing was not confirmed; check the current code and pairing address",
        ));
    }
    Ok(())
}

/// Connect only to an advertised Wireless debugging TLS service on the local network.
pub fn connect_android_wifi(address: &str) -> io::Result<()> {
    let endpoint = local_endpoint(address)?;
    if !tls_services()?
        .iter()
        .any(|(_, advertised)| *advertised == endpoint)
    {
        return Err(invalid(
            "Android TLS service was not advertised; check Wi-Fi, mDNS and the connection port",
        ));
    }
    let address = endpoint.to_string();
    let output = run_adapter(
        adb_command()?.args(["connect", &address]),
        Duration::from_secs(15),
    )?;
    if !output.lines().any(|line| {
        line == format!("connected to {address}")
            || line == format!("already connected to {address}")
    }) {
        return Err(invalid(
            "wireless connection was not confirmed; pair the device first",
        ));
    }
    Ok(())
}

pub(crate) fn detect_wireless() -> io::Result<Vec<DetectedDevice>> {
    let services = tls_services()?;
    let output = run_adapter(
        adb_command()?.args(["devices", "-l"]),
        Duration::from_secs(12),
    )?;
    Ok(output
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let id = fields.next()?;
            let state = fields.next()?;
            if state != "device"
                || !services.iter().any(|(alias, endpoint)| {
                    alias == id.trim_end_matches('.')
                        || local_endpoint(id).is_ok_and(|selected| selected == *endpoint)
                })
            {
                return None;
            }
            Some(DetectedDevice {
                id: id.into(),
                state: state.into(),
                model: fields
                    .find_map(|field| field.strip_prefix("model:"))
                    .map(str::to_owned),
            })
        })
        .collect())
}
