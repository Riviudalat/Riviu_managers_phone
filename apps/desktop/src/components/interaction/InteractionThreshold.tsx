import { InfoDot as Info } from "../InfoDot";
import { Banner } from "../States";
import { wholeNumber } from "../../interactionPlan";
import type { InteractionPostReading, MetricPlan, PostTargets } from "../../types";

/** Everything the threshold section needs, bundled so the shell owns the state. */
export type ThresholdControls = {
  wanted: PostTargets;
  setWanted: (next: PostTargets) => void;
  /** Whether a reading should pay for the view count. */
  readViews: boolean;
  setReadViews: (next: boolean) => void;
  reading: InteractionPostReading | null;
  busy: boolean;
  error: string | null;
  onMeasure: () => void;
  /** False when there is no link to read or no phone to read it with. */
  canMeasure: boolean;
};

type MetricKey = keyof PostTargets;

const METRICS: { key: MetricKey; label: string; what: string }[] = [
  {
    key: "views",
    label: "View",
    what:
      "Số phát chỉ hiện trên lưới hồ sơ tác giả, dưới từng ô — và lưới không nói ô nào là bài " +
      "nào, nên phải mở từng ô và so caption. Đo 24/08/2026: ~4,5 phút một lần đọc khi bài nằm " +
      "gần đầu lưới, lâu hơn khi bài nằm sâu. Đo được thì view tích luỹ theo lượt (~1,4 view " +
      "mỗi máy xác nhận tới bài), " +
      "nên ngưỡng view là một lịch chạy chứ không phải một lần bấm.",
  },
  {
    key: "likes",
    label: "Tim",
    what:
      "Mỗi acc chỉ tim được một lần, nên trần đúng bằng số máy chưa tim bài này — một ngưỡng " +
      "cao hơn thế thì chạy bao lâu cũng không tới, và app nói trước chứ không để anh farm cả " +
      "tiếng rồi mới biết.",
  },
  {
    key: "comments",
    label: "Bình luận",
    what:
      "Một acc bình luận được nhiều lần nên trần không phải cỡ fleet. Nó là chuyện thẩm mỹ: " +
      "mười bốn acc để năm mươi bình luận thì đọc ra đúng cái nó là.",
  },
];

/** How one metric's verdict reads on screen. */
function metricLine(label: string, now: number | null, plan: MetricPlan | null): string {
  const at = now === null ? "chưa đọc được" : `${now.toLocaleString("vi-VN")}`;
  if (!plan) return `${label}: đang ${at} — không đặt ngưỡng`;
  if (plan.unreachable) return `${label}: đang ${at} — ${plan.unreachable}`;
  if (plan.shortfall === 0) return `${label}: đang ${at} — đã đạt`;
  const passes =
    plan.passes === null ? "" : ` · ước ${plan.passes} lượt`;
  const ceiling =
    plan.ceiling === null ? "" : ` · trần ${plan.ceiling.toLocaleString("vi-VN")}`;
  return `${label}: đang ${at} — còn thiếu ${plan.shortfall.toLocaleString(
    "vi-VN",
  )}${passes}${ceiling}`;
}

/**
 * "Bài này mới bao nhiêu, tôi muốn bao nhiêu, farm thế nào là đủ?"
 *
 * **The reading is a button, not a side effect.** Likes and comments come off the post page in
 * two label reads, but a view count is a navigation that took about four and a half minutes when
 * timed 24/08/2026 — longer when the post sits deeper in the grid — and takes a phone away for
 * all of it, so nothing here runs on a debounce or on a paste. The operator presses Đo
 * bài, and `readViews` decides whether they are paying for the slow half.
 *
 * The plan is the backend's own `plan_thresholds`, not a copy: a like target above the accounts
 * that have not liked yet is refused here rather than discovered an hour into a run.
 */
export function InteractionThreshold({ controls }: { controls: ThresholdControls }) {
  const { wanted, setWanted, reading } = controls;
  const plan = reading?.plan;
  return (
    <>
      <div className="nu-group-head">
        Ngưỡng
        <Info
          of="Ngưỡng"
          what="Đặt số muốn đạt cho từng loại rồi bấm Đo bài: app lái một máy đọc bài thật và nói còn thiếu bao nhiêu, mấy lượt, hoặc vì sao không tới được. Để trống loại nào là không đặt ngưỡng loại đó."
        />
      </div>
      <div className="nu-grid interaction-grid-3">
        {METRICS.map(({ key, label, what }) => (
          <label className="nu-field" key={key}>
            <span className="nu-label">
              {label}
              <Info of={label} what={what} />
            </span>
            <input
              type="number"
              min={0}
              max={100000000}
              placeholder="—"
              // Named explicitly: the visible label carries an `Info` dot, so the name derived
              // from it is the label plus the whole tooltip.
              aria-label={`${label} muốn đạt`}
              value={wanted[key] ?? ""}
              onChange={(event) =>
                setWanted({
                  ...wanted,
                  [key]:
                    event.target.value === "" ? null : wholeNumber(event.target.value),
                })
              }
            />
          </label>
        ))}
      </div>

      <label className="nu-switch">
        <input
          type="checkbox"
          checked={controls.readViews}
          onChange={(event) => controls.setReadViews(event.target.checked)}
          aria-label="Đọc cả số view"
        />
        <span className="nu-switch-track" aria-hidden="true" />
        <span className="nu-switch-label">
          Đọc cả số view (chậm)
          <Info
            of="Đọc cả số view"
            what="Tắt thì chỉ đọc tim và bình luận — hai lần đọc nhãn trên trang bài, vài giây. Bật thì thêm một vòng đi bộ lưới hồ sơ để tìm ô của bài, đo 24/08/2026 là ~4,5 phút với bài nằm gần đầu lưới, lâu hơn khi bài nằm sâu — và chiếm một máy suốt thời gian đó."
          />
        </span>
      </label>

      <button
        type="button"
        className="ghost"
        disabled={!controls.canMeasure || controls.busy}
        onClick={controls.onMeasure}
      >
        {controls.busy ? "Đang đọc bài…" : "Đo bài"}
      </button>

      {controls.error && <Banner tone="error">{controls.error}</Banner>}

      {reading && (
        <div className="interaction-threshold-read">
          {METRICS.map(({ key, label }) => (
            <small key={key}>{metricLine(label, reading.now[key], plan?.[key] ?? null)}</small>
          ))}
          {!reading.viewsRead && (
            <small className="hint">
              Số view chưa đọc — bật "Đọc cả số view" rồi đo lại nếu cần.
            </small>
          )}
        </div>
      )}
    </>
  );
}
