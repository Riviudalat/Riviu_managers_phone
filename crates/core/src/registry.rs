use std::collections::{HashSet, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::RwLock;

use crate::events::{AppEvent, EventBus};
use crate::types::{DeviceInfo, DeviceStatus, TileStreamState};

/// Phones that leave the roster within [`MASS_DISCONNECT_WINDOW`] of each other are one box
/// event, not that many phone faults, once they are at least this many and at least
/// [`MASS_DISCONNECT_MIN_SHARE_PERCENT`] of the phones known just before.
///
/// Not measured. The one data point is r125 (15/10/2026 07:04): a 20-phone USB box went from
/// 20 phones to 0 within 8 s, and the grid then read 0/0 for 18 minutes. Three keeps a single
/// unplugged phone, or two swapped by hand, on the ordinary path where the row goes. A wrong
/// call either way is safe: a kept row is display only and authorises nothing.
pub const MASS_DISCONNECT_MIN_PHONES: usize = 3;
/// Not measured. Low on purpose, so that one hub of three phones in a fleet of up to 30 is
/// still one event (the 30% first proposed would need six phones of a 20-phone box). It only
/// bites on larger fleets, where a few unrelated departures, such as reboots, are commoner.
pub const MASS_DISCONNECT_MIN_SHARE_PERCENT: usize = 10;
/// Not measured. Covers a box that drops across consecutive scans (scans run every 3 s, and
/// the r125 scan that saw the drop took 7.7 s) without joining unrelated departures minutes
/// apart.
pub const MASS_DISCONNECT_WINDOW: Duration = Duration::from_secs(20);
/// How long a phone lost in a mass disconnect stays on the grid. Not measured: long enough
/// that an operator who finds a dark box after a night still sees which phones it held,
/// short enough that a retired box is not shown for days. Expiry only bounds memory: a kept
/// row is display only, so its age cannot authorise anything either way.
pub const ABSENT_ROW_TTL: Duration = Duration::from_secs(12 * 3600);
/// A kept or departed row was a live phone, so the TTL already bounds both sets by the phones
/// plugged in within it; this cap only guards against a flapping network transport.
const MAX_KEPT_ROWS: usize = 256;
const MAX_PENDING_MASS_DISCONNECTS: usize = 8;
/// Not measured: a fleet of 50 USB phones lists 50 serials; the cap only guards against a
/// network transport that returns under a new serial each time.
const MAX_LISTED_PHONES: usize = 4096;

/// One mass disconnect, for the activity log. See [`DeviceRegistry::take_mass_disconnects`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MassDisconnect {
    /// The phones lost, sorted.
    pub udids: Vec<String>,
    /// Phones known just before: live rows plus departures inside the window.
    pub known: usize,
}

impl MassDisconnect {
    /// The single operator notice for the event, in Vietnamese because it is shown as is.
    pub fn notice(&self) -> String {
        const LISTED: usize = 30;
        let mut phones = self
            .udids
            .iter()
            .take(LISTED)
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");
        if self.udids.len() > LISTED {
            phones.push_str(", …");
        }
        format!(
            "Mất kết nối {}/{} máy cùng lúc (hub/nguồn USB?). Riviu giữ các máy này trên lưới ở \
             trạng thái mất kết nối và không giao việc cho chúng; cắm lại là máy tự về, không \
             cần bấm gì. Máy: {phones}",
            self.udids.len(),
            self.known
        )
    }
}

struct KeptRow {
    since: Instant,
    row: DeviceInfo,
}

/// The roster behind [`DeviceRegistry`]. Only `live` is a phone anything may act on.
#[derive(Default)]
struct Roster {
    live: Vec<DeviceInfo>,
    /// Phones lost in a mass disconnect, shown as disconnected until they return or expire.
    kept: Vec<KeptRow>,
    /// Ordinary departures inside [`MASS_DISCONNECT_WINDOW`], held back so that a box that
    /// drops across two scans is still one event. Never shown.
    departed: VecDeque<KeptRow>,
    pending: VecDeque<MassDisconnect>,
    /// Every phone a roster of this process listed, in any state. A phone absent since start
    /// never left the roster, so no kept row speaks for it (drop audit schedule_restart#1).
    listed: HashSet<String>,
    /// Set once `listed` reached [`MAX_LISTED_PHONES`]: from then on every phone counts as
    /// listed, which is the behaviour from before the set existed.
    listed_full: bool,
}

impl Roster {
    fn note_listed(&mut self, udid: &str) {
        if self.listed_full || self.listed.contains(udid) {
            return;
        }
        if self.listed.len() >= MAX_LISTED_PHONES {
            self.listed_full = true;
            self.listed.clear();
            return;
        }
        self.listed.insert(udid.to_owned());
    }

    fn display(&self, now: Instant) -> Vec<DeviceInfo> {
        let mut devices = self.live.clone();
        devices.extend(
            self.kept
                .iter()
                .filter(|kept| now.saturating_duration_since(kept.since) < ABSENT_ROW_TTL)
                .filter(|kept| !self.live.iter().any(|live| live.udid == kept.row.udid))
                .map(|kept| kept.row.clone()),
        );
        devices
    }

    fn forget_absent(&mut self, udid: &str) {
        self.kept.retain(|kept| kept.row.udid != udid);
        self.departed.retain(|gone| gone.row.udid != udid);
    }

    /// Replace the live roster with a scan, and keep the phones it lost when they are a box.
    fn replace_live(&mut self, devices: Vec<DeviceInfo>, now: Instant) {
        let previous = std::mem::replace(&mut self.live, devices);
        let present = self
            .live
            .iter()
            .map(|device| device.udid.clone())
            .collect::<HashSet<_>>();
        for udid in &present {
            self.note_listed(udid);
        }
        // A phone that is back replaces its kept row; nothing of the old row survives.
        self.kept.retain(|kept| {
            !present.contains(&kept.row.udid)
                && now.saturating_duration_since(kept.since) < ABSENT_ROW_TTL
        });
        self.departed.retain(|gone| {
            !present.contains(&gone.row.udid)
                && now.saturating_duration_since(gone.since) < MASS_DISCONNECT_WINDOW
        });
        let known = previous.len() + self.departed.len();
        let lost = previous
            .into_iter()
            .filter(|row| !present.contains(&row.udid))
            .collect::<Vec<_>>();
        if lost.is_empty() {
            return;
        }
        let gone = lost.len() + self.departed.len();
        let mass = gone >= MASS_DISCONNECT_MIN_PHONES
            && gone * 100 >= known * MASS_DISCONNECT_MIN_SHARE_PERCENT;
        if !mass {
            // Today's path for a single phone: the row goes. Held back unseen, in case more
            // of its box follows within the window.
            self.departed
                .extend(lost.into_iter().map(|row| KeptRow { since: now, row }));
            while self.departed.len() > MAX_KEPT_ROWS {
                self.departed.pop_front();
            }
            return;
        }
        let rows = self
            .departed
            .drain(..)
            .map(|gone| gone.row)
            .chain(lost)
            .collect::<Vec<_>>();
        // Constant for the life of the row: preparation caches key on `last_error`.
        let reason = format!(
            "Mất kết nối lúc {} cùng {} máy (hub/nguồn USB?); cắm lại là máy tự về",
            chrono::Local::now().format("%H:%M"),
            rows.len()
        );
        let mut udids = Vec::with_capacity(rows.len());
        for row in rows {
            udids.push(row.udid.clone());
            self.kept.retain(|kept| kept.row.udid != row.udid);
            self.kept.push(KeptRow {
                since: now,
                // Nothing of the live row that claims a working phone survives.
                row: DeviceInfo {
                    status: DeviceStatus::Disconnected,
                    battery: None,
                    wda_ready: false,
                    wda_expires_at: None,
                    stream_url: None,
                    tile_stream_state: TileStreamState::Parked,
                    last_error: Some(reason.clone()),
                    ..row
                },
            });
        }
        if self.kept.len() > MAX_KEPT_ROWS {
            let excess = self.kept.len() - MAX_KEPT_ROWS;
            self.kept.drain(..excess);
        }
        udids.sort();
        if self.pending.len() == MAX_PENDING_MASS_DISCONNECTS {
            self.pending.pop_front();
        }
        self.pending.push_back(MassDisconnect { udids, known });
    }
}

/// One fleet scan exactly as the drivers answered it, before any merge or status update.
#[derive(Clone)]
pub struct FleetScan {
    /// When the scan began, so its age is never understated.
    pub started_at: tokio::time::Instant,
    pub devices: Vec<DeviceInfo>,
}

/// The fleet roster.
///
/// [`Self::list`], [`Self::get`] and [`Self::set_status`] see only the phones the last scan
/// found. Phones lost in a mass disconnect stay visible through [`Self::list_with_absent`]
/// and `DevicesUpdated` as disconnected rows, and nothing that picks phones for work or
/// leases ever reads them.
#[derive(Clone)]
pub struct DeviceRegistry {
    devices: Arc<RwLock<Roster>>,
    latest_scan: Arc<RwLock<Option<FleetScan>>>,
    events: EventBus,
    started_at: Instant,
}

impl DeviceRegistry {
    pub fn new(events: EventBus) -> Self {
        Self {
            devices: Arc::new(RwLock::new(Roster::default())),
            latest_scan: Arc::new(RwLock::new(None)),
            events,
            started_at: Instant::now(),
        }
    }

    /// When this roster began, which is when this process started watching phones.
    pub fn started_at(&self) -> Instant {
        self.started_at
    }

    /// Whether any roster of this process listed `udid`, in any state. Never a reason to act
    /// on the phone: it only tells a phone absent since start from one that left.
    pub fn listed_since_start(&self, udid: &str) -> bool {
        let roster = self.devices.read();
        roster.listed_full || roster.listed.contains(udid)
    }

    /// Whether any roster of this process listed a phone at all.
    pub fn listed_any_since_start(&self) -> bool {
        let roster = self.devices.read();
        roster.listed_full || !roster.listed.is_empty()
    }

    /// The live roster: phones the last scan found.
    pub fn list(&self) -> Vec<DeviceInfo> {
        self.devices.read().live.clone()
    }

    /// The grid's roster: [`Self::list`] plus phones lost in a mass disconnect, as
    /// `Disconnected` rows with a reason. Display only; never pick phones for work from it.
    pub fn list_with_absent(&self) -> Vec<DeviceInfo> {
        self.devices.read().display(Instant::now())
    }

    /// When the mass disconnect that took `udid` down was recorded, while its kept row lasts
    /// and the phone is not listed again. Never a reason to act on the phone.
    pub fn mass_disconnect_since(&self, udid: &str) -> Option<Instant> {
        let roster = self.devices.read();
        if roster.live.iter().any(|device| device.udid == udid) {
            return None;
        }
        roster
            .kept
            .iter()
            .find(|kept| kept.row.udid == udid)
            .map(|kept| kept.since)
    }

    /// Mass disconnects since the last call, oldest first, each to be logged once.
    pub fn take_mass_disconnects(&self) -> Vec<MassDisconnect> {
        self.devices.write().pending.drain(..).collect()
    }

    /// Keep the background scan's raw answer for readers that need connection state as the
    /// drivers reported it (the publish dispatcher), not the merged rows shown in the UI.
    pub fn record_scan(&self, scan: FleetScan) {
        *self.latest_scan.write() = Some(scan);
    }

    pub fn latest_scan(&self) -> Option<FleetScan> {
        self.latest_scan.read().clone()
    }

    /// A live phone. A row kept from a mass disconnect is not one.
    pub fn get(&self, udid: &str) -> Option<DeviceInfo> {
        self.devices
            .read()
            .live
            .iter()
            .find(|d| d.udid == udid)
            .cloned()
    }

    /// Replace the live roster with a scan. A phone that left goes, as before, unless it left
    /// with a box (see [`MASS_DISCONNECT_MIN_PHONES`]): then it stays on the grid as a
    /// disconnected row until it returns or [`ABSENT_ROW_TTL`] passes, and the event waits
    /// in [`Self::take_mass_disconnects`].
    pub fn upsert_many(&self, devices: Vec<DeviceInfo>) {
        let now = Instant::now();
        let devices = {
            let mut guard = self.devices.write();
            guard.replace_live(devices, now);
            guard.display(now)
        };
        self.events.emit(AppEvent::DevicesUpdated { devices });
    }

    pub fn upsert(&self, device: DeviceInfo) {
        {
            let mut guard = self.devices.write();
            guard.forget_absent(&device.udid);
            guard.note_listed(&device.udid);
            if let Some(existing) = guard.live.iter_mut().find(|d| d.udid == device.udid) {
                *existing = device.clone();
            } else {
                guard.live.push(device.clone());
            }
        }
        self.events.emit(AppEvent::DeviceUpdated {
            device: device.clone(),
        });
        self.events.emit(AppEvent::DevicesUpdated {
            devices: self.list_with_absent(),
        });
    }

    /// Change one phone's status, and **say so when there is no such phone.**
    ///
    /// Two defects lived in the four lines this replaces, and both were silent.
    ///
    /// **It did nothing for a udid it could not find, with no `else`, no log and no return
    /// value.** That case is reachable on any busy fleet: `upsert_many` replaces the *whole*
    /// roster on every rescan, so a driver task that finishes just after a scan is holding a
    /// udid the roster no longer has. The update vanished, and the phone kept showing whatever
    /// it showed before -- `ready` after it had errored, or `error` after it had recovered.
    /// That is the shape of "the app lists the phone but cannot drive it", read from the wrong
    /// end.
    ///
    /// **And the read-modify-write was not atomic.** It called `get()` (read lock, released),
    /// mutated the clone, then `upsert()` (write lock). Anything happening in between -- another
    /// `set_status`, or a whole-roster swap -- was overwritten by a value computed before it.
    /// Now the find and the mutation happen under one write lock, and only the announcement
    /// happens outside it, because emitting under a lock is how a different bug starts.
    ///
    /// Returns whether it applied. Callers that ignore it are no worse off than before; callers
    /// that care can now ask.
    pub fn set_status(&self, udid: &str, status: DeviceStatus, error: Option<String>) -> bool {
        // Kept for the log line before the value moves into the mutation below.
        let wanted = format!("{status:?}");
        let updated = {
            let mut guard = self.devices.write();
            guard
                .live
                .iter_mut()
                .find(|device| device.udid == udid)
                .map(|device| {
                    device.status = status;
                    device.last_error = error;
                    device.clone()
                })
        };
        let Some(device) = updated else {
            tracing::warn!(
                "dropped a status update for {udid}: not in the roster (status {wanted}). \
                 A rescan replaces the whole roster, so a driver task finishing just after one \
                 can hold a udid that is briefly gone."
            );
            return false;
        };
        self.events.emit(AppEvent::DeviceUpdated {
            device: device.clone(),
        });
        self.events.emit(AppEvent::DevicesUpdated {
            devices: self.list_with_absent(),
        });
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ConnectionKind, DeviceInfo, DevicePlatform, DeviceStatus, TileStreamState};

    fn phone(udid: &str) -> DeviceInfo {
        DeviceInfo {
            udid: udid.to_string(),
            name: format!("phone {udid}"),
            model: "SM-G955F".into(),
            platform: DevicePlatform::Android,
            os_version: "9".into(),
            connection: ConnectionKind::Usb,
            status: DeviceStatus::Ready,
            battery: None,
            wda_ready: false,
            wda_expires_at: None,
            stream_url: None,
            tile_stream_state: TileStreamState::Parked,
            last_error: None,
            wifi_ip: None,
        }
    }

    fn registry() -> DeviceRegistry {
        DeviceRegistry::new(EventBus::new(64))
    }

    #[test]
    fn a_status_change_lands_and_is_announced_twice() {
        let registry = registry();
        registry.upsert_many(vec![phone("a"), phone("b")]);
        let mut events = registry.events.subscribe();

        assert!(registry.set_status("a", DeviceStatus::Error, Some("adb offline".into())));

        let stored = registry.get("a").expect("still in the roster");
        assert_eq!(stored.status, DeviceStatus::Error);
        assert_eq!(stored.last_error.as_deref(), Some("adb offline"));
        // One for the phone, one for the roster: the grid listens to the second.
        assert!(matches!(
            events.try_recv(),
            Ok(AppEvent::DeviceUpdated { .. })
        ));
        assert!(matches!(
            events.try_recv(),
            Ok(AppEvent::DevicesUpdated { .. })
        ));
    }

    /// **The update that used to vanish.**
    ///
    /// No `else`, no log, no return value -- so a status for a phone the roster no longer holds
    /// was simply gone, and the phone kept showing the status it had before.
    #[test]
    fn a_status_for_a_phone_that_is_not_in_the_roster_is_reported_as_a_miss() {
        let registry = registry();
        registry.upsert_many(vec![phone("a")]);

        assert!(
            !registry.set_status("gone", DeviceStatus::Error, None),
            "a udid that is not there has to answer no"
        );
        assert!(registry.get("gone").is_none(), "and must not be invented");
    }

    /// The reachable version of the same thing: a rescan drops a phone, and a driver task that
    /// was already running finishes afterwards holding its udid.
    #[test]
    fn a_rescan_that_drops_a_phone_makes_a_late_status_a_miss_rather_than_a_ghost() {
        let registry = registry();
        registry.upsert_many(vec![phone("a"), phone("b")]);
        // The scan sees only one phone this time -- a cable, or a blip.
        registry.upsert_many(vec![phone("a")]);

        assert!(!registry.set_status("b", DeviceStatus::Error, Some("late".into())));
        assert_eq!(registry.list().len(), 1, "the roster is what the scan said");
    }

    /// Both fields move together. They used to be written to a clone that a concurrent writer
    /// could overwrite whole; now they are written under the lock that finds the row.
    #[test]
    fn a_status_change_clears_a_stale_error_as_well_as_setting_one() {
        let registry = registry();
        registry.upsert_many(vec![phone("a")]);

        registry.set_status("a", DeviceStatus::Error, Some("unauthorized".into()));
        registry.set_status("a", DeviceStatus::Ready, None);

        let stored = registry.get("a").expect("present");
        assert_eq!(stored.status, DeviceStatus::Ready);
        assert_eq!(
            stored.last_error, None,
            "a recovered phone must not keep the sentence that explained its failure"
        );
    }

    #[test]
    fn upsert_replaces_a_known_phone_and_appends_an_unknown_one() {
        let registry = registry();
        registry.upsert_many(vec![phone("a")]);

        let mut renamed = phone("a");
        renamed.name = "renamed".into();
        registry.upsert(renamed);
        assert_eq!(registry.list().len(), 1);
        assert_eq!(registry.get("a").unwrap().name, "renamed");

        registry.upsert(phone("b"));
        assert_eq!(registry.list().len(), 2);
    }
}
