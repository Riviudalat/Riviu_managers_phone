import {
  BUDGET_TOTAL,
  budgetCeiling,
  budgetFree,
  budgetUsed,
  fitToBudget,
  type BudgetKey,
} from "../../nurtureBudget";
import type { NurtureSettings } from "../../types";
import { InfoDot as Info } from "../InfoDot";
import { NurtureWindows } from "./NurtureWindows";
import { Switch, FeatureRow } from "../NurturePopup";

/**
 * How the session behaves: the four rates that share one 100% budget, the watch window,
 * fatigue, night hours and the carousel.
 */
export function NurtureBehaviourTab({
  settings,
  patch,
  patchRate,
  setSettings,
  overBudget,
  targets,
}: {
  settings: NurtureSettings;
  patch: <K extends keyof NurtureSettings>(key: K, value: NurtureSettings[K]) => void;
  patchRate: (key: BudgetKey, value: number) => void;
  setSettings: React.Dispatch<React.SetStateAction<NurtureSettings | null>>;
  overBudget: boolean;
  /** The phones selected on the grid, for a window that wants only some of them. */
  targets: string[];
}) {
  return (
    <div className="nurture-sect nu-pane" role="tabpanel">
      <div className="nu-grid">
        <label className="nu-field">
          <span className="nu-label">
            Giới hạn video
            <Info
              of="Giới hạn video"
              what="Phiên dừng sau đúng số video này (nhân với số vòng). Thời lượng phiên vẫn là trần riêng: cái nào tới trước thì dừng."
            />
          </span>
          <input
            type="number"
            min={1}
            max={10000}
            value={settings.numVideos}
            onChange={(e) => patch("numVideos", Number(e.target.value) || 1)}
          />
        </label>
        <label className="nu-field">
          <span className="nu-label">
            Vòng
            <Info
              of="Vòng"
              what="Nhân với giới hạn video để ra tổng số video của phiên: 15 video × 2 vòng = 30 video."
            />
          </span>
          <input
            type="number"
            min={1}
            max={100}
            value={settings.numRounds}
            onChange={(e) => patch("numRounds", Number(e.target.value) || 1)}
          />
        </label>
      </div>

      <div className="nu-group">
        <div className="nu-group-head">
          Tương tác
          {/* The budget, stated where it is spent: four rates sharing a hundred
              need the remainder on screen or every drag is a guess. */}
          <span className={`nu-budget${overBudget ? " is-over" : ""}`}>
            {overBudget
              ? `Đang dùng ${budgetUsed(settings)}% / ${BUDGET_TOTAL}%`
              : `Còn ${budgetFree(settings)}% / ${BUDGET_TOTAL}%`}
          </span>
        </div>
        {overBudget && (
          /* Two ways to get here: a config saved before the budget existed, and a
             switch turned back on over a number that no longer fits. Both leave
             every ceiling at or below where its rate already is, so no slider can
             be dragged up. Said out loud with a one-press fix rather than rewritten
             on load or on the switch click: these are the operator's tuned numbers,
             and something silently editing them is worse than a sentence asking. */
          <p className="nu-budget-warn" role="alert">
            Các tỉ lệ đang bật dùng chung {BUDGET_TOTAL}%, mà cộng lại đang là{" "}
            {budgetUsed(settings)}%. Kéo xuống cho vừa, hoặc{" "}
            <button
              type="button"
              className="link"
              onClick={() =>
                setSettings((prev) => (prev ? { ...prev, ...fitToBudget(prev) } : prev))
              }
            >
              đưa về {BUDGET_TOTAL}%
            </button>{" "}
            (trừ dần từ tỉ lệ lớn nhất).
          </p>
        )}
        <FeatureRow
          label="Thích"
          what="Tỉ lệ post được thả tim. Chỉ tính thành công khi nhãn nút tim đổi trạng thái, không phải khi tap xong — nên số 'đã tim' luôn nhỏ hơn hoặc bằng số lần thử."
          percent={settings.likeProb}
          ceiling={budgetCeiling(settings, "likeProb")}
          enabled={settings.likeEnabled ?? true}
          onPercent={(v) => patchRate("likeProb", v)}
          onEnabled={(v) => patch("likeEnabled", v)}
        />
        <FeatureRow
          label="Bình luận"
          what="Tỉ lệ post được bình luận. AI đọc nội dung post rồi tự viết; chỉ tính là đã gửi khi nút Gửi tắt lại. Cần API key ở tab AI."
          percent={settings.commentProb}
          ceiling={budgetCeiling(settings, "commentProb")}
          enabled={settings.commentEnabled ?? true}
          onPercent={(v) => patchRate("commentProb", v)}
          onEnabled={(v) => patch("commentEnabled", v)}
        />
        <FeatureRow
          label="Follow"
          what="Tỉ lệ post được follow tác giả, tính riêng chứ không kèm thích hay bình luận. Xác nhận bằng việc nút Follow mất khỏi thẻ."
          percent={settings.followProb}
          ceiling={budgetCeiling(settings, "followProb")}
          enabled={settings.followEnabled ?? true}
          onPercent={(v) => patchRate("followProb", v)}
          onEnabled={(v) => patch("followEnabled", v)}
        />
        <FeatureRow
          label="Vuốt nhanh"
          what="Tỉ lệ post bị vuốt qua nhanh, không xem hết — giống lúc người ta lướt cho qua mấy bài không quan tâm."
          percent={settings.frenzyProb}
          ceiling={budgetCeiling(settings, "frenzyProb")}
          enabled={settings.frenzyEnabled ?? true}
          onPercent={(v) => patchRate("frenzyProb", v)}
          onEnabled={(v) => patch("frenzyEnabled", v)}
        />
      </div>

      <div className="nu-group">
        <div className="nu-group-head">Nhịp</div>
        <div className="nu-grid">
          <label className="nu-field">
            <span className="nu-label">
              Xem min
              <Info
                of="Xem min"
                what="Số giây ít nhất dừng lại ở mỗi post. Nhịp phiên còn nhân thêm hệ số theo tâm trạng, nên số 'xem' trong log có thể ra ngoài khoảng min–max."
              />
            </span>
            <input
              type="number"
              step="0.5"
              min={0.5}
              max={120}
              value={settings.watchMin}
              onChange={(e) => patch("watchMin", Number(e.target.value) || 1)}
            />
          </label>
          <label className="nu-field">
            <span className="nu-label">
              Xem max
              <Info
                of="Xem max"
                what="Số giây nhiều nhất dừng lại ở mỗi post, trước khi nhân hệ số nhịp. Đặt sát min thì phiên đều đặn hơn nhưng cũng máy móc hơn."
              />
            </span>
            <input
              type="number"
              step="0.5"
              min={0.5}
              max={120}
              value={settings.watchMax}
              onChange={(e) => patch("watchMax", Number(e.target.value) || 5)}
            />
          </label>
        </div>
        <Switch
          checked={settings.fatigue}
          onChange={(v) => patch("fatigue", v)}
          label="Mỏi dần"
          what="Càng về cuối phiên càng xem lâu và tương tác thưa hơn, thay vì giữ đúng một nhịp từ đầu tới cuối. Bật thì số tim thực tế thấp hơn tỉ lệ đã đặt."
        />
        <Switch
          checked={settings.timeOfDay}
          onChange={(v) => patch("timeOfDay", v)}
          label="Theo giờ trong ngày"
          what="Nhịp thay đổi theo giờ thật của máy tính: đêm và giờ làm thì chậm và ít tương tác hơn giờ cao điểm."
        />
        <Switch
          checked={settings.pauseSwipe}
          onChange={(v) => patch("pauseSwipe", v)}
          label="Ngập ngừng khi vuốt"
          what="Thỉnh thoảng vuốt nửa vời rồi mới vuốt hẩn, và thời gian mỗi cú vuốt không đều nhau."
        />
        <Switch
          checked={settings.humanLimits ?? false}
          onChange={(v) => patch("humanLimits", v)}
          label="Giới hạn nhịp người"
          what="Tắt (mặc định): các tỉ lệ bạn đặt ở trên là tỉ lệ thực. Bật: engine tự áp thêm trần 8–16 tim / 1–3 bình luản / 1–2 follow mỗi giờ, chỉ cho tương tác 2 trong 5 bài gần nhất, chờ 12–35 giây sau mỗi hành động và nghỉ 15–90 giây mỗi 7–13 bài — phiên trông giống người hơn nhưng chạy ít hơn nhiều so với số bạn đặt."
        />
        <div className="nu-grid">
          <label className="nu-field">
            <span className="nu-label">
              Nghỉ đêm từ
              <Info
                of="Nghỉ đêm"
                what="Rơi vào khoảng giờ này thì phiên tự dừng, tính theo giờ máy tính. Để 0 và 0 là không nghỉ đêm."
              />
            </span>
            <input
              type="number"
              min={0}
              max={23}
              value={settings.nightStart}
              onChange={(e) => patch("nightStart", Number(e.target.value) || 0)}
            />
          </label>
          <label className="nu-field">
            <span className="nu-label">đến</span>
            <input
              type="number"
              min={0}
              max={23}
              value={settings.nightEnd}
              onChange={(e) => patch("nightEnd", Number(e.target.value) || 0)}
            />
          </label>
        </div>
      </div>

      <div className="nu-group">
        <div className="nu-group-head">Bài ảnh</div>
        <div className="nu-feature">
          <label className="nu-switch nu-switch-bare">
            <input
              type="checkbox"
              checked={settings.carouselEnabled ?? true}
              onChange={(e) => patch("carouselEnabled", e.target.checked)}
              aria-label="Bật vuốt ngang bài ảnh"
            />
            <span className="nu-switch-track" aria-hidden="true" />
          </label>
          <span className="nu-feature-name">
            Vuốt ngang
            <Info
              of="Vuốt ngang"
              what="Bài nhiều ảnh thì vuốt ngang xem tiếp, thay vì bỏ qua sau ảnh đầu. Phần trăm bên cạnh là xem bao nhiêu phần của bài."
            />
          </span>
          <label className="nu-feature-pct">
            <input
              type="number"
              min={1}
              max={100}
              step={5}
              value={settings.carouselPortionPercent ?? 100}
              onChange={(e) =>
                patch("carouselPortionPercent", Number(e.target.value) || 100)
              }
              aria-label="Xem bao nhiêu phần trăm bài ảnh"
              title="100% là xem tới hết bài — dừng khi một cú vuốt không còn làm đổi ảnh. 50% là xem khoảng nửa bài rồi vuốt sang bài khác."
            />
            <span aria-hidden="true">%</span>
          </label>
        </div>
      </div>

      <label className="nu-field">
        <span className="nu-label">
          Bundle TikTok
          <Info
            of="Bundle TikTok"
            what="App id của TikTok. Trên Android app tự tìm package đã cài trên từng máy nên thường không cần sửa ô này; nó chủ yếu dành cho iPhone."
          />
        </span>
        <input value={settings.bundleId} onChange={(e) => patch("bundleId", e.target.value)} />
      </label>

      {/* The schedule sits at the bottom of this pane rather than in a tab of its own: a
          window overrides the rates above it, and the two were a tab apart. */}
      <NurtureWindows settings={settings} patch={patch} targets={targets} />
    </div>
  );
}
