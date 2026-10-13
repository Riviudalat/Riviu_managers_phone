//! Discovery of the local ADB server, and its bounded recovery.
//!
//! Discovery is read-only. Since 14/10/2026 (owner decision, HX-ADB) Riviu may restart a
//! server that has stopped answering, but only through [`ServerWatch::begin_restart`]: a hang
//! seen on several phones, confirmed by a roster probe, outside a cooldown and an hourly cap.
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
    /// A server that carried phones stopped answering while Riviu was not restarting it:
    /// another tool's adb of a different build replaced it, or the server crashed.
    Lost,
    /// The port answers again after [`Self::Lost`]. Transports and forwards made through
    /// the old server are gone; scrcpy/minicap readers and agents must reconnect.
    Returned,
    /// Riviu itself restarted a hung server ([`ServerWatch::begin_restart`]). Same
    /// consequences as [`Self::Returned`]; never reported as another tool's restart.
    Restarted,
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

/// Watch over the servers [`crate::AdbProgram::inventory_read`] already polls, plus the
/// host-side failures of ordinary adb calls.
///
/// It only interprets what happens anyway, so it adds no adb traffic and itself starts,
/// stops or reconnects nothing; the restart it budgets is run by
/// [`crate::AdbProgram::restart_server`]. A port that never carried a phone is not
/// reported: a missing companion server on 5038 is the normal state of most machines.
#[derive(Debug, Default)]
pub struct ServerWatch {
    ports: std::collections::HashMap<u16, PortSeen>,
    pending: std::collections::VecDeque<AdbServerNotice>,
    hang: HangDetector,
    restarts: RestartBudget,
    /// When Riviu began restarting a server. Inventory reads during it are not an outage.
    restarting: Option<std::time::Instant>,
}

/// Host-side adb failures: the client got no answer from its server, or the call outlived
/// its deadline. Phone-side answers (offline, unauthorized, not found) are not counted.
pub(crate) fn host_side_fault(message: &str) -> bool {
    let text = message.to_ascii_lowercase();
    [
        "could not read ok from adb server",
        "cannot connect to daemon",
        "daemon not running",
        "failed to start daemon",
        "timed out after",
    ]
    .iter()
    .any(|marker| text.contains(marker))
}

/// A server is suspected hung once this many **different** phones have had a host-side
/// failure inside [`HANG_WINDOW`]. One slow or offline phone fails on its own serial only.
///
/// Caps from r107 (14/10/2026, 30 phones on GenFarmer's adb 34.0.5 server): five phones hit
/// their observation deadline within 8 s and four within the same second, next to
/// `adb shell timed out after 2s` and two `could not read ok from ADB Server`; minutes later
/// `adb shell echo` answered in 230–290 ms again. The suspicion alone never restarts
/// anything: a roster probe must also go unanswered.
pub const HANG_MIN_PHONES: usize = 3;
pub const HANG_WINDOW: std::time::Duration = std::time::Duration::from_secs(20);
/// Bounds memory when every phone fails at once; far above [`HANG_MIN_PHONES`].
const HANG_MAX_SAMPLES: usize = 64;

/// Minimum spacing between two restarts. GenFarmer's `AdbHangRecoveryService` uses the same
/// `MIN_MS_BETWEEN_KILLS = 45000`; a server needs that long to re-enumerate 30 phones.
pub const RESTART_COOLDOWN: std::time::Duration = std::time::Duration::from_secs(45);
/// Restarts allowed in any rolling hour. A cap, not a measurement: past it the host has a
/// problem a restart does not fix, and the operator is told instead.
pub const MAX_RESTARTS_PER_HOUR: usize = 4;
const RESTART_BUDGET_WINDOW: std::time::Duration = std::time::Duration::from_secs(3600);
/// A restart that never reported its end stops counting as in progress after this long, so
/// a cancelled recovery cannot hold background starts forever.
const RESTART_MAX_DURATION: std::time::Duration = std::time::Duration::from_secs(90);

#[derive(Debug, Default)]
struct HangDetector {
    failures: std::collections::VecDeque<(std::time::Instant, String)>,
}

impl HangDetector {
    fn record(&mut self, serial: &str, now: std::time::Instant) {
        self.prune(now);
        if self.failures.len() == HANG_MAX_SAMPLES {
            self.failures.pop_front();
        }
        self.failures.push_back((now, serial.to_string()));
    }

    fn prune(&mut self, now: std::time::Instant) {
        while self
            .failures
            .front()
            .is_some_and(|(at, _)| now.saturating_duration_since(*at) > HANG_WINDOW)
        {
            self.failures.pop_front();
        }
    }

    fn phones(&mut self, now: std::time::Instant) -> usize {
        self.prune(now);
        self.failures
            .iter()
            .map(|(_, serial)| serial.as_str())
            .collect::<std::collections::HashSet<_>>()
            .len()
    }
}

#[derive(Debug, Default)]
struct RestartBudget {
    history: std::collections::VecDeque<std::time::Instant>,
}

/// Why a suspected hang did not lead to a restart now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestartRefusal {
    Cooldown { remaining: std::time::Duration },
    HourlyCap,
    InProgress,
}

impl RestartRefusal {
    pub fn message(self) -> String {
        match self {
            Self::Cooldown { remaining } => format!(
                "ADB vừa được khởi động lại; chờ thêm {} giây trước lần sau",
                remaining.as_secs().max(1)
            ),
            Self::HourlyCap => format!(
                "đã khởi động lại ADB {MAX_RESTARTS_PER_HOUR} lần trong một giờ; Riviu không \
                 tự khởi động lại nữa. Kiểm tra cáp/hub USB và phần mềm khác đang dùng ADB"
            ),
            Self::InProgress => "đang khởi động lại ADB".to_string(),
        }
    }
}

impl RestartBudget {
    fn refusal(&mut self, now: std::time::Instant) -> Option<RestartRefusal> {
        while self
            .history
            .front()
            .is_some_and(|at| now.saturating_duration_since(*at) >= RESTART_BUDGET_WINDOW)
        {
            self.history.pop_front();
        }
        if let Some(last) = self.history.back() {
            let since = now.saturating_duration_since(*last);
            if since < RESTART_COOLDOWN {
                return Some(RestartRefusal::Cooldown {
                    remaining: RESTART_COOLDOWN - since,
                });
            }
        }
        (self.history.len() >= MAX_RESTARTS_PER_HOUR).then_some(RestartRefusal::HourlyCap)
    }
}

impl ServerWatch {
    /// One host-side adb failure on `serial`. See [`host_side_fault`].
    pub fn note_host_fault(&mut self, serial: &str, now: std::time::Instant) {
        if !self.restart_in_progress(now) {
            self.hang.record(serial, now);
        }
    }

    /// Enough different phones failed host-side inside [`HANG_WINDOW`] to probe the server.
    pub fn hang_suspected(&mut self, now: std::time::Instant) -> bool {
        !self.restart_in_progress(now) && self.hang.phones(now) >= HANG_MIN_PHONES
    }

    /// The phones behind the current suspicion, for the journal and the port choice.
    pub fn hang_serials(&mut self) -> Vec<String> {
        let mut serials: Vec<String> = self
            .hang
            .failures
            .iter()
            .map(|(_, serial)| serial.clone())
            .collect();
        serials.sort();
        serials.dedup();
        serials
    }

    /// Why a restart could not begin now, without claiming the budget.
    pub fn restart_refusal(&mut self, now: std::time::Instant) -> Option<RestartRefusal> {
        if self.restart_in_progress(now) {
            return Some(RestartRefusal::InProgress);
        }
        self.restarts.refusal(now)
    }

    /// The probe answered: the failures were the phones', not the server's.
    pub fn clear_hang(&mut self) {
        self.hang.failures.clear();
    }

    /// Claim the restart budget. On `Ok` the caller must run the restart and then call
    /// [`Self::end_restart`]; until then inventory gaps are not reported as an outage.
    pub fn begin_restart(&mut self, now: std::time::Instant) -> Result<(), RestartRefusal> {
        if self.restart_in_progress(now) {
            return Err(RestartRefusal::InProgress);
        }
        if let Some(refusal) = self.restarts.refusal(now) {
            return Err(refusal);
        }
        self.restarts.history.push_back(now);
        self.restarting = Some(now);
        Ok(())
    }

    /// Close a restart begun by [`Self::begin_restart`] and queue its operator notice.
    pub fn end_restart(
        &mut self,
        port: u16,
        roster: Option<&str>,
        message: String,
        now: std::time::Instant,
    ) {
        let started = self.restarting.take().unwrap_or(now);
        self.hang.failures.clear();
        let transports = match self.ports.get(&port) {
            Some(PortSeen::Up { transports })
            | Some(PortSeen::Suspect { transports, .. })
            | Some(PortSeen::Down { transports, .. }) => *transports,
            None => 0,
        };
        match roster {
            Some(text) => {
                self.ports.insert(
                    port,
                    PortSeen::Up {
                        transports: count_transports(text),
                    },
                );
            }
            None => {
                self.ports.insert(
                    port,
                    PortSeen::Down {
                        since: now,
                        transports,
                    },
                );
            }
        }
        self.push(AdbServerNotice {
            port,
            change: AdbServerChange::Restarted,
            transports,
            down_for: Some(now.saturating_duration_since(started)),
            message,
        });
    }

    fn restart_in_progress(&self, now: std::time::Instant) -> bool {
        self.restarting
            .is_some_and(|since| now.saturating_duration_since(since) < RESTART_MAX_DURATION)
    }

    pub fn observe(&mut self, port: u16, roster: Option<&str>, now: std::time::Instant) {
        if roster.is_none() && self.restart_in_progress(now) {
            // Riviu's own kill-server: not another tool's outage, and end_restart reports it.
            return;
        }
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
    /// Also true while Riviu restarts a server: a client spawned then would auto-start one.
    pub fn holding_starts(&self, now: std::time::Instant) -> bool {
        self.restart_in_progress(now)
            || self.ports.values().any(|seen| match seen {
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

/// The pid listening on `127.0.0.1:<port>` in `netstat -ano -p tcp` output.
pub(crate) fn parse_listening_pid(netstat: &str, port: u16) -> Option<u32> {
    let suffix = format!(":{port}");
    netstat.lines().find_map(|line| {
        let fields: Vec<&str> = line.split_whitespace().collect();
        match fields.as_slice() {
            [proto, local, _, state, pid]
                if proto.eq_ignore_ascii_case("tcp")
                    && local.ends_with(&suffix)
                    && state.eq_ignore_ascii_case("listening") =>
            {
                pid.parse().ok().filter(|pid| *pid > 0)
            }
            _ => None,
        }
    })
}

/// The `Version x.y.z` line of `adb version`, which names the build rather than the protocol.
/// Two builds answer the same `host:version` (41), so only this tells 34.0.5 from 37.0.1.
pub(crate) fn parse_adb_build(version: &str) -> Option<&str> {
    version
        .lines()
        .find_map(|line| line.trim().strip_prefix("Version "))
        .map(str::trim)
        .filter(|build| !build.is_empty())
}

/// Run one host tool with a deadline and return stdout. Not adb: `netstat`, `powershell`,
/// `taskkill`, so it is outside the adb queue.
#[cfg(windows)]
async fn host_tool(program: &str, args: &[&str], timeout: Duration) -> anyhow::Result<String> {
    let mut command = tokio::process::Command::new(program);
    command
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .creation_flags(0x0800_0000);
    let output = tokio::time::timeout(timeout, command.output())
        .await
        .with_context(|| format!("{program} timed out after {timeout:?}"))?
        .with_context(|| format!("run {program}"))?;
    ensure!(
        output.status.success(),
        "{program} failed: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// The process listening on the adb server port, if the host can say.
#[cfg(windows)]
pub(crate) async fn listening_pid(port: u16) -> Option<u32> {
    let netstat = host_tool(
        "netstat.exe",
        &["-ano", "-p", "tcp"],
        Duration::from_secs(10),
    )
    .await
    .ok()?;
    parse_listening_pid(&netstat, port)
}

#[cfg(not(windows))]
pub(crate) async fn listening_pid(_port: u16) -> Option<u32> {
    None
}

/// The executable of `pid`. `Win32_Process.ExecutablePath`, because `tasklist` gives only
/// the image name and every adb is `adb.exe`.
#[cfg(windows)]
pub(crate) async fn process_image(pid: u32) -> Option<std::path::PathBuf> {
    let script =
        format!("(Get-CimInstance Win32_Process -Filter 'ProcessId={pid}').ExecutablePath");
    let path = host_tool(
        "powershell.exe",
        &["-NoProfile", "-NonInteractive", "-Command", &script],
        Duration::from_secs(15),
    )
    .await
    .ok()?;
    let path = path.trim();
    (!path.is_empty()).then(|| std::path::PathBuf::from(path))
}

#[cfg(not(windows))]
pub(crate) async fn process_image(_pid: u32) -> Option<std::path::PathBuf> {
    None
}

/// End a server that ignored `kill-server`. Only the pid that listens on the port.
#[cfg(windows)]
pub(crate) async fn terminate(pid: u32) -> anyhow::Result<()> {
    host_tool(
        "taskkill.exe",
        &["/F", "/PID", &pid.to_string()],
        Duration::from_secs(10),
    )
    .await
    .map(|_| ())
}

#[cfg(not(windows))]
pub(crate) async fn terminate(pid: u32) -> anyhow::Result<()> {
    anyhow::bail!("terminating adb server pid {pid} is only implemented on Windows")
}

/// Roster probes that must all go unanswered before a suspected hang is called one. A loaded
/// server can miss one 600 ms read (see [`PortSeen::Suspect`]); three a second apart cannot.
const HANG_PROBES: usize = 3;
const HANG_PROBE_SPACING: Duration = Duration::from_secs(1);

/// The roster of `port` if any of [`HANG_PROBES`] reads answers, else `None` (hung or gone).
pub(crate) async fn probe(port: u16) -> Option<String> {
    for attempt in 0..HANG_PROBES {
        if attempt > 0 {
            tokio::time::sleep(HANG_PROBE_SPACING).await;
        }
        if let Ok(text) = roster(port).await {
            return Some(text);
        }
    }
    None
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
    fn a_hang_needs_several_phones_inside_the_window_and_restarts_are_budgeted() {
        let start = std::time::Instant::now();
        let at = |seconds: u64| start + std::time::Duration::from_secs(seconds);
        let mut watch = ServerWatch::default();
        for second in 0..10 {
            watch.note_host_fault("slow-phone", at(second));
        }
        watch.note_host_fault("phone-b", at(10));
        assert!(
            !watch.hang_suspected(at(10)),
            "one phone failing repeatedly is that phone"
        );
        watch.note_host_fault("phone-c", at(11));
        assert!(watch.hang_suspected(at(11)));
        assert!(
            !watch.hang_suspected(at(11) + HANG_WINDOW),
            "failures older than the window no longer count"
        );
        assert!(host_side_fault(
            "adb: error: could not read ok from ADB Server"
        ));
        assert!(host_side_fault("adb shell timed out after 2s"));
        assert!(!host_side_fault("adb: device offline"));
        assert!(!host_side_fault("error: device unauthorized."));

        assert_eq!(watch.begin_restart(at(100)), Ok(()));
        assert_eq!(
            watch.begin_restart(at(101)),
            Err(RestartRefusal::InProgress)
        );
        watch.end_restart(5037, Some("phone device\n"), "ok".into(), at(110));
        assert_eq!(watch.drain()[0].change, AdbServerChange::Restarted);
        assert_eq!(
            watch.begin_restart(at(120)),
            Err(RestartRefusal::Cooldown {
                remaining: std::time::Duration::from_secs(25)
            })
        );
        for restart in 1..MAX_RESTARTS_PER_HOUR as u64 {
            let when = at(100) + RESTART_COOLDOWN * restart as u32;
            assert_eq!(watch.begin_restart(when), Ok(()));
            watch.end_restart(5037, None, "failed".into(), when);
        }
        let past_cooldown = at(100) + RESTART_COOLDOWN * MAX_RESTARTS_PER_HOUR as u32;
        assert_eq!(
            watch.begin_restart(past_cooldown),
            Err(RestartRefusal::HourlyCap)
        );
        assert_eq!(
            watch.begin_restart(at(100) + RESTART_BUDGET_WINDOW),
            Ok(()),
            "the cap rolls"
        );
    }

    #[test]
    fn the_server_process_and_build_are_read_from_host_tools() {
        let netstat = "\r\nActive Connections\r\n\r\n  Proto  Local Address          Foreign Address        State           PID\r\n  TCP    127.0.0.1:15037        0.0.0.0:0              LISTENING       4\r\n  TCP    127.0.0.1:5037         127.0.0.1:61000        ESTABLISHED     17576\r\n  TCP    127.0.0.1:5037         0.0.0.0:0              LISTENING       17576\r\n";
        assert_eq!(parse_listening_pid(netstat, 5037), Some(17576));
        assert_eq!(parse_listening_pid(netstat, 5038), None);
        let genfarmer = "Android Debug Bridge version 1.0.41\nVersion 34.0.5-10900879\nInstalled as C:\\Users\\x\\.genfarmer\\adb.exe\n";
        let riviu = "Android Debug Bridge version 1.0.41\r\nVersion 37.0.1-14910828\r\n";
        assert_eq!(parse_adb_build(genfarmer), Some("34.0.5-10900879"));
        assert_eq!(parse_adb_build(riviu), Some("37.0.1-14910828"));
        assert_eq!(parse_adb_build("no version here"), None);
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
