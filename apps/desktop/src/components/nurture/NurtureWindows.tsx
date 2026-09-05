import type { NurtureSettings, NurtureWindow, NurtureWindowBehaviour } from "../../types";
import { nurtureFieldValidation, type NurtureSettingsIssue } from "../../nurtureValidation";
import { InfoDot as Info } from "../InfoDot";

/**
 * When the schedule is allowed to run, and how it runs in each stretch of the day.
 *
 * **Lives under Hành vi, not in a tab of its own.** A schedule is a statement about behaviour
 * — these hours, this hard, on these phones — and reading it a tab away from the rates it
 * overrides is how a window ends up quietly contradicting the panel above it.
 *
 * With no windows the schedule keeps its old shape: one cadence, all day. That is what every
 * settings row written before this editor existed contains, so the empty state is a real mode
 * of the product and says so, rather than pretending the feature is simply unconfigured.
 */

/** `480` → `"08:00"`. */
function toClock(minute: number): string {
  const m = Math.max(0, Math.min(1439, Math.round(minute)));
  return `${String(Math.floor(m / 60)).padStart(2, "0")}:${String(m % 60).padStart(2, "0")}`;
}

/** `"08:00"` → `480`; anything unreadable keeps the value that was already there. */
function fromClock(value: string, fallback: number): number {
  const match = /^(\d{1,2}):(\d{2})$/.exec(value.trim());
  if (!match) return fallback;
  const minute = Number(match[1]) * 60 + Number(match[2]);
  return Number.isFinite(minute) && minute >= 0 && minute <= 1439 ? minute : fallback;
}

/** A window whose end is at or before its start runs through midnight. */
function wrapsMidnight(window: NurtureWindow): boolean {
  return window.endMinute <= window.startMinute;
}

function newWindowId(): string {
  // Hyphens and hex only, which is what the Rust validator accepts — the id becomes a
  // settings key holding this window's "next due" mark.
  return `w-${crypto.randomUUID().slice(0, 8)}`;
}

function behaviourFrom(settings: NurtureSettings): NurtureWindowBehaviour {
  return {
    numVideos: settings.numVideos,
    numRounds: settings.numRounds,
    likeProb: settings.likeProb,
    commentProb: settings.commentProb,
    saveProb: settings.saveProb ?? 0,
    saveEnabled: settings.saveEnabled ?? false,
    followProb: settings.followProb,
  };
}

export function NurtureWindows({
  settings,
  patch,
  targets,
  issue,
  issueId,
}: {
  settings: NurtureSettings;
  patch: <K extends keyof NurtureSettings>(key: K, value: NurtureSettings[K]) => void;
  /** The phones selected on the grid right now — what "dùng máy đang chọn" means. */
  targets: string[];
  issue?: NurtureSettingsIssue | null;
  issueId?: string;
}) {
  const windows = settings.scheduleWindows ?? [];

  const setWindows = (next: NurtureWindow[]) => patch("scheduleWindows", next);
  const editWindow = (index: number, change: Partial<NurtureWindow>) =>
    setWindows(windows.map((w, i) => (i === index ? { ...w, ...change } : w)));

  const addWindow = () => {
    const last = windows[windows.length - 1];
    setWindows([
      ...windows,
      {
        id: newWindowId(),
        // Start where the previous one ended, so a second window does not land on top of the
        // first — overlapping windows are legal but the first match wins, and a duplicate
        // pair would leave the operator wondering why the second never runs.
        startMinute: last ? last.endMinute : 8 * 60,
        endMinute: last ? Math.min(1439, last.endMinute + 180) : 11 * 60,
        everyMinutes: 60,
        durationMinutes: 20,
        udids: [],
        behaviour: null,
      },
    ]);
  };

  return (
    <div className="nurture-sect nurture-sched">
      <label className="check">
        <input
          type="checkbox"
          checked={settings.scheduleEnabled}
          onChange={(e) => patch("scheduleEnabled", e.target.checked)}
        />
        <span className="nu-inline">
          Lịch tự chạy
          <Info
            of="Lịch tự chạy"
            what="Tự khởi động phiên, không cần bấm Bắt đầu. Không có khung giờ nào thì chạy cả ngày theo một chu kỳ duy nhất; có khung thì chỉ chạy trong khung."
          />
        </span>
      </label>

      {(windows.length === 0 || issue?.field === "scheduleEveryMinutes" || issue?.field === "scheduleDurationMinutes") && (
        <>
          <div className="nurture-row">
            <label>
              <span className="nu-inline">
                Mỗi (phút)
                <Info of="Mỗi (phút)" what="Khoảng cách giữa hai lần tự khởi động." />
              </span>
              <input
                type="number"
                min={15}
                max={1440}
                value={settings.scheduleEveryMinutes}
                {...nurtureFieldValidation("scheduleEveryMinutes", issue, issueId)}
                onChange={(e) =>
                  patch("scheduleEveryMinutes", Number(e.target.value) || 240)
                }
              />
            </label>
            <label>
              <span className="nu-inline">
                Thời lượng (phút)
                <Info
                  of="Thời lượng (phút)"
                  what="Thời lượng tối đa của phiên theo lịch; phiên dừng ở ranh giới giữa hai video. Phiên bấm tay không dùng giới hạn này."
                />
              </span>
              <input
                type="number"
                min={15}
                max={360}
                value={settings.scheduleDurationMinutes}
                {...nurtureFieldValidation("scheduleDurationMinutes", issue, issueId)}
                onChange={(e) =>
                  patch("scheduleDurationMinutes", Number(e.target.value) || 150)
                }
              />
            </label>
          </div>
          {windows.length === 0 && <p className="nu-hint">
            Chưa có khung giờ nào — lịch sẽ chạy <strong>cả ngày</strong> theo chu kỳ trên,
            kể cả ban đêm. Thêm khung giờ để giới hạn lại.
          </p>}
        </>
      )}

      {windows.map((window, index) => {
        const behaviour = window.behaviour ?? null;
        return (
          <div className="nu-window" key={window.id}>
            <div className="nu-window-head">
              <input
                type="time"
                aria-label={`Giờ bắt đầu khung ${index + 1}`}
                value={toClock(window.startMinute)}
                onChange={(e) =>
                  editWindow(index, {
                    startMinute: fromClock(e.target.value, window.startMinute),
                  })
                }
              />
              <span aria-hidden="true">–</span>
              <input
                type="time"
                aria-label={`Giờ kết thúc khung ${index + 1}`}
                value={toClock(window.endMinute)}
                onChange={(e) =>
                  editWindow(index, { endMinute: fromClock(e.target.value, window.endMinute) })
                }
              />
              {wrapsMidnight(window) && (
                <span className="nu-window-wrap" title="Khung này vắt qua nửa đêm">
                  qua đêm
                </span>
              )}
              <div className="grow" />
              <button
                type="button"
                className="ghost"
                aria-label={`Xoá khung ${index + 1}`}
                onClick={() => setWindows(windows.filter((_, i) => i !== index))}
              >
                Xoá
              </button>
            </div>

            <div className="nurture-row">
              <label>
                <span className="nu-inline">Mỗi (phút)</span>
                <input
                  type="number"
                  min={15}
                  max={1440}
                  value={window.everyMinutes}
                  onChange={(e) =>
                    editWindow(index, { everyMinutes: Number(e.target.value) || 60 })
                  }
                />
              </label>
              <label>
                <span className="nu-inline">Thời lượng (phút)</span>
                <input
                  type="number"
                  min={15}
                  max={360}
                  value={window.durationMinutes}
                  onChange={(e) =>
                    editWindow(index, { durationMinutes: Number(e.target.value) || 20 })
                  }
                />
              </label>
            </div>

            {/* **The phone list says "tất cả" in words, not as a blank.**
                An empty list means every connected phone — the same rule the schedule has
                always had — and the one thing that makes that rule safe is being able to read
                it. A blank field here would look like "none". */}
            <div className="nu-window-phones">
              {/* Plain text, not `.nu-inline`: that class is an inline *flex* row, so
                  "Máy:" and the value became two flex items and stacked into two lines the
                  moment the 360px panel ran out of room. */}
              <span className="nu-window-who">
                Máy:{" "}
                <strong title={window.udids.join(", ") || undefined}>
                  {window.udids.length ? `${window.udids.length} máy đã chọn` : "tất cả"}
                </strong>
              </span>
              <button
                type="button"
                className="ghost"
                disabled={!targets.length}
                title={
                  targets.length
                    ? targets.join(", ")
                    : "Chọn máy trên lưới trước rồi bấm lại"
                }
                onClick={() => editWindow(index, { udids: targets })}
              >
                Dùng máy đang chọn ({targets.length})
              </button>
              <button
                type="button"
                className="ghost"
                disabled={!window.udids.length}
                onClick={() => editWindow(index, { udids: [] })}
              >
                Tất cả
              </button>
            </div>

            <label className="check">
              <input
                type="checkbox"
                checked={behaviour !== null}
                onChange={(e) =>
                  editWindow(index, {
                    behaviour: e.target.checked ? behaviourFrom(settings) : null,
                  })
                }
              />
              <span className="nu-inline">
                Cấu hình riêng cho khung này
                <Info
                  of="Cấu hình riêng cho khung này"
                  what="Tắt thì khung dùng đúng số video, công tắc và tỉ lệ ở phần trên. Bật thì khung mang trọn bộ riêng, không âm thầm nhận thay đổi toàn cục về sau."
                />
              </span>
            </label>

            {behaviour && (
              <>
                <label className="check">
                  <input
                    type="checkbox"
                    aria-label={`Bật Lưu trong khung ${index + 1}`}
                    checked={behaviour.saveEnabled ?? false}
                    onChange={(e) =>
                      editWindow(index, {
                        behaviour: { ...behaviour, saveEnabled: e.target.checked },
                      })
                    }
                  />
                  <span>Lưu trong khung này</span>
                </label>
                <div className="nurture-row nu-window-behaviour">
                {(
                  [
                    ["numVideos", "Video", 1, 10000],
                    ["numRounds", "Vòng", 1, 100],
                    ["likeProb", "Tim %", 0, 100],
                    ["saveProb", "Lưu %", 0, 100],
                    ["commentProb", "Bình luận %", 0, 100],
                    ["followProb", "Theo dõi %", 0, 100],
                  ] as const
                ).map(([key, label, min, max]) => (
                  <label key={key}>
                    <span className="nu-inline">{label}</span>
                    <input
                      type="number"
                      min={min}
                      max={max}
                      value={key === "saveProb" ? (behaviour.saveProb ?? 0) : behaviour[key]}
                      onChange={(e) =>
                        editWindow(index, {
                          behaviour: { ...behaviour, [key]: Number(e.target.value) || 0 },
                        })
                      }
                    />
                  </label>
                ))}
                </div>
              </>
            )}
          </div>
        );
      })}

      <button type="button" className="ghost" onClick={addWindow}>
        + Thêm khung giờ
      </button>
    </div>
  );
}
