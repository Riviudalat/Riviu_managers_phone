import { useEffect, useState } from "react";
import { listGroups } from "../../api";
import type { DeviceGroup, DeviceInfo } from "../../types";

/**
 * Who comments, and under what name.
 *
 * Grouped by **how each device reads the screen**, not by brand: that is the property the
 * thread rule depends on, and naming it here is what stops the mixed-reader refusal from
 * looking arbitrary.
 */
export function InteractionActorPicker({
  pixelActors,
  hierarchyActors,
  deviceNumber,
  actors,
  onToggle,
  onReplace,
  handles,
  onHandleChange,
  onHandleBlur,
  mentionText,
  onMentionText,
  mentions,
  mentionActorCount,
}: {
  pixelActors: DeviceInfo[];
  hierarchyActors: DeviceInfo[];
  deviceNumber: Map<string, number>;
  actors: string[];
  onToggle: (udid: string) => void;
  onReplace: (udids: string[]) => void;
  handles: Record<string, string>;
  onHandleChange: (udid: string, value: string) => void;
  onHandleBlur: (udid: string, value: string) => void;
  mentionText: string;
  onMentionText: (value: string) => void;
  mentions: string[];
  mentionActorCount: number;
}) {
  /// Saved device groups, offered as a way to fill the actor list in one go.
  ///
  /// Deliberately not `SelectionStrip`, which every other page uses: that component says
  /// "chưa chọn → sẽ dùng tất cả", and here an empty actor list is refused rather than
  /// meaning everything. Borrowing it would have put a sentence on screen that is false.
  const [groups, setGroups] = useState<DeviceGroup[]>([]);
  useEffect(() => {
    let alive = true;
    listGroups()
      .then((next) => {
        if (alive) setGroups(next);
      })
      // Groups are a shortcut, not a requirement: the tiles still work without them.
      .catch(() => undefined);
    return () => {
      alive = false;
    };
  }, []);

  const choices = [...pixelActors, ...hierarchyActors];

  return (
    <div className="interaction-actors">
      <label className="nu-field interaction-mention">
        <span className="nu-label">
          Tag thêm acc (@handle) — cách nhau bằng dấu cách hoặc phẩy
        </span>
        <input
          type="text"
          placeholder="@ann @bob"
          spellCheck={false}
          value={mentionText}
          onChange={(event) => onMentionText(event.target.value)}
        />
      </label>
      {/* What the tag actually does, and it is not one thing. Measured 24/08/2026: TikTok only
          links a mention that was picked from its own suggestion list, so the Android path
          types the handle as real key events, waits for the list and taps the exact row —
          which produces a real mention. A handle TikTok does not offer stays literal, and the
          iPhone path cannot reach that list at all. Saying "it becomes a link" flatly would be
          a promise the second and third cases break. */}
      {mentions.length > 0 && (
        <p className="hint">
          {`Máy Android sẽ chọn ${mentions.map((m) => `@${m}`).join(" ")} từ danh sách gợi ý của TikTok để thành tag thật; handle nào TikTok không gợi ý ra thì chỉ còn là chữ (không báo cho acc đó). Máy iPhone luôn chỉ chèn chữ. Kết quả ghi lại ở từng dòng trong tab Theo dõi. `}
          {mentionActorCount > 0
            ? `Ngoài ra ${mentionActorCount} acc trong fleet khớp @handle sẽ tự vào post để bình luận.`
            : "Chưa có máy nào trong fleet khớp @handle, nên không máy nào được kéo vào bài."}
        </p>
      )}
      {groups.length > 0 && (
        <label className="nu-field">
          <span className="nu-label">Lấy từ nhóm</span>
          <select
            value=""
            onChange={(event) => {
              const group = groups.find((entry) => entry.id === event.target.value);
              if (!group) return;
              // Intersected with what is actually here: a group remembers udids, and a phone
              // unplugged since would otherwise be selected and then refused at dispatch with
              // nothing on screen explaining why.
              onReplace(
                group.udids.filter((udid) =>
                  choices.some((device) => device.udid === udid),
                ),
              );
            }}
          >
            <option value="">Chọn nhóm…</option>
            {groups.map((group) => (
              <option key={group.id} value={group.id}>
                {group.name} ({group.udids.length})
              </option>
            ))}
          </select>
        </label>
      )}
      {choices.length === 0 && <span className="hint">Chưa có thiết bị</span>}
      {[
        { label: "iPhone (nhận dạng ảnh)", group: pixelActors },
        { label: "Android (đọc cây giao diện)", group: hierarchyActors },
      ]
        .filter((section) => section.group.length > 0)
        .map((section) => (
          <div key={section.label} className="interaction-actor-group">
            <span className="hint">{section.label}</span>
            {section.group.map((device) => {
              const picked = actors.includes(device.udid);
              const name = device.name || device.model || device.udid.slice(0, 8);
              return (
                <div
                  key={device.udid}
                  className={`interaction-actor-tile${picked ? " selected" : ""}`}
                >
                  {/* No visible checkbox: the whole tile is the target and the orange fill is
                      the "chosen" signal. The checkbox is still here, only moved off-screen —
                      it keeps the label clickable and lets the tests (and a screen reader)
                      read the picked state by the device name. */}
                  <label className="tile-pick" title={device.model || device.udid}>
                    <input
                      type="checkbox"
                      className="tile-check"
                      aria-label={name}
                      checked={picked}
                      onChange={() => onToggle(device.udid)}
                    />
                    <span className="tile-num" aria-hidden="true">
                      {deviceNumber.get(device.udid) ?? "?"}
                    </span>
                    <span className="tile-name">{name}</span>
                  </label>
                  {/* The @handle this phone is logged into. Kept next to the phone so an
                      operator sets it once, here, and tagging it later pulls this phone into
                      the post. Blurring saves it to the device meta. */}
                  <input
                    type="text"
                    className="interaction-handle"
                    placeholder="@handle"
                    spellCheck={false}
                    title="Nick TikTok máy này đang đăng nhập — để tag thì máy này tự vào comment"
                    value={handles[device.udid] ?? ""}
                    onChange={(event) => onHandleChange(device.udid, event.target.value)}
                    onBlur={(event) => onHandleBlur(device.udid, event.target.value)}
                  />
                </div>
              );
            })}
          </div>
        ))}
    </div>
  );
}
