import { useState, type ReactNode } from "react";
import { CheckCircle2, Search } from "lucide-react";
import { MachineChoice } from "../MachineChoice";
import type { DeviceInfo, DeviceMeta, NurtureSettings, TargetRef } from "../../types";
import { orderDevicesByNumber, tileName, tileNumber } from "../../deviceNaming";
import { nurtureFieldValidation, type NurtureSettingsIssue } from "../../nurtureValidation";

const actions = [
  { key: "like", label: "Tim", enabled: "likeEnabled", rate: "likeProb", fallback: true },
  { key: "save", label: "Lưu bài", enabled: "saveEnabled", rate: "saveProb", fallback: false },
  { key: "comment", label: "Bình luận", enabled: "commentEnabled", rate: "commentProb", fallback: true },
  { key: "follow", label: "Theo dõi", enabled: "followEnabled", rate: "followProb", fallback: true },
] as const;

/** Only settings change on preset selection. Dispatch remains the parent's explicit Start. */
function nurturePreset(settings: NurtureSettings, preset: "gentle" | "balanced"): NurtureSettings {
  return {
    ...settings,
    numRounds: 1,
    scheduleDurationMinutes: preset === "gentle" ? 15 : 20,
    numVideos: preset === "gentle" ? 15 : 30,
    likeEnabled: preset === "balanced",
    saveEnabled: preset === "balanced",
    commentEnabled: false,
    followEnabled: false,
    ...(preset === "balanced" ? { likeProb: 20, saveProb: 10 } : {}),
  };
}

function presetOf(settings: NurtureSettings) {
  if (settings.numRounds !== 1 || settings.commentEnabled !== false || settings.followEnabled !== false) return "custom";
  if (settings.scheduleDurationMinutes === 15 && settings.numVideos === 15 && settings.likeEnabled === false && settings.saveEnabled === false) return "gentle";
  if (settings.scheduleDurationMinutes === 20 && settings.numVideos === 30 && settings.likeEnabled && settings.saveEnabled && settings.likeProb === 20 && settings.saveProb === 10) return "balanced";
  return "custom";
}

export function NurtureSessionSetup({ settings, onChange, issue, issueId }: {
  settings: NurtureSettings;
  onChange: (settings: NurtureSettings) => void;
  issue: NurtureSettingsIssue | null;
  issueId: string;
}) {
  const [custom, setCustom] = useState(false);
  const preset = custom ? "custom" : presetOf(settings);
  const patch = (change: Partial<NurtureSettings>) => onChange({ ...settings, ...change });
  return <section className="nurture-session-card" aria-label="Thiết lập phiên">
    <header className="nurture-card-heading"><h2>Thiết lập phiên</h2><span className="automation-section-meta">Áp dụng cho mỗi máy</span></header>
    <div className="nurture-session-limits">
      <label>Tổng số video muốn lướt<span className="nurture-unit-input"><input id="nurture-basic-videos" aria-label="Tổng số video muốn lướt" type="number" min={1} max={10000} step={1} value={Number.isFinite(settings.numVideos * settings.numRounds) ? settings.numVideos * settings.numRounds : ""}
        {...nurtureFieldValidation("numVideos", issue, issueId)} onChange={(event) => patch({ numVideos: Number(event.target.value), numRounds: 1 })} /><span>video / máy</span></span></label>
      <label>Thời lượng tối đa<span className="nurture-unit-input"><input type="number" min={15} max={360} value={settings.scheduleDurationMinutes}
        {...nurtureFieldValidation("scheduleDurationMinutes", issue, issueId)} onChange={(event) => patch({ scheduleDurationMinutes: Number(event.target.value) })} /><span>phút</span></span></label>
    </div>
    <p className="nurture-setup-note">Tổng cho mỗi máy trong phiên. Dừng khi đủ số video hoặc hết thời lượng.</p>
    <div className="nurture-presets" aria-label="Hồ sơ nhịp chạy">
      {([
        ["gentle", "Nhẹ nhàng", "Chỉ xem nội dung"],
        ["balanced", "Cân bằng", "Xem, tim và lưu"],
        ["custom", "Tùy chỉnh", "Tự đặt tỷ lệ"],
      ] as const).map(([key, label, description]) => <button key={key} type="button" aria-pressed={preset === key}
        onClick={() => { setCustom(key === "custom"); if (key !== "custom") onChange(nurturePreset(settings, key)); else document.getElementById("nurture-basic-videos")?.focus(); }}>
        <strong>{label}</strong><span>{description}</span>
      </button>)}
    </div>
    <div className="nurture-action-heading"><strong>Hành động trên mỗi bài</strong><span>Tỷ lệ thực hiện</span></div>
    <div className="nurture-action-rows">
      {actions.map(({ key, label, enabled, rate, fallback }) => <div className={`nurture-action-row${settings[enabled] ?? fallback ? "" : " is-off"}`} key={key}>
        <label><input type="checkbox" checked={settings[enabled] ?? fallback} onChange={(event) => patch({ [enabled]: event.target.checked })} /><span>{label}</span></label>
        <input aria-label={`Tỷ lệ ${label}`} type="range" min={0} max={100} value={settings[rate] ?? 0}
          onChange={(event) => patch({ [rate]: Number(event.target.value) })} />
        <span className="nurture-rate-value"><input aria-label={`Phần trăm ${label}`} type="number" min={0} max={100} value={settings[rate] ?? 0}
          onChange={(event) => patch({ [rate]: Math.max(0, Math.min(100, Math.floor(Number(event.target.value)))) })} /><span>%</span></span>
      </div>)}
    </div>
    <p className="nurture-setup-note">Các tỷ lệ độc lập. Bài đã tim hoặc lưu được bỏ qua, không tính thêm lượt.</p>
    {settings.scheduleEnabled && <p className="nurture-setup-note">Lịch tự chạy đang bật. Xem ở tab Hẹn giờ.</p>}
    <div className="nurture-cleanup-note"><CheckCircle2 size={16} aria-hidden="true" /> Kết thúc: đóng TikTok và kiểm tra đã tắt.</div>
  </section>;
}

export function NurtureMachinePicker({ devices, metas, targets, onTargetRefChange, scopeControl }: {
  devices: DeviceInfo[];
  metas: Map<string, DeviceMeta>;
  targets: string[];
  onTargetRefChange?: (target: TargetRef) => void;
  scopeControl?: ReactNode;
}) {
  const [query, setQuery] = useState("");
  const ordered = orderDevicesByNumber(devices, metas).map((device, index) => ({ device, number: tileNumber(index + 1, metas.get(device.udid)), name: tileName(device, metas.get(device.udid)) }));
  const filtered = ordered.filter(({ device, number, name }) => `${number} ${name} ${metas.get(device.udid)?.handle ?? ""}`.toLocaleLowerCase("vi").includes(query.trim().toLocaleLowerCase("vi")));
  const selected = new Set(targets);
  const available = ordered.filter(({ device }) => device.status === "ready");
  const unavailable = devices.length - available.length;
  const setTargets = (udids: string[]) => onTargetRefChange?.({ type: "explicit", udids });
  return <section className="nurture-machines-card" aria-label="Máy thực hiện">
    <header className="nurture-card-heading"><h2>Máy thực hiện</h2><span className="nurture-count" role="status">Đã chọn {targets.length}</span></header>
    <label className="nurture-machine-search"><Search size={16} aria-hidden="true" /><input type="search" aria-label="Tìm máy Nuôi TikTok" placeholder="Tìm số máy hoặc tên" value={query} onChange={(event) => setQuery(event.target.value)} /></label>
    <div className="nurture-machine-tools">
      <button type="button" className="ghost" title="Chọn tất cả máy sẵn sàng, kể cả máy ngoài kết quả tìm kiếm" disabled={!onTargetRefChange || !available.length} onClick={() => setTargets(available.map(({device}) => device.udid))}>Chọn tất cả sẵn sàng</button>
      <button type="button" className="ghost" disabled={!onTargetRefChange || !targets.length} onClick={() => setTargets([])}>Bỏ chọn</button>
      {scopeControl}
      <small className="nurture-machine-ready-hint">{available.length} sẵn sàng · {devices.length} tổng</small>
    </div>
    {query && <p className="nurture-machine-filter-count">{filtered.length} máy khớp tìm kiếm</p>}
    {unavailable > 0 && <p className="nurture-setup-note">{unavailable} máy chưa sẵn sàng, chưa thể chọn thêm.</p>}
    <div className={`nurture-machine-grid machine-choice-grid${devices.length > 12 ? " is-compact" : ""}`} role="group" aria-label="Danh sách chọn máy Nuôi TikTok">
      {filtered.map(({ device, number, name }) => {
        const ready = device.status === "ready";
        const checked = selected.has(device.udid);
        return <MachineChoice key={device.udid} number={number} name={name} status={device.status} reason={device.lastError} checked={checked}
          label={`Chọn Máy ${number} · ${name}`} disabled={!onTargetRefChange || (!ready && !checked)}
          onChange={(selected) => setTargets(selected ? [...targets, device.udid] : targets.filter(id => id !== device.udid))}
          detail={metas.get(device.udid)?.handle ? <span>@{metas.get(device.udid)?.handle?.replace(/^@+/, "")}</span> : undefined} />;
      })}
      {!filtered.length && <p className="nurture-machine-empty">{devices.length ? "Không có máy khớp tìm kiếm." : "Kết nối thiết bị để chọn máy thực hiện."}</p>}
    </div>
  </section>;
}
