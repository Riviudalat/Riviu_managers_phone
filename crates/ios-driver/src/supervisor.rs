//! Per-device ownership of the USB processes: one WDA runner, one control
//! relay, one stream, one session — and a way to find them again after a crash.
//!
//! Live tests found three relays (18100, 18101, 18102), several
//! `riviu_pmd.py wda-proxy` processes and several `tidevice xctest` runs alive
//! at once for a single phone. Every one of them competed for the same usbmux
//! channel, which is the root of the "error sending request" failures.
//!
//! Two mechanisms fix that here:
//!
//! * [`DeviceSlot`] — one async lock per UDID. Relay spawn, runner start,
//!   recycle and app launch all run inside it, so a second job for the same
//!   device queues behind the first instead of racing it.
//! * [`ProcessRegistry`] — a small on-disk record of the child processes this
//!   app owns. A fresh app instance reclaims them on startup, so a crash cannot
//!   leave a relay holding the port forever. Killing is by recorded PID *and*
//!   a command-line check, never by name pattern: `pkill -f tidevice` would take
//!   out a colleague's unrelated session.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::Mutex;
use tokio::process::{Child, Command};

/// The processes we may own for one device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Role {
    /// `riviu_pmd.py wda-proxy` — owns the XCTest runner and the control relay.
    Proxy,
    /// `riviu_pmd.py stream` — reads the device MJPEG port.
    Stream,
}

impl Role {
    fn as_str(&self) -> &'static str {
        match self {
            Role::Proxy => "proxy",
            Role::Stream => "stream",
        }
    }

    fn from_str(s: &str) -> Option<Self> {
        match s {
            "proxy" => Some(Role::Proxy),
            "stream" => Some(Role::Stream),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
struct Record {
    udid: String,
    role: Role,
    pid: u32,
    /// A distinctive substring of the command line, checked before killing so a
    /// recycled PID belonging to something else is never touched.
    fingerprint: String,
}

/// On-disk record of child processes this app started.
#[derive(Clone)]
pub struct ProcessRegistry {
    path: PathBuf,
    live: Arc<Mutex<HashMap<(String, Role), Record>>>,
}

impl ProcessRegistry {
    pub fn new(state_dir: PathBuf) -> Self {
        let _ = std::fs::create_dir_all(&state_dir);
        Self {
            path: state_dir.join("owned-processes.json"),
            live: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn record(&self, udid: &str, role: Role, pid: u32, fingerprint: &str) {
        let rec = Record {
            udid: udid.to_string(),
            role,
            pid,
            fingerprint: fingerprint.to_string(),
        };
        self.live.lock().insert((udid.to_string(), role), rec);
        self.flush();
    }

    pub fn forget(&self, udid: &str, role: Role) {
        self.live.lock().remove(&(udid.to_string(), role));
        self.flush();
    }

    fn flush(&self) {
        let snapshot: Vec<serde_json::Value> = self
            .live
            .lock()
            .values()
            .map(|r| {
                serde_json::json!({
                    "udid": r.udid,
                    "role": r.role.as_str(),
                    "pid": r.pid,
                    "fingerprint": r.fingerprint,
                })
            })
            .collect();
        if let Ok(text) = serde_json::to_string_pretty(&snapshot) {
            let _ = std::fs::write(&self.path, text);
        }
    }

    /// Kill child processes recorded by a previous run of this app that are
    /// still alive. Called once at startup, before any device is touched.
    ///
    /// Returns a description of what was reclaimed, for the log.
    pub async fn reclaim_orphans(&self) -> Vec<String> {
        let Ok(text) = std::fs::read_to_string(&self.path) else {
            return Vec::new();
        };
        let Ok(items) = serde_json::from_str::<Vec<serde_json::Value>>(&text) else {
            return Vec::new();
        };
        let mut reclaimed = Vec::new();
        for item in items {
            let (Some(pid), Some(udid), Some(role), Some(fingerprint)) = (
                item.get("pid").and_then(|v| v.as_u64()),
                item.get("udid").and_then(|v| v.as_str()),
                item.get("role").and_then(|v| v.as_str()).and_then(Role::from_str),
                item.get("fingerprint").and_then(|v| v.as_str()),
            ) else {
                continue;
            };
            let pid = pid as u32;
            if kill_if_matches(pid, fingerprint).await {
                reclaimed.push(format!("{} {} pid={pid}", &udid[..8.min(udid.len())], role.as_str()));
            }
        }
        // Whatever survived is not ours to chase; start from a clean sheet.
        let _ = std::fs::remove_file(&self.path);
        reclaimed
    }
}

/// Kill `pid`, but only after confirming its command line still contains
/// `fingerprint`. PIDs get reused; a stale record must never kill a stranger.
async fn kill_if_matches(pid: u32, fingerprint: &str) -> bool {
    let Ok(out) = Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "command="])
        .output()
        .await
    else {
        return false;
    };
    let cmdline = String::from_utf8_lossy(&out.stdout);
    if cmdline.trim().is_empty() || !cmdline.contains(fingerprint) {
        return false;
    }
    let _ = Command::new("kill")
        .arg(pid.to_string())
        .output()
        .await;
    // Give it a moment to release the port before anyone rebinds.
    tokio::time::sleep(std::time::Duration::from_millis(600)).await;
    true
}

/// A child process this app owns, deregistered from the on-disk record when it
/// is shut down or dropped.
pub struct OwnedChild {
    /// Recorded so the on-disk registry can reclaim this process after a crash.
    #[allow(dead_code)]
    pub pid: u32,
    role: Role,
    udid: String,
    child: Option<Child>,
    registry: ProcessRegistry,
}

impl OwnedChild {
    pub fn adopt(
        registry: &ProcessRegistry,
        udid: &str,
        role: Role,
        child: Child,
        fingerprint: &str,
    ) -> Self {
        let pid = child.id().unwrap_or(0);
        registry.record(udid, role, pid, fingerprint);
        Self {
            pid,
            role,
            udid: udid.to_string(),
            child: Some(child.into()),
            registry: registry.clone(),
        }
    }

    /// Has the process exited on its own?
    pub fn has_exited(&mut self) -> bool {
        match self.child.as_mut() {
            Some(c) => matches!(c.try_wait(), Ok(Some(_)) | Err(_)),
            None => true,
        }
    }

    pub async fn shutdown(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.start_kill();
            let _ = tokio::time::timeout(std::time::Duration::from_secs(5), child.wait()).await;
        }
        self.registry.forget(&self.udid, self.role);
    }
}

impl Drop for OwnedChild {
    fn drop(&mut self) {
        // `kill_on_drop(true)` on the Command handles the process itself; all
        // that is left is to stop claiming ownership of it.
        if let Some(mut child) = self.child.take() {
            let _ = child.start_kill();
        }
        self.registry.forget(&self.udid, self.role);
    }
}

/// Everything the driver owns for one device, behind one lock.
#[derive(Default)]
pub struct DeviceOwned {
    /// Local port forwarded to the device's WDA HTTP port.
    pub wda_port: Option<u16>,
    pub proxy: Option<OwnedChild>,
    pub stream: Option<OwnedChild>,
    /// Next proxy spawn must kill and restart the device-side runner. Set only
    /// after a *confirmed* wedge: a runner that answers `/status` but cannot
    /// gesture. Never set from a health probe alone.
    pub force_restart: bool,
}

/// One lock per device. Every operation that touches the device's USB channels
/// runs inside it, which is what guarantees a single relay per UDID.
#[derive(Default)]
pub struct DeviceSlot {
    pub owned: tokio::sync::Mutex<DeviceOwned>,
}

/// Slots keyed by UDID, created on demand.
#[derive(Clone, Default)]
pub struct SlotMap {
    slots: Arc<Mutex<HashMap<String, Arc<DeviceSlot>>>>,
}

impl SlotMap {
    pub fn get(&self, udid: &str) -> Arc<DeviceSlot> {
        let mut map = self.slots.lock();
        map.entry(udid.to_string())
            .or_insert_with(|| Arc::new(DeviceSlot::default()))
            .clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("riviu-supervisor-test-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn a_slot_is_shared_per_udid_and_distinct_across_udids() {
        let slots = SlotMap::default();
        let a1 = slots.get("udid-a");
        let a2 = slots.get("udid-a");
        let b = slots.get("udid-b");
        assert!(
            Arc::ptr_eq(&a1, &a2),
            "two jobs for one device must contend for the same lock"
        );
        assert!(!Arc::ptr_eq(&a1, &b), "devices must not share a lock");
    }

    #[tokio::test]
    async fn a_second_job_for_one_device_waits_instead_of_racing() {
        let slots = SlotMap::default();
        let slot = slots.get("udid-a");
        let guard = slot.owned.lock().await;

        let second = slots.get("udid-a");
        assert!(
            second.owned.try_lock().is_err(),
            "the second job must queue, not proceed to spawn its own relay"
        );

        // A different device is unaffected.
        let other = slots.get("udid-b");
        assert!(other.owned.try_lock().is_ok());

        drop(guard);
        assert!(slots.get("udid-a").owned.try_lock().is_ok());
    }

    #[test]
    fn the_registry_round_trips_records_to_disk() {
        let dir = temp_dir("roundtrip");
        let reg = ProcessRegistry::new(dir.clone());
        reg.record("udid-a", Role::Proxy, 4242, "wda-proxy --udid udid-a");
        reg.record("udid-a", Role::Stream, 4243, "stream --udid udid-a");

        let text = std::fs::read_to_string(dir.join("owned-processes.json")).unwrap();
        assert!(text.contains("4242"), "{text}");
        assert!(text.contains("4243"), "{text}");

        reg.forget("udid-a", Role::Proxy);
        let text = std::fs::read_to_string(dir.join("owned-processes.json")).unwrap();
        assert!(!text.contains("4242"), "{text}");
        assert!(text.contains("4243"), "{text}");
    }

    /// The fingerprint check is the safety net against killing a stranger that
    /// happens to have inherited a recorded PID.
    #[tokio::test]
    async fn a_pid_whose_command_does_not_match_is_left_alone() {
        // PID 1 is launchd; it must never match our fingerprint, and must
        // certainly not be signalled.
        assert!(!kill_if_matches(1, "riviu_pmd.py wda-proxy --udid zzz").await);
    }

    #[tokio::test]
    async fn reclaim_ignores_records_whose_process_is_gone() {
        let dir = temp_dir("reclaim");
        let reg = ProcessRegistry::new(dir.clone());
        // A PID that cannot be running our sidecar.
        reg.record("udid-a", Role::Proxy, 999_999, "riviu_pmd.py wda-proxy");
        let reclaimed = reg.reclaim_orphans().await;
        assert!(reclaimed.is_empty(), "{reclaimed:?}");
        assert!(
            !dir.join("owned-processes.json").exists(),
            "the record file is cleared once reclaim has run"
        );
    }
}
