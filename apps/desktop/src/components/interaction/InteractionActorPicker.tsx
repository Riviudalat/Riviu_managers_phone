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
  deviceLabel,
  commentEnabled,
  threadsByGroup,
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
  /** The number the operator wrote on the phone — `tileNumber`, same as the wall. */
  deviceNumber: Map<string, number>;
  /** What the operator calls it — `tileName`, so a rename lands here too. */
  deviceLabel: Map<string, string>;
  /** Comment-only metadata stays out of Like/Save-only campaigns. */
  commentEnabled: boolean;
  /**
   * Whether loading a group means anything for the campaign being set up.
   *
   * A group is a set of phones that talk to *each other*: Toả has them all reply to one root
   * comment, Nối tiếp has them reply down the list. `Riêng lẻ` has no thread at all — every
   * phone posts its own root comment and never reads another's — so a group there names a
   * set with nothing to do with the shape, and offering it invites the reading that it does
   * something. Phones are still picked one by one.
   */
  threadsByGroup: boolean;
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
  /// Fetched per mount, so a tab switch re-asks. Left that way deliberately: it is a read of a
  /// small table with no device involved, and a group renamed in Tài khoản while this panel sat
  /// open should show up. Unlike the disclosure state above it, nothing the operator did is
  /// lost by re-reading it.
  ///
  /// Deliberately not `SelectionStrip`, which every other page uses: that component says
  /// "chưa chọn → sẽ dùng tất cả", and here an empty actor list is refused rather than
  /// meaning everything. Borrowing it would have put a sentence on screen that is false.
  const [groups, setGroups] = useState<DeviceGroup[]>([]);
  useEffect(() => {
    if (!commentEnabled) {
      setGroups([]);
      return;
    }
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
  }, [commentEnabled]);

  const choices = [...pixelActors, ...hierarchyActors];

  return (
    <div className="interaction-actors">
      {commentEnabled && (
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
      )}
      {/* What the tag actually does, and it is not one thing. Measured 24/08/2026: TikTok only
          links a mention that was picked from its own suggestion list, so the Android path
          types the handle as real key events, waits for the list and taps the exact row —
          which produces a real mention. A handle TikTok does not offer stays literal, and the
          iPhone path cannot reach that list at all. Saying "it becomes a link" flatly would be
          a promise the second and third cases break. */}
      {commentEnabled && mentions.length > 0 && (
        <p className="hint">
          {`Máy Android sẽ chọn ${mentions.map((m) => `@${m}`).join(" ")} từ danh sách gợi ý của TikTok để thành tag thật; handle nào TikTok không gợi ý ra thì chỉ còn là chữ (không báo cho acc đó). Máy iPhone luôn chỉ chèn chữ. Kết quả ghi lại ở từng dòng trong tab Theo dõi. `}
          {mentionActorCount > 0
            ? `Ngoài ra ${mentionActorCount} acc trong fleet khớp @handle sẽ tự vào post để bình luận.`
            : "Chưa có máy nào trong fleet khớp @handle, nên không máy nào được kéo vào bài."}
        </p>
      )}
      {commentEnabled && threadsByGroup && groups.length > 0 && (
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
              //
              // **Sorted by the operator's number, because this list decides who replies to
              // whom.** `partition_actors` slices it into cohorts in order and a `Chain` has
              // message N reply to message N-1, so the actor order *is* the thread. Loading a
              // group used to hand over whatever order SQLite produced — serial-number order,
              // via a covering index — so "nhóm A gồm máy 1, 4, 2" ran in an order that
              // matched nothing on screen and that nothing explained.
              onReplace(
                group.udids
                  .filter((udid) => choices.some((device) => device.udid === udid))
                  .sort(
                    (a, b) =>
                      (deviceNumber.get(a) ?? Number.MAX_SAFE_INTEGER) -
                      (deviceNumber.get(b) ?? Number.MAX_SAFE_INTEGER),
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
            {/* **Ordered by the operator's number, so a tile is findable.**
                The list arrived in fleet order, which is USB enumeration order and moves when
                a phone drops off. Numbers are the thing that does not move, and they are what
                an operator says out loud — so they also decide the order they are shown in. */}
            {[...section.group]
              .sort(
                (a, b) =>
                  (deviceNumber.get(a.udid) ?? Number.MAX_SAFE_INTEGER) -
                  (deviceNumber.get(b.udid) ?? Number.MAX_SAFE_INTEGER),
              )
              .map((device) => {
              const picked = actors.includes(device.udid);
              // **No model here.** Twenty phones on this fleet report `SM-G950F`, so the model
              // told the operator nothing while taking the width that the name needed. What
              // identifies a phone is the number written on it and whatever they renamed it
              // to; the udid stays in the explicit technical disclosure.
              const name = deviceLabel.get(device.udid) || device.name || "Máy chưa đặt tên";
              return (
                <div
                  key={device.udid}
                  className={`interaction-actor-tile${picked ? " selected" : ""}`}
                >
                  {/* No visible checkbox: the whole tile is the target and the orange fill is
                      the "chosen" signal. The checkbox is still here, only moved off-screen —
                      it keeps the label clickable and lets the tests (and a screen reader)
                      read the picked state by the device name. */}
                  <label className="tile-pick">
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
                  <details
                    className="interaction-raw-code"
                    aria-label={`Chi tiết kỹ thuật ${name}`}
                  >
                    <summary>Chi tiết</summary>
                    <code>{device.udid}</code>
                  </details>
                  {/* The @handle this phone is logged into. Kept next to the phone so an
                      operator sets it once, here, and tagging it later pulls this phone into
                      the post. Blurring saves it to the device meta. */}
                  {commentEnabled && (
                    <input
                      type="text"
                      className="interaction-handle"
                    placeholder="@handle"
                    spellCheck={false}
                    aria-label={`Tài khoản TikTok của ${name}`}
                    title="Nick TikTok máy này đang đăng nhập — để tag thì máy này tự vào comment"
                      value={handles[device.udid] ?? ""}
                      onChange={(event) => onHandleChange(device.udid, event.target.value)}
                      onBlur={(event) => onHandleBlur(device.udid, event.target.value)}
                    />
                  )}
                </div>
              );
            })}
          </div>
        ))}
    </div>
  );
}
