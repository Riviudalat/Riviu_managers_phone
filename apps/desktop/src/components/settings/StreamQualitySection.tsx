import { useEffect, useState } from "react";
import { getStreamSettings, setStreamSettings } from "../../api";
import { describeError } from "../../describeError";
import type { StreamSettings } from "../../types";

/// Pinned to `MIN_VIEW_FPS` and `MAX_SETTABLE_VIEW_FPS` on the Rust side by
/// `the_fps_field_offers_exactly_the_range_this_file_clamps_to` in `commands.rs`, which
/// reads these two lines and names the other when one changes.
///
/// Rust clamps regardless — these only stop the field from displaying a number the encoder
/// will never run at while the operator waits to see it take effect.
const MIN_STREAM_FPS = 5;
const MAX_STREAM_FPS = 30;

/// Display only, and it must match `ViewPreset::Tile::max_fps()` in `scrcpy.rs` — the cap
/// is enforced there, not here. It is named in the hint because a field labelled "FPS"
/// that only half the picture obeys is the same disagreement the overlay/encoder mismatch
/// already cost us once.
const TILE_FPS_CEILING = 10;

/**
 * Stream quality and frame rate for the phone grid.
 *
 * The row this panel was missing. `StreamSettings` had been in the Rust command surface the
 * whole time with nothing on the frontend calling it, so quality and frame rate were
 * unreachable — which is also why "they are lost on restart" was only half the story.
 */
export function StreamQualitySection() {
  const [streamSettings, setStreamSettingsState] = useState<StreamSettings | null>(null);
  const [savingStream, setSavingStream] = useState(false);
  const [streamMessage, setStreamMessage] = useState<string | null>(null);

  useEffect(() => {
    getStreamSettings()
      .then(setStreamSettingsState)
      .catch((error) => setStreamMessage(describeError(error)));
  }, []);

  /// Send the whole row, not the one field that changed: the command takes a complete
  /// `StreamSettings` and a partial one would reset the fields it omitted to their defaults.
  const saveStream = async (change: Partial<StreamSettings>) => {
    if (!streamSettings) return;
    setSavingStream(true);
    setStreamMessage(null);
    try {
      // The reply is the clamped value Rust actually stored, so the field shows what took
      // effect rather than what was typed.
      setStreamSettingsState(await setStreamSettings({ ...streamSettings, ...change }));
    } catch (error) {
      setStreamMessage(describeError(error));
    } finally {
      setSavingStream(false);
    }
  };
  return (
    <section className="settings-section">
      <h3>Chất lượng stream</h3>
      <p className="hint">
        Áp cho Android. Lưới và overlay mã hoá riêng — overlay là một máy chiếm cả cửa sổ
        nên để cao hơn được. Đổi xong sẽ khởi động lại các tile đang chạy, mất khoảng một
        giây hình đen mỗi máy.
      </p>
      <p className="hint">
        FPS ở đây là của overlay. Tile trong lưới bị chặn ở {TILE_FPS_CEILING} hình/giây:
        hai mươi tile giải mã cùng một chỗ với overlay, và đo trên dàn máy này thì 24
        hình/giây tốn 135% một nhân CPU, còn 5 hình/giây tốn 85%. Chặn tile lại là để
        máy đang điều khiển được mượt.
      </p>
      <div className="row">
        <label>
          Chất lượng lưới
          <select
            value={streamSettings?.gridQuality ?? "medium"}
            disabled={!streamSettings || savingStream}
            onChange={(event) => {
              void saveStream({
                gridQuality: event.target.value as StreamSettings["gridQuality"],
              });
            }}
          >
            <option value="low">Thấp</option>
            <option value="medium">Vừa</option>
            <option value="high">Cao</option>
            <option value="extra">Rất cao</option>
          </select>
        </label>
        <label>
          Chất lượng overlay
          <select
            value={streamSettings?.focusQuality ?? "high"}
            disabled={!streamSettings || savingStream}
            onChange={(event) => {
              void saveStream({
                focusQuality: event.target.value as StreamSettings["focusQuality"],
              });
            }}
          >
            <option value="low">Thấp</option>
            <option value="medium">Vừa</option>
            <option value="high">Cao</option>
            <option value="extra">Rất cao</option>
          </select>
        </label>
        <label>
          FPS overlay
          <input
            type="number"
            min={MIN_STREAM_FPS}
            max={MAX_STREAM_FPS}
            value={streamSettings?.fps ?? MAX_STREAM_FPS}
            disabled={!streamSettings || savingStream}
            onChange={(event) => {
              const fps = Number(event.target.value);
              if (!Number.isFinite(fps)) return;
              // Clamped here as well as in Rust, so the field cannot show a number the
              // encoder will never run at while the operator waits for it to take effect.
              void saveStream({
                fps: Math.min(Math.max(Math.round(fps), MIN_STREAM_FPS), MAX_STREAM_FPS),
              });
            }}
          />
        </label>
      </div>
      {streamMessage && <p className="error">{streamMessage}</p>}
    </section>
  );
}
