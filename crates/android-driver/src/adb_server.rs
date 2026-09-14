//! Read-only discovery of an already running local ADB server. Never starts,
//! kills, reconnects or changes an existing server during discovery.
use anyhow::{ensure, Context};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub(crate) async fn roster(port: u16) -> anyhow::Result<String> {
    tokio::time::timeout(Duration::from_millis(600), async {
        let mut socket =
            tokio::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port)).await?;
        let request = b"host:devices-l";
        socket
            .write_all(format!("{:04x}", request.len()).as_bytes())
            .await?;
        socket.write_all(request).await?;
        let mut status = [0; 4];
        socket.read_exact(&mut status).await?;
        ensure!(&status == b"OKAY", "ADB roster refused");
        let mut length = [0; 4];
        socket.read_exact(&mut length).await?;
        let length = usize::from_str_radix(std::str::from_utf8(&length)?, 16)?;
        ensure!(length <= 65535, "ADB roster exceeds protocol limit");
        let mut data = vec![0; length];
        socket.read_exact(&mut data).await?;
        String::from_utf8(data).context("ADB roster is not UTF-8")
    })
    .await
    .context("ADB discovery timed out")?
}

pub(crate) fn explicit_endpoint() -> bool {
    [
        "RIVIU_ADB_SERVER_PORT",
        "ADB_SERVER_SOCKET",
        "ANDROID_ADB_SERVER_PORT",
        "ANDROID_ADB_SERVER_ADDRESS",
    ]
    .iter()
    .any(|name| std::env::var(name).is_ok_and(|v| !v.trim().is_empty()))
}

pub(crate) fn merge_rosters(
    primary: &str,
    alternate: &str,
    previous: &std::collections::HashMap<String, u16>,
) -> (
    Vec<crate::adb::AdbDeviceLine>,
    std::collections::HashMap<String, u16>,
) {
    let mut selected = std::collections::BTreeMap::new();
    for (port, source) in [(5037, primary), (5038, alternate)] {
        for device in crate::adb::parse_devices(&format!("List of devices attached\n{source}")) {
            if !selected.contains_key(&device.serial) || previous.get(&device.serial) == Some(&port)
            {
                selected.insert(device.serial.clone(), (device, port));
            }
        }
    }
    let mut routes = previous.clone();
    let mut devices = Vec::new();
    for (serial, (device, port)) in selected {
        routes.insert(serial, port);
        devices.push(device);
    }
    (devices, routes)
}

fn populated(roster: Option<&str>) -> bool {
    roster.is_some_and(|text| {
        text.lines().any(|line| {
            line.split_whitespace().nth(1).is_some_and(|state| {
                matches!(
                    state,
                    "device" | "offline" | "unauthorized" | "recovery" | "sideload"
                )
            })
        })
    })
}

fn select_port(primary: Option<&str>, alternate: Option<&str>) -> u16 {
    if !populated(primary) && populated(alternate) {
        5038
    } else {
        5037
    }
}

/// Respect explicit endpoint settings. Otherwise retain the standard server if
/// it owns any transports, or select the populated companion server on 5038.
/// Pin the result for the lifetime of the driver so sessions and forwards agree.
pub async fn discover() -> anyhow::Result<Option<u16>> {
    if let Ok(value) = std::env::var("RIVIU_ADB_SERVER_PORT") {
        let port = value
            .trim()
            .parse::<u16>()
            .context("RIVIU_ADB_SERVER_PORT must be 1-65535")?;
        ensure!(port > 0, "RIVIU_ADB_SERVER_PORT must be 1-65535");
        return Ok(Some(port));
    }
    if [
        "ADB_SERVER_SOCKET",
        "ANDROID_ADB_SERVER_PORT",
        "ANDROID_ADB_SERVER_ADDRESS",
    ]
    .iter()
    .any(|name| std::env::var(name).is_ok_and(|value| !value.trim().is_empty()))
    {
        return Ok(None);
    }
    let (primary, alternate) = tokio::join!(roster(5037), roster(5038));
    let port = select_port(primary.as_deref().ok(), alternate.as_deref().ok());
    tracing::info!(
        port,
        "ADB server selected from existing transport inventory"
    );
    Ok(Some(port))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn merged_inventory_keeps_both_servers_and_stable_duplicate_routes() {
        let primary = "phone-a device model:A\nphone-b device model:B\n";
        let alternate = "phone-c device model:C\nphone-b device model:B\n";
        let prior = std::collections::HashMap::from([("phone-b".into(), 5038)]);
        let (devices, routes) = merge_rosters(primary, alternate, &prior);
        assert_eq!(devices.len(), 3);
        assert_eq!(routes["phone-a"], 5037);
        assert_eq!(routes["phone-b"], 5038);
        assert_eq!(routes["phone-c"], 5038);
        let (devices, routes) = merge_rosters("phone-b device model:B\n", "", &routes);
        assert_eq!(devices.len(), 1);
        assert_eq!(routes["phone-b"], 5037);
        assert_eq!(
            routes["phone-c"], 5038,
            "remember disconnected routes for owned cleanup"
        );
    }
    #[test]
    fn uses_companion_server_only_when_primary_has_no_transports() {
        assert_eq!(
            select_port(Some(""), Some("phone device model:SM_G955F\n")),
            5038
        );
        assert_eq!(select_port(None, Some("phone device\n")), 5038);
        assert_eq!(
            select_port(Some("phone unauthorized\n"), Some("other device\n")),
            5037
        );
        assert_eq!(
            select_port(Some("phone offline\n"), Some("other device\n")),
            5037
        );
        assert_eq!(select_port(Some(""), Some("")), 5037);
        assert_eq!(select_port(None, None), 5037);
    }

    #[test]
    fn device_and_long_lived_commands_share_the_selected_endpoint() {
        let adb = crate::AdbProgram::at("fixture-adb.exe".into()).with_server_port(Some(5038));
        let mut command = tokio::process::Command::new(adb.path());
        adb.apply_server(&mut command);
        command.args(["-s", "phone", "shell", "am", "instrument"]);
        let args: Vec<_> = command
            .as_std()
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert_eq!(&args[..4], ["-P", "5038", "-s", "phone"]);
        let port = command
            .as_std()
            .get_envs()
            .find(|(name, _)| *name == "ANDROID_ADB_SERVER_PORT")
            .and_then(|(_, value)| value);
        assert_eq!(port, Some(std::ffi::OsStr::new("5038")));
    }
}
