//! Physical peripherals (Giai đoạn D, xiaowei "外设版" — peripherals edition): **USB relay**
//! power control for the fleet.
//!
//! The peripherals report's headline hardware is a USB relay board used to cut/restore a
//! phone's power or pulse its power button — a *hard* reboot for a phone that adb can no
//! longer reach (exactly the "máy kẹt ở app" case in the nurture notes, where a soft restart
//! does nothing). The common cheap boards (one/two/four/eight channel, CH340 USB-serial) all
//! speak the same 4-byte "LCUS" protocol at 9600 8N1, which is what [`encode_lcus`] builds.
//!
//! Gamepad/HID routing — the report's other half — lives in the frontend (`peripheralMap.ts`
//! with the Web Gamepad API), because the browser reads controllers natively and the actions
//! it maps to (tap/key/macro) are already frontend gestures. Only the relay needs host serial
//! I/O, which is here.
//!
//! The encoding is pure and unit-tested; the serial write is a thin blocking shell run off
//! the async runtime. Real acceptance needs a relay board on the bench — a fleet step.

use std::io::Write;
use std::time::Duration;

use serde::Serialize;
use tauri::State;

use crate::command_error::CommandError;
use crate::state::AppState;

/// Baud/format every LCUS-family board uses.
const RELAY_BAUD: u32 = 9600;
/// Clamp for a pulse hold, so a typo cannot cut a phone's power for an hour or flicker it
/// faster than the hardware can switch.
const PULSE_MIN_MS: u64 = 50;
const PULSE_MAX_MS: u64 = 10_000;

/// LCUS-type 5V USB relay command: `[0xA0, channel, state, checksum]`.
///
/// `state` `0x01` energises the coil (a normally-open contact closes), `0x00` releases it.
/// `channel` is 1-based. The checksum is the low byte of the sum of the first three, which is
/// all these boards validate. Example the datasheets all print: channel 1 on = `A0 01 01 A2`.
pub fn encode_lcus(channel: u8, on: bool) -> [u8; 4] {
    let state: u8 = if on { 0x01 } else { 0x00 };
    let checksum = 0xA0u16
        .wrapping_add(channel as u16)
        .wrapping_add(state as u16) as u8;
    [0xA0, channel, state, checksum]
}

/// A host serial port, for the operator to pick the relay's COM port.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SerialPortInfo {
    pub name: String,
    pub kind: String,
}

/// Enumerate host serial ports. Empty (not an error) when the platform cannot enumerate — the
/// operator can still type a port name.
pub fn list_ports() -> Vec<SerialPortInfo> {
    let Ok(ports) = serialport::available_ports() else {
        return Vec::new();
    };
    ports
        .into_iter()
        .map(|port| SerialPortInfo {
            kind: match port.port_type {
                serialport::SerialPortType::UsbPort(_) => "usb",
                serialport::SerialPortType::BluetoothPort => "bluetooth",
                serialport::SerialPortType::PciPort => "pci",
                serialport::SerialPortType::Unknown => "unknown",
            }
            .to_string(),
            name: port.port_name,
        })
        .collect()
}

/// Open the port, write the frame, flush. Blocking — callers run it off the async runtime.
fn write_frame(port: &str, frame: &[u8]) -> anyhow::Result<()> {
    let mut handle = serialport::new(port, RELAY_BAUD)
        .timeout(Duration::from_millis(500))
        .open()
        .map_err(|error| anyhow::anyhow!("mở cổng {port} thất bại: {error}"))?;
    handle.write_all(frame)?;
    handle.flush()?;
    Ok(())
}

/// List host serial ports (D peripherals). For choosing the relay board's COM port.
#[tauri::command]
pub async fn list_serial_ports(
    state: State<'_, AppState>,
) -> Result<Vec<SerialPortInfo>, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    tokio::task::spawn_blocking(list_ports)
        .await
        .map_err(|error| CommandError::operation(error.to_string()))
}

/// Hold a relay channel on or off (D peripherals). Raw state — for wiring a phone's power line
/// through a normally-open contact and leaving it cut, say.
#[tauri::command]
pub async fn relay_set_channel(
    state: State<'_, AppState>,
    port: String,
    channel: u8,
    on: bool,
) -> Result<(), CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let frame = encode_lcus(channel, on);
    tokio::task::spawn_blocking(move || write_frame(&port, &frame))
        .await
        .map_err(|error| CommandError::operation(error.to_string()))?
        .map_err(|error| CommandError::operation(error.to_string()))
}

/// Pulse a relay channel and return it (D peripherals) — the hard reboot.
///
/// `energize = true` presses (coil on for `hold_ms`, then off): a power button wired to the
/// contact. `energize = false` cuts (coil off, then on): a power line wired through it. The
/// hold is clamped so a typo cannot strand a phone powered off.
#[tauri::command]
pub async fn relay_pulse_channel(
    state: State<'_, AppState>,
    port: String,
    channel: u8,
    hold_ms: u64,
    energize: bool,
) -> Result<(), CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let first = encode_lcus(channel, energize);
    let second = encode_lcus(channel, !energize);
    let hold = hold_ms.clamp(PULSE_MIN_MS, PULSE_MAX_MS);

    let start_port = port.clone();
    tokio::task::spawn_blocking(move || write_frame(&start_port, &first))
        .await
        .map_err(|error| CommandError::operation(error.to_string()))?
        .map_err(|error| CommandError::operation(error.to_string()))?;
    tokio::time::sleep(Duration::from_millis(hold)).await;
    tokio::task::spawn_blocking(move || write_frame(&port, &second))
        .await
        .map_err(|error| CommandError::operation(error.to_string()))?
        .map_err(|error| CommandError::operation(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lcus_matches_the_datasheet_vectors() {
        // The exact frames the LCUS datasheets print.
        assert_eq!(encode_lcus(1, true), [0xA0, 0x01, 0x01, 0xA2]);
        assert_eq!(encode_lcus(1, false), [0xA0, 0x01, 0x00, 0xA1]);
        assert_eq!(encode_lcus(2, true), [0xA0, 0x02, 0x01, 0xA3]);
        assert_eq!(encode_lcus(2, false), [0xA0, 0x02, 0x00, 0xA2]);
    }

    #[test]
    fn checksum_is_the_low_byte_of_the_sum_for_every_channel() {
        for channel in 1..=8u8 {
            for on in [true, false] {
                let frame = encode_lcus(channel, on);
                let sum = frame[0] as u16 + frame[1] as u16 + frame[2] as u16;
                assert_eq!(frame[3], (sum & 0xFF) as u8, "channel {channel} on={on}");
                assert_eq!(frame[0], 0xA0);
                assert_eq!(frame[2], if on { 0x01 } else { 0x00 });
            }
        }
    }

    #[test]
    fn high_channels_do_not_panic_on_the_checksum() {
        // 0xA0 + 0xFF + 0x01 overflows a u8; the sum is taken in u16 first, so this is just a
        // wrapped low byte, not a panic.
        let frame = encode_lcus(0xFF, true);
        assert_eq!(frame, [0xA0, 0xFF, 0x01, 0xA0]);
    }
}
