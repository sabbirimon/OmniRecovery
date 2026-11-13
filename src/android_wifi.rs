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

#[cfg(windows)]
mod windows_usb {
    use std::io::{self, Read, Write};
    use std::net::TcpStream;
    use std::time::Duration;

    use prost::Message;

    use crate::DetectedDevice;
    use crate::acquisition::usb_serial;
    use crate::source::invalid;

    // AOSP packages/modules/adb/proto/adb_host.proto. Unused fields are skipped.
    #[derive(Message)]
    struct Devices {
        #[prost(message, repeated, tag = "1")]
        device: Vec<Device>,
    }

    #[derive(Message)]
    struct Device {
        #[prost(string, tag = "1")]
        serial: String,
        #[prost(int32, tag = "2")]
        state: i32,
        #[prost(string, tag = "5")]
        model: String,
        #[prost(int32, tag = "7")]
        connection_type: i32,
    }

    pub(super) fn detect() -> io::Result<Vec<DetectedDevice>> {
        let timeout = Duration::from_secs(2);
        let mut socket = TcpStream::connect_timeout(&([127, 0, 0, 1], 5037).into(), timeout)?;
        socket.set_read_timeout(Some(timeout))?;
        socket.set_write_timeout(Some(timeout))?;
        read_snapshot(&mut socket)
    }

    fn read_snapshot(stream: &mut (impl Read + Write)) -> io::Result<Vec<DetectedDevice>> {
        const REQUEST: &str = "host:track-devices-proto-binary";
        write!(stream, "{:04x}{REQUEST}", REQUEST.len())?;
        let mut header = [0; 4];
        stream.read_exact(&mut header)?;
        if &header != b"OKAY" {
            return Err(invalid("ADB server declined the device inventory request"));
        }
        stream.read_exact(&mut header)?;
        let size = std::str::from_utf8(&header)
            .ok()
            .and_then(|length| usize::from_str_radix(length, 16).ok())
            .ok_or_else(|| invalid("invalid ADB inventory frame length"))?;
        let mut payload = vec![0; size];
        stream.read_exact(&mut payload)?;
        Ok(Devices::decode(payload.as_slice())?
            .device
            .into_iter()
            .filter_map(|device| {
                if device.connection_type != 1
                    || !usb_serial(&device.serial)
                    || device.serial.starts_with("emulator-")
                {
                    return None;
                }
                let state = match device.state {
                    1 => "connecting",
                    2 => "authorizing",
                    3 => "unauthorized",
                    4 => "permission_required",
                    6 => "offline",
                    7 => "bootloader",
                    8 => "device",
                    10 => "recovery",
                    11 => "sideload",
                    _ => return None,
                };
                Some(DetectedDevice {
                    id: device.serial,
                    state: state.into(),
                    model: (!device.model.is_empty()).then_some(device.model),
                })
            })
            .collect())
    }

    #[test]
    fn windows_usb_phone_does_not_need_a_bus_address() -> io::Result<()> {
        let payload = Devices {
            device: vec![
                Device {
                    serial: "REAL-USB".into(),
                    model: "SM-A207F".into(),
                    connection_type: 1,
                    state: 8,
                },
                Device {
                    serial: "NETWORK".into(),
                    connection_type: 2,
                    state: 8,
                    ..Device::default()
                },
                Device {
                    serial: "WAIT-USB".into(),
                    connection_type: 1,
                    state: 3,
                    ..Device::default()
                },
            ],
        }
        .encode_to_vec();
        let mut response = format!("OKAY{:04x}", payload.len()).into_bytes();
        response.extend(payload);
        let mut request = vec![0; 35];
        request.extend(response);
        let devices = read_snapshot(&mut io::Cursor::new(request))?;
        assert_eq!(devices.len(), 2);
        assert_eq!(devices[0].id, "REAL-USB");
        assert_eq!(devices[1].state, "unauthorized");
        Ok(())
    }
}

#[cfg(windows)]
pub(crate) fn detect_windows_usb() -> io::Result<Vec<DetectedDevice>> {
    windows_usb::detect()
}
