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

/// What happened to an ADB server Riviu was using, from Riviu's own roster reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdbServerChange {
    /// A server that carried phones stopped answering. Riviu never runs `kill-server`
    /// (contracts.md), so the cause is outside this process: another tool's adb of a
    /// different build replaced it, or the server crashed.
    Lost,
    /// The port answers again after [`Self::Lost`]. Transports and forwards made through
    /// the old server are gone; scrcpy/minicap readers and agents must reconnect.
    Returned,
}

/// One operator-facing notice. Text is Vietnamese because it is shown as-is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdbServerNotice {
    pub port: u16,
    pub change: AdbServerChange,
    /// Phones the server carried at the last good read before it went away.
    pub transports: usize,
    /// Only for [`AdbServerChange::Returned`]: how long the port was silent.
    pub down_for: Option<std::time::Duration>,
    pub message: String,
}

#[derive(Debug, Clone, Copy)]
enum PortSeen {
    Up { transports: usize },
    /// One missed read. `roster` gives up after 600 ms, and a loaded server can be that
    /// slow once; a single miss is not called an outage.
    Suspect { since: std::time::Instant, transports: usize },
    Down { since: std::time::Instant, transports: usize },
}

/// Bounded so a flapping server cannot grow memory; the oldest notice goes first.
const MAX_PENDING_NOTICES: usize = 16;

/// How long background stream starts hold off after a populated server goes away.
///
/// A cap, not a measurement. Every adb client spawned while no server listens runs
/// `daemon not running; starting now` with Riviu's build and races the tool that is
/// restarting it (08/10/2026: 30 phones' view starts did exactly that for ~70 s). But the
/// hold must end: if that tool killed the server and exited, the next adb call is the only
/// thing that will ever bring it back, so after this window the keeper's own per-phone
/// backoff takes over.
pub const OUTAGE_START_HOLD: std::time::Duration = std::time::Duration::from_secs(15);

/// Read-only watch over the servers [`crate::AdbProgram::inventory_read`] already polls.
///
/// It only interprets reads that happen anyway, so it adds no adb traffic and can never
/// start, stop or reconnect anything. A port that never carried a phone is not reported:
/// a missing companion server on 5038 is the normal state of most machines.
#[derive(Debug, Default)]
pub struct ServerWatch {
    ports: std::collections::HashMap<u16, PortSeen>,
    pending: std::collections::VecDeque<AdbServerNotice>,
}

impl ServerWatch {
    pub fn observe(&mut self, port: u16, roster: Option<&str>, now: std::time::Instant) {
        let previous = self.ports.get(&port).copied();
        let next = match (roster, previous) {
            (Some(text), Some(PortSeen::Down { since, transports })) => {
                self.push(AdbServerNotice {
                    port,
                    change: AdbServerChange::Returned,
                    transports,
                    down_for: Some(now.saturating_duration_since(since)),
                    message: format!(
                        "ADB server cổng {port} đã chạy lại sau {} giây. Riviu đang mở lại \
                         stream và phiên điều khiển; thao tác đang dở trên máy chưa được xác \
                         nhận sẽ không tự gửi lại.",
                        now.saturating_duration_since(since).as_secs()
                    ),
                });
                PortSeen::Up { transports: count_transports(text) }
            }
            (Some(text), _) => PortSeen::Up { transports: count_transports(text) },
            (None, Some(PortSeen::Up { transports })) if transports > 0 => {
                PortSeen::Suspect { since: now, transports }
            }
            (None, Some(PortSeen::Suspect { since, transports })) => {
                self.push(AdbServerNotice {
                    port,
                    change: AdbServerChange::Lost,
                    transports,
                    down_for: None,
                    message: format!(
                        "Phần mềm khác vừa khởi động lại hoặc tắt ADB (cổng {port}) khi Riviu \
                         đang dùng {transports} máy. Riviu không tự tắt ADB; stream và phiên \
                         điều khiển sẽ tự nối lại khi ADB chạy lại. Nếu đang mở GenFarmer, \
                         scrcpy hoặc Android Studio, hãy đóng chúng hoặc cho Riviu dùng cổng \
                         ADB riêng (RIVIU_ADB_SERVER_PORT)."
                    ),
                });
                PortSeen::Down { since, transports }
            }
            (None, Some(down @ PortSeen::Down { .. })) => down,
            (None, _) => return,
        };
        self.ports.insert(port, next);
    }

    /// True for at most [`OUTAGE_START_HOLD`] after a populated server went away.
    pub fn holding_starts(&self, now: std::time::Instant) -> bool {
        self.ports.values().any(|seen| match seen {
            PortSeen::Down { since, .. } => now.saturating_duration_since(*since) < OUTAGE_START_HOLD,
            PortSeen::Up { .. } | PortSeen::Suspect { .. } => false,
        })
    }

    pub fn drain(&mut self) -> Vec<AdbServerNotice> {
        self.pending.drain(..).collect()
    }

    fn push(&mut self, notice: AdbServerNotice) {
        if self.pending.len() == MAX_PENDING_NOTICES {
            self.pending.pop_front();
        }
        self.pending.push_back(notice);
    }
}

fn count_transports(roster: &str) -> usize {
    roster
        .lines()
        .filter(|line| line.split_whitespace().nth(1).is_some())
        .count()
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
    fn a_foreign_restart_of_a_populated_server_is_reported_once_each_way() {
        // Shape of 08/10/2026 15:51:53Z: GenFarmer's own adb replaced the 5037 server while
        // Riviu streamed 30 phones; 5038 never existed on that host.
        let start = std::time::Instant::now();
        let mut watch = ServerWatch::default();
        let thirty: String = (0..30).map(|n| format!("ce{n:016} device model:SM_G955F\n")).collect();
        watch.observe(5037, Some(&thirty), start);
        watch.observe(5038, None, start);
        assert!(watch.drain().is_empty(), "a healthy read and an absent companion say nothing");

        watch.observe(5037, None, start + std::time::Duration::from_secs(2));
        watch.observe(5037, None, start + std::time::Duration::from_secs(4));
        assert!(watch.holding_starts(start + std::time::Duration::from_secs(4)));
        assert!(
            !watch.holding_starts(start + std::time::Duration::from_secs(2) + OUTAGE_START_HOLD),
            "the hold is finite even if nothing ever restarts the server"
        );
        let lost = watch.drain();
        assert_eq!(lost.len(), 1, "one notice per loss, not per poll: {lost:?}");
        assert_eq!(lost[0].change, AdbServerChange::Lost);
        assert_eq!((lost[0].port, lost[0].transports), (5037, 30));
        assert!(lost[0].message.starts_with("Phần mềm khác vừa khởi động lại hoặc tắt ADB (cổng 5037)"));

        watch.observe(5037, Some(""), start + std::time::Duration::from_secs(70));
        assert!(
            !watch.holding_starts(start + std::time::Duration::from_secs(70)),
            "answering again ends the outage even before phones re-attach"
        );
        let back = watch.drain();
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].change, AdbServerChange::Returned);
        assert_eq!(back[0].down_for, Some(std::time::Duration::from_secs(68)));
        assert!(watch.drain().is_empty());
    }

    #[test]
    fn one_slow_read_is_not_an_outage() {
        let now = std::time::Instant::now();
        let mut watch = ServerWatch::default();
        watch.observe(5037, Some("phone device\n"), now);
        watch.observe(5037, None, now);
        assert!(!watch.holding_starts(now));
        watch.observe(5037, Some("phone device\n"), now);
        assert!(watch.drain().is_empty(), "a single 600 ms roster timeout says nothing");
    }

    #[test]
    fn an_empty_server_going_away_is_not_an_outage() {
        let now = std::time::Instant::now();
        let mut watch = ServerWatch::default();
        watch.observe(5038, Some(""), now);
        watch.observe(5038, None, now);
        assert!(!watch.holding_starts(now));
        assert!(watch.drain().is_empty());
    }

    #[test]
    fn a_flapping_server_cannot_grow_the_notice_queue() {
        let now = std::time::Instant::now();
        let mut watch = ServerWatch::default();
        for _ in 0..100 {
            watch.observe(5037, Some("phone device\n"), now);
            watch.observe(5037, None, now);
            watch.observe(5037, None, now);
        }
        assert_eq!(watch.drain().len(), MAX_PENDING_NOTICES);
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
