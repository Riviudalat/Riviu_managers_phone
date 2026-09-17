import { useState, type ReactNode } from "react";
import { Search } from "lucide-react";
import { changeDistribution, distributionOf, nurtureActions } from "../../nurtureDistribution";
import { MachineChoice } from "../MachineChoice";
import { DetailDrawer } from "../WorkspacePrimitives";
import type { DeviceInfo, DeviceMeta, NurtureSettings, TargetRef } from "../../types";
import { orderDevicesByNumber, tileName, tileNumber } from "../../deviceNaming";
import { nurtureFieldValidation, type NurtureSettingsIssue } from "../../nurtureValidation";

export function NurtureSessionSetup({ settings, onChange, issue, issueId }: {
  settings: NurtureSettings;
  onChange: (settings: NurtureSettings) => void;
  issue: NurtureSettingsIssue | null;
  issueId: string;
}) {
  const patch = (change: Partial<NurtureSettings>) => onChange({ ...settings, ...change });
  const distribution=distributionOf(settings);
  return <section className="nurture-session-card" aria-label="Thiết lập phiên">
    <header className="nurture-card-heading"><h2>Thiết lập phiên</h2><span className="automation-section-meta">Áp dụng cho mỗi máy</span></header>
    <div className="nurture-session-limits">
      <label>Tổng số video muốn lướt<span className="nurture-unit-input"><input id="nurture-basic-videos" aria-label="Tổng số video muốn lướt" type="number" min={1} max={10000} step={1} value={Number.isFinite(settings.numVideos * settings.numRounds) ? settings.numVideos * settings.numRounds : ""}
        {...nurtureFieldValidation("numVideos", issue, issueId)} onChange={(event) => patch({ numVideos: Number(event.target.value), numRounds: 1 })} /><span>video / máy</span></span></label>
      <label>Thời lượng tối đa<span className="nurture-unit-input"><input type="number" min={15} max={360} value={settings.scheduleDurationMinutes}
        {...nurtureFieldValidation("scheduleDurationMinutes", issue, issueId)} onChange={(event) => patch({ scheduleDurationMinutes: Number(event.target.value) })} /><span>phút</span></span></label>
    </div>
    <div className={`nurture-search-settings${settings.feedSource === "search" ? " has-keyword" : ""}`}>
      <label>Nguồn video<select value={settings.feedSource ?? "forYou"} onChange={event=>patch({feedSource:event.target.value as "forYou"|"search"})}>
        <option value="forYou">Đề xuất (For You)</option><option value="search">Lướt theo từ khóa</option>
      </select></label>
      {settings.feedSource === "search" && <label>Từ khóa tìm kiếm<input aria-label="Từ khóa tìm kiếm" maxLength={100} placeholder="Ví dụ: đà lạt" value={settings.searchKeyword ?? ""}
        {...nurtureFieldValidation("searchKeyword",issue,issueId)} onChange={event=>patch({searchKeyword:event.target.value})}/></label>}
      {settings.feedSource === "search" && <p className="nurture-setup-note">Mở tìm kiếm TikTok, nhập từ khóa và lướt video trong kết quả. Hiện hỗ trợ Android.</p>}
    </div>
    <div className="nurture-action-heading"><div><strong>Phân bổ 100 video</strong><small>Mỗi video chọn một hành động; phần còn lại chỉ xem.</small></div><span className="nurture-total-rate">100%</span></div>
    <div className="nurture-distribution" role="img" aria-label={`Chỉ xem ${distribution.watch}%, ${distribution.actions.map(a=>`${a.label} ${a.value}%`).join(", ")}`}>
      <span className="is-watch" style={{flexGrow:distribution.watch}} />
      {distribution.actions.filter(a=>a.value>0).map(a=><span key={a.key} style={{flexGrow:a.value,background:a.color}} />)}
    </div>
    <div className="nurture-watch-share"><span><i/>Chỉ xem<span className="nurture-share-description">Tự cân bằng khi bạn đổi tỷ lệ</span></span><strong>{distribution.watch}%</strong></div>
    <div className="nurture-action-rows">
      {nurtureActions.map(({ key, label, enabled, rate, color }) => <div className={`nurture-action-row${settings[enabled] !== false ? "" : " is-off"}`} key={key}>
        <label><input type="checkbox" checked={settings[enabled] !== false} onChange={(event) => event.target.checked?onChange(changeDistribution(settings,rate,settings[rate]??0)):patch({[enabled]:false})} /><i style={{background:color}}/><span>{label}</span></label>
        <input aria-label={`Tỷ lệ ${label}`} type="range" min={0} max={100} value={settings[rate] ?? 0}
          disabled={settings[enabled]===false} onChange={(event) => onChange(changeDistribution(settings,rate,Number(event.target.value)))} />
        <span className="nurture-rate-value"><input aria-label={`Phần trăm ${label}`} type="number" min={0} max={100} value={settings[rate] ?? 0}
          disabled={settings[enabled]===false} onChange={(event) => onChange(changeDistribution(settings,rate,Number(event.target.value)))} /><span>%</span></span>
      </div>)}
    </div>
    {settings.scheduleEnabled && <p className="nurture-setup-note">Lịch tự chạy đang bật. Xem ở tab Hẹn giờ.</p>}
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
  const [open, setOpen] = useState(false);
  const ordered = orderDevicesByNumber(devices, metas).map((device, index) => ({ device, number: tileNumber(index + 1, metas.get(device.udid)), name: tileName(device, metas.get(device.udid)) }));
  const filtered = ordered.filter(({ device, number, name }) => `${number} ${name} ${metas.get(device.udid)?.handle ?? ""}`.toLocaleLowerCase("vi").includes(query.trim().toLocaleLowerCase("vi")));
  const selected = new Set(targets);
  const available = ordered.filter(({ device }) => device.status === "ready");
  const unavailable = devices.length - available.length;
  const setTargets = (udids: string[]) => onTargetRefChange?.({ type: "explicit", udids });
  const names = ordered.filter(({ device }) => selected.has(device.udid)).map(({ number, name }) => `${number} · ${name}`).join(", ");
  return <>
    <div className="nurture-scope-bar" role="group" aria-label="Phạm vi Nuôi TikTok">
      <strong>Máy thực hiện</strong>
      {scopeControl}
      <span className="nurture-count" role="status">Đã chọn {targets.length}</span>
      <span className="nurture-scope-names" title={names}>{names || "Chưa có máy được chọn"}</span>
      <button type="button" className="ghost" aria-expanded={open} aria-haspopup="dialog" onClick={() => setOpen(true)}>Chọn máy</button>
    </div>
    <DetailDrawer open={open} title="Chọn máy Nuôi TikTok" onClose={() => setOpen(false)}
      footer={<><span>{targets.length} máy được chọn</span><button type="button" className="primary" onClick={() => setOpen(false)}>Xong</button></>}>
    <section className="nurture-machines-card" aria-label="Máy thực hiện">
    <header className="nurture-card-heading"><h2>Máy thực hiện</h2><span className="nurture-count" role="status">Đã chọn {targets.length}</span></header>
    <label className="nurture-machine-search"><Search size={16} aria-hidden="true" /><input type="search" aria-label="Tìm máy Nuôi TikTok" placeholder="Tìm số máy hoặc tên" value={query} onChange={(event) => setQuery(event.target.value)} /></label>
    <div className="nurture-machine-tools">
      <button type="button" className="ghost" title="Chọn tất cả máy sẵn sàng, kể cả máy ngoài kết quả tìm kiếm" disabled={!onTargetRefChange || !available.length} onClick={() => setTargets(available.map(({device}) => device.udid))}>Chọn tất cả sẵn sàng</button>
      <button type="button" className="ghost" disabled={!onTargetRefChange || !targets.length} onClick={() => setTargets([])}>Bỏ chọn</button>
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
    </section>
    </DetailDrawer>
  </>;
}

/** A review of the current draft, never a prediction of device outcomes. */
export function NurtureSetupSummary({ settings, targetCount }: { settings: NurtureSettings; targetCount: number }) {
  const distribution = distributionOf(settings);
  return <aside className="nurture-setup-summary" aria-label="Tóm tắt phiên nuôi" tabIndex={0}>
    <h2>Phiên sắp chạy</h2>
    <dl>
      <div><dt>Máy thực hiện</dt><dd>{targetCount} máy</dd></div>
      <div><dt>Giới hạn mỗi máy</dt><dd>{settings.numVideos * settings.numRounds} video · {settings.scheduleDurationMinutes} phút</dd></div>
      <div><dt>Nguồn video</dt><dd>{settings.feedSource === "search" ? settings.searchKeyword || "Chưa nhập từ khóa" : "Đề xuất (For You)"}</dd></div>
      <div><dt>Nhịp xem</dt><dd>{settings.watchMin}–{settings.watchMax} giây</dd></div>
      <div><dt>Phân bổ hành động</dt><dd>{distribution.actions.filter(action => action.value > 0).map(action => `${action.label} ${action.value}%`).join(" · ") || "Chỉ xem"}</dd></div>
      <div><dt>Chỉ xem</dt><dd>{distribution.watch}%</dd></div>
    </dl>
    <p>Phiên dừng khi chạm giới hạn đầu tiên. Kết quả và bằng chứng được ghi riêng cho từng máy.</p>
    <p>{settings.scheduleEnabled ? "Lịch tự chạy đang bật; kiểm tra khung giờ trong Hẹn giờ." : "Chưa bật lịch tự chạy. Chuyển tab không bắt đầu phiên."}</p>
  </aside>;
}
