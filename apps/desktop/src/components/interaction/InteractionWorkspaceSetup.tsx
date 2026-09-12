import { useEffect, useState, type ComponentProps, type ReactNode } from "react";
import { Check, Search } from "lucide-react";
import { listGroups } from "../../api";
import { effectiveMessageCount, manualCommentsOf, wholeNumber, type ThreadKind } from "../../interactionPlan";
import { linkErrorVi } from "../../interactionErrors";
import type { DeviceGroup, DeviceInfo } from "../../types";
import { StatusChip } from "../WorkspacePrimitives";
import { MachineChoice } from "../MachineChoice";
import { Banner } from "../States";
import { AccountReadControl } from "./AccountReadControl";
import { InteractionPlanPreview } from "./InteractionPlanPreview";
import { InteractionSheetImport } from "./InteractionSheetImport";
import { ConversationEditor } from "./ConversationEditor";
import { InteractionThreshold } from "./InteractionThreshold";
import type { InteractionSetupTab } from "./InteractionSetupTab";
import "../../styles/interaction-workspace.css";

type Setup = ComponentProps<typeof InteractionSetupTab>;
const STEPS = ["Chọn bài viết", "Hành động & máy", "Kiểm tra & chạy"];
const ACTIONS = [["like", "Tim"], ["save", "Lưu"], ["comment", "Bình luận"]] as const;

/** Presentation only: parsing, planning, persistence and dispatch stay in the shell. */
export function InteractionWorkspaceSetup({ setup: p, profiles, scopeControl, effectiveActors, busy, onRun, onReparse }: {
  setup: Setup;
  profiles?: ReactNode;
  scopeControl?: ReactNode;
  effectiveActors: string[];
  busy: boolean;
  onRun: () => void;
  onReparse: () => void;
}) {
  const [step, setStep] = useState(1);
  const { draft, patch } = p;
  const targets = p.lines.flatMap((line) => line.target ? [line.target] : []);
  const actionOrder = ACTIONS.filter(([key]) => draft.actions[key]).map(([, label]) => label).join(" → ");
  const linkIssues = p.issues.filter((issue) => issue.field === "links");
  const linksReady = targets.length > 0 && !p.lines.some((line) => !line.target) && !p.linkBusy && !p.linkError && !linkIssues.length;
  const ready = linksReady && p.issues.length === 0;
  const stepIssues = step === 1 ? linkIssues : p.issues;
  const messages = effectiveMessageCount(draft, p.largestCohort);
  const assignmentCount = p.preview?.plan?.assignments.length ?? 0;
  const changeStep = (next: number) => {
    if (busy) return;
    setStep(next);
  };

  return <div className="interaction-wizard">
    {profiles && <details className="iw-profile-tools"><summary>Hồ sơ & cài đặt</summary>{profiles}</details>}
    <nav className="iw-steps" role="tablist" aria-label="Nội dung thiết lập Tương tác">
      {STEPS.map((label, index) => <button key={label} type="button" role="tab" disabled={busy}
        id={`iw-tab-${index + 1}`} aria-controls={`iw-panel-${index + 1}`} aria-selected={step === index + 1} tabIndex={step === index + 1 ? 0 : -1} onClick={() => changeStep(index + 1)}
        onKeyDown={event => { const next = event.key === "Home" ? 1 : event.key === "End" ? 3 : event.key === "ArrowRight" ? step % 3 + 1 : event.key === "ArrowLeft" ? (step + 1) % 3 + 1 : null; if (next) { event.preventDefault(); changeStep(next); document.getElementById(`iw-tab-${next}`)?.focus(); } }}>
        <span className="iw-step-number" aria-hidden="true">{step > index + 1 ? <Check size={14} /> : index + 1}</span><span>{label}</span>
      </button>)}
    </nav>
    <div className={`iw-stage iw-step-${step}`}>
      <div className="iw-step-content" role="tabpanel" id="iw-panel-1" aria-labelledby="iw-tab-1" hidden={step !== 1}>
        <section className="iw-panel iw-links" aria-label="Bài viết cần tương tác">
          <div className="iw-heading"><div><span className="automation-section-kicker">Nguồn nội dung</span><h3>Bài viết cần tương tác</h3></div></div>
          <InteractionSheetImport onApply={(urls) => patch("rawLinks", (previous) => [...new Set([...previous.split(/\r?\n/).map((line) => line.trim()).filter(Boolean), ...urls])].join("\n"))} />
          <label className="iw-field"><span>Link TikTok — mỗi dòng một link</span>
            <textarea value={draft.rawLinks} onChange={(event) => patch("rawLinks", event.target.value)} placeholder="Dán link TikTok, mỗi dòng một bài" rows={4} />
          </label>
          <div className="iw-link-tools"><span>Hỗ trợ link video và bài ảnh.</span><div>
            {p.lines.some((line) => line.error === "unresolvedShortLink") && <button type="button" className="ghost" disabled={p.linkBusy} onClick={p.onResolveShortLinks}>Gỡ link rút gọn</button>}
            <button type="button" className="ghost" disabled={p.linkBusy || !draft.rawLinks.trim()} onClick={onReparse}>{p.linkBusy ? "Đang kiểm tra…" : "Kiểm tra link"}</button>
          </div></div>
          {p.linkError && <Banner tone="error">{p.linkError}</Banner>}
          <div className="iw-table-scroll interaction-link-list" tabIndex={0} aria-label="Kết quả kiểm tra link">
            <table className="iw-table"><thead><tr><th>Bài viết</th><th>Loại</th><th>Kiểm tra</th></tr></thead><tbody>
              {p.lines.length ? p.lines.map((line) => <tr key={line.lineNo}>
                <td><strong>{line.target ? `@${line.target.author}` : `Dòng ${line.lineNo}`}</strong><small title={line.target?.normalizedUrl ?? line.original}>{line.target?.normalizedUrl ?? line.original}</small></td>
                <td>{line.target ? line.target.kind === "photo" ? "Bài ảnh" : "Video" : "—"}</td>
                <td><StatusChip tone={line.target ? "success" : "error"}>{line.target ? "Đúng định dạng" : linkErrorVi(line.error)}</StatusChip></td>
              </tr>) : <tr><td colSpan={3}><p className="iw-empty">{draft.rawLinks.trim() ? "Đang chờ kiểm tra link…" : "Dán link phía trên để bắt đầu."}</p></td></tr>}
            </tbody></table>
          </div>
        </section>
        <aside className="iw-panel iw-context" aria-label="Đầu vào và kết quả" tabIndex={0}>
          <span className="automation-section-kicker">Luồng thực hiện</span><h3>Đầu vào → Kết quả</h3>
          <ol>{[
            ["Bài viết cụ thể", "Hệ thống mở đúng link trên từng máy."],
            ["Hành động bạn chọn", "Tim, Lưu, Bình luận có thể bật riêng."],
            ["Kết quả từng máy", "Xem việc đã xác nhận, đã có sẵn hoặc cần kiểm tra."],
          ].map(([title, copy], index) => <li key={title}><span>{index + 1}</span><div><strong>{title}</strong><p>{copy}</p></div></li>)}</ol>
          <p className="iw-context-end">Nội dung thực tế được đọc trên máy trước khi tương tác.</p>
        </aside>
      </div>
      <div className="iw-step-content" role="tabpanel" id="iw-panel-2" aria-labelledby="iw-tab-2" hidden={step !== 2}>
        <section className="iw-panel iw-settings" aria-label="Hành động thực hiện">
          <div className="iw-heading"><div><span className="automation-section-kicker">Cấu hình</span><h3>Hành động thực hiện</h3></div><StatusChip>{targets.length} bài</StatusChip></div>
          <div className="iw-action-choices" role="group" aria-label="Hành động">
            {ACTIONS.map(([key, label]) => <label key={key} className={draft.actions[key] ? "selected" : ""}>
              <input type="checkbox" aria-label={label} checked={draft.actions[key]} disabled={draft.actions[key] && Object.values(draft.actions).filter(Boolean).length === 1}
                onChange={(event) => { const checked = event.target.checked; patch("actions", (previous) => ({ ...previous, [key]: checked })); }} />{label}
            </label>)}
          </div>
          <p className="iw-order"><span>Thứ tự</span><strong>{actionOrder}</strong></p>
          {draft.actions.comment ? <>
            <div className="iw-fields">
              {draft.textSource !== "script" && <label className="iw-field"><span>Cách bình luận</span><select value={draft.threadKind} onChange={(event) => patch("threadKind", event.target.value as ThreadKind)}>
                <option value="standalone">Riêng lẻ · mỗi máy một bình luận</option><option value="star">Cùng trả lời bình luận gốc</option><option value="chain">Trả lời nối tiếp</option>
              </select></label>}
              <label className="iw-field"><span>Nội dung bình luận</span><select value={draft.textSource} onChange={(event) => patch("textSource", event.target.value as "ai" | "manual" | "script")}><option value="ai">AI viết theo bài</option><option value="manual">Nội dung tự nhập</option><option value="script">Hội thoại theo kịch bản</option></select></label>
            </div>
            {draft.textSource === "script" ? <ConversationEditor draft={draft} onChange={value=>patch("conversationJson",value)} onRawChange={value=>patch("conversationRawJson",value)} targets={targets} devices={p.devices.filter(device=>effectiveActors.includes(device.udid))} handles={p.handles}/> : draft.textSource === "ai" ? <label className="iw-field"><span>Hướng dẫn giọng điệu cho AI</span><textarea rows={3} value={draft.instruction} onChange={(event) => patch("instruction", event.target.value)} /></label>
              : <label className="iw-field"><span>Danh sách bình luận — mỗi dòng một câu</span><textarea rows={4} value={draft.manualText} onChange={(event) => patch("manualText", event.target.value)} /><small>{manualCommentsOf(draft).length} câu · cần ít nhất {messages}</small></label>}
            <p className="iw-help">{draft.textSource === "script" ? "Các vai giữ đúng máy; mỗi link có hội thoại riêng và được thực hiện xen kẽ." : draft.threadKind === "standalone" ? "Mỗi máy tự mở bài và gửi bình luận riêng." : "Cần ít nhất 2 máy cùng loại. Một máy gửi gốc trước khi các máy còn lại trả lời."} {draft.textSource === "ai" && "AI chỉ gửi khi đọc đủ nội dung bài."}</p>
            {draft.textSource !== "script" && <button type="button" className="ghost iw-advanced-button" aria-expanded={p.advancedOpen} onClick={() => p.setAdvancedOpen(!p.advancedOpen)}>{p.advancedOpen ? "Ẩn tuỳ chỉnh nâng cao" : "Tuỳ chỉnh nâng cao"}</button>}
            {p.advancedOpen && draft.textSource !== "script" && <div className="iw-advanced">
              <div className="iw-fields">
                <label className="iw-field"><span>Số bình luận mỗi link</span><input type="number" min={draft.threadKind === "standalone" ? 1 : 2} max={64} placeholder={`${messages} · tự động`} value={draft.messageCount ?? ""} onChange={(event) => patch("messageCount", event.target.value === "" ? null : wholeNumber(event.target.value))} /></label>
                <label className="iw-field"><span>Số từ tối đa mỗi câu</span><input type="number" min={4} max={20} value={draft.maxWords} onChange={(event) => patch("maxWords", wholeNumber(event.target.value))} /></label>
              </div>
              <p className="iw-help">Để trống số bình luận để tự lấy bằng số máy đã chọn.</p>
              {draft.threadKind !== "standalone" && <label className="iw-checkbox"><input type="checkbox" checked={draft.mentionParent} onChange={(event) => patch("mentionParent", event.target.checked)} />Các máy tag nhau khi trả lời</label>}
              <label className="iw-field"><span>Tag thêm tài khoản (@handle)</span><input value={draft.mentionText} onChange={(event) => patch("mentionText", event.target.value)} placeholder="Cách nhau bằng dấu cách hoặc phẩy" /></label>
              {p.mentions.length > 0 && <p className="iw-help">{p.mentionActorCount} tài khoản đã gán khớp tag được thêm vào lượt chạy. Android chọn tag từ gợi ý; iPhone chỉ chèn chữ. Xem kết quả tại Theo dõi.</p>}
              <InteractionThreshold controls={p.threshold} />
            </div>}
          </> : <div className="iw-no-comment"><strong>Chỉ thực hiện {actionOrder.replace(" → ", " và ")}.</strong><p>Không tạo bình luận. Bạn có thể chạy với một máy.</p></div>}
        </section>
        <WorkspaceActors setup={p} effectiveActors={effectiveActors} scopeControl={scopeControl} />
      </div>
      <section className="iw-panel iw-review" role="tabpanel" id="iw-panel-3" aria-labelledby="iw-tab-3" aria-label="Kiểm tra lượt chạy" hidden={step !== 3}>
        <div className="iw-review-summary">
          <div><strong>{targets.length}</strong><span>bài viết</span></div><div><strong>{effectiveActors.length}</strong><span>máy thực hiện</span></div>
          <div><strong>{draft.actions.comment ? assignmentCount : targets.length * effectiveActors.length * Object.values(draft.actions).filter(Boolean).length}</strong><span>{draft.actions.comment ? "lượt bình luận dự kiến" : "hành động dự kiến"}</span></div>
          <div className="iw-review-order"><span>Trên mỗi bài, mỗi máy</span><strong>{actionOrder}</strong></div>
        </div>
        <div className="iw-table-scroll" tabIndex={0} aria-label="Máy và hành động đã chọn"><table className="iw-table"><thead><tr><th>Máy / tài khoản</th><th>Bài viết</th><th>Thực hiện</th><th>Chuẩn bị</th></tr></thead><tbody>
          {effectiveActors.map((udid) => <tr key={udid}><td><strong>{p.deviceNumber.get(udid)} · {p.deviceLabel.get(udid) ?? "Máy chưa đặt tên"}</strong><small>{p.handles[udid] ? `@${p.handles[udid].replace(/^@+/, "")}` : "Chưa gán tài khoản"}</small></td><td>{targets.length} bài</td><td>{actionOrder}</td><td><StatusChip tone={ready ? "success" : "warning"}>{ready ? "Đã lập kế hoạch" : "Cần kiểm tra"}</StatusChip></td></tr>)}
        </tbody></table></div>
        {draft.actions.comment && <details className="iw-plan"><summary>Thứ tự bình luận theo kế hoạch</summary><InteractionPlanPreview preview={p.preview} devices={p.devices} deviceNumber={p.deviceNumber} deviceLabel={p.deviceLabel} handles={p.handles} threadKind={draft.threadKind} commentEnabled /></details>}
        {p.warnings.length > 0 && <details className="iw-plan"><summary>{p.warnings.length} lưu ý trước khi chạy</summary>{p.warnings.map((warning) => <p key={warning}>{warning}</p>)}</details>}
        <p className="iw-run-note">Hành động sẽ thực hiện trên tài khoản đã chọn. Khi kết quả chưa rõ, lượt đó dừng để kiểm tra và không tự gửi lại.</p>
      </section>
    </div>
    {p.runError && <Banner tone="error">{p.runError}</Banner>}
    {step > 1 && stepIssues.length > 0 && <div className="iw-issues" role="status"><ul>{stepIssues.map((issue) => <li key={`${issue.field}:${issue.message}`}>{issue.message}{issue.fix && <button type="button" className="ghost" onClick={() => patch("messageCount", issue.fix!.messageCount)}>{issue.fix.label}</button>}{issue.technicalDetail && <details><summary>Chi tiết</summary><code>{issue.technicalDetail}</code></details>}</li>)}</ul></div>}
    <footer className="iw-footer"><div><strong>{targets.length} bài · {effectiveActors.length} máy</strong><span>{step === 1 ? "Bước tiếp theo: chọn hành động và máy." : step === 2 ? actionOrder : "Kết quả được ghi riêng cho từng máy."}</span></div><div>
      {step > 1 && <button type="button" className="ghost" disabled={busy} onClick={() => setStep(step - 1)}>Quay lại</button>}
      <button type="button" className="primary" disabled={busy || (step === 1 ? !linksReady : !ready)} onClick={() => step === 3 ? onRun() : setStep(step + 1)}>{busy ? "Đang bắt đầu…" : step === 1 ? "Chọn hành động & máy →" : step === 2 ? "Kiểm tra lượt chạy →" : "Bắt đầu tương tác"}</button>
    </div></footer>
  </div>;
}

function WorkspaceActors({ setup: p, effectiveActors, scopeControl }: { setup: Setup; effectiveActors: string[]; scopeControl?: ReactNode }) {
  const [query, setQuery] = useState("");
  const [groups, setGroups] = useState<DeviceGroup[]>([]);
  const [account, setAccount] = useState<string | null>(null);
  const choices = [...p.pixelActors, ...p.hierarchyActors].sort((a, b) => (p.deviceNumber.get(a.udid) ?? 0) - (p.deviceNumber.get(b.udid) ?? 0));
  const thread = p.draft.actions.comment && p.draft.threadKind !== "standalone";
  useEffect(() => {
    if (!thread) return;
    let live = true;
    void listGroups().then((next) => { if (live) setGroups(next); }).catch(() => undefined);
    return () => { live = false; };
  }, [thread]);
  const filtered = choices.filter((device) => [p.deviceLabel.get(device.udid), p.deviceNumber.get(device.udid), p.handles[device.udid]].join(" ").toLocaleLowerCase("vi").includes(query.toLocaleLowerCase("vi")));
  const inspected = choices.find((device) => device.udid === account);
  const replaceActors = (devices: DeviceInfo[]) => p.patch("actors", devices.map((device) => device.udid));
  return <section className="iw-panel iw-machines" tabIndex={0} aria-label="Máy thực hiện">
    <div className="iw-heading"><div><span className="automation-section-kicker">Phạm vi</span><h2>Máy thực hiện</h2></div><span className="machine-select-count" role="status">Đã chọn {effectiveActors.length}</span></div>
    <label className="iw-search"><Search size={16} aria-hidden="true" /><input type="search" aria-label="Tìm máy Tương tác" placeholder="Tìm số máy hoặc tên" value={query} onChange={(event) => setQuery(event.target.value)} /></label>
    <div className="iw-picker-tools"><button type="button" className="ghost" title="Chọn tất cả máy sẵn sàng trong phạm vi, kể cả ngoài kết quả tìm kiếm" disabled={!choices.some(device => device.status === "ready")} onClick={() => replaceActors(choices.filter((device) => device.status === "ready"))}>Chọn tất cả sẵn sàng</button><button type="button" className="ghost" disabled={!p.draft.actors.length} onClick={() => replaceActors([])}>Bỏ chọn</button>{scopeControl}<small>{choices.filter(d => d.status === "ready").length} sẵn sàng · {choices.length} tổng</small></div>
    {thread && groups.length > 0 && <label className="iw-field"><span>Lấy từ nhóm</span><select value="" onChange={(event) => { const group = groups.find((entry) => entry.id === event.target.value); if (group) replaceActors(choices.filter((device) => group.udids.includes(device.udid))); }}><option value="">Chọn nhóm…</option>{groups.map((group) => <option key={group.id} value={group.id}>{group.name} ({group.udids.length})</option>)}</select></label>}
    <div className={`iw-machine-scroll machine-choice-grid${choices.length > 12 ? " is-compact" : ""}`} role="group" aria-label="Danh sách máy thực hiện">
      {filtered.length ? filtered.map((device) => {
        const name = p.deviceLabel.get(device.udid) ?? device.name;
        const tagged = !p.draft.actors.includes(device.udid) && effectiveActors.includes(device.udid);
        const checked = effectiveActors.includes(device.udid);
        return <MachineChoice key={device.udid} number={p.deviceNumber.get(device.udid) ?? 0} name={name} status={device.status} reason={device.lastError}
          label={name} checked={checked} disabled={tagged || (device.status !== "ready" && !checked)}
          title={tagged ? "Được thêm bởi tag tài khoản; sửa tag trong tuỳ chỉnh nâng cao" : undefined}
          onChange={() => p.patch("actors", (previous) => previous.includes(device.udid) ? previous.filter((id) => id !== device.udid) : [...previous, device.udid])}
          detail={p.draft.actions.comment ? <button type="button" className="iw-account-link" aria-label={`Tài khoản TikTok của ${name}`} onClick={() => setAccount(account === device.udid ? null : device.udid)}>{p.handles[device.udid] ? `@${p.handles[device.udid].replace(/^@+/, "")}` : "Gán tài khoản"}{tagged ? " · từ tag" : ""}</button> : <span>{device.platform === "android" ? "Android" : "iPhone"}</span>} />;
      }) : <p className="iw-empty">{choices.length ? "Không có máy khớp tìm kiếm." : "Chưa có máy trong phạm vi đã chọn."}</p>}
    </div>
    {inspected && p.draft.actions.comment && <div className="iw-account-editor" aria-label="Chỉnh tài khoản máy">
      <div className="iw-heading"><strong>{p.deviceLabel.get(inspected.udid)}</strong><button type="button" className="ghost" onClick={() => setAccount(null)}>Đóng tài khoản</button></div>
      <label className="iw-field"><span>Nick đã gán</span><input spellCheck={false} value={p.handles[inspected.udid] ?? ""} disabled={p.savingHandles?.[inspected.udid]} aria-invalid={Boolean(p.handleErrors?.[inspected.udid])} onChange={(event) => p.onHandleChange(inspected.udid, event.target.value)} onBlur={(event) => p.onHandleBlur(inspected.udid, event.target.value)} /></label>
      <small>Nick do bạn gán; chưa xác nhận tài khoản đang đăng nhập.</small>
      {p.savingHandles?.[inspected.udid] && <small role="status">Đang lưu nick…</small>}
      {p.handleErrors?.[inspected.udid] && <><small role="alert">{p.handleErrors[inspected.udid]}</small><button type="button" className="ghost" onClick={() => p.onHandleReload?.(inspected.udid)}>Tải lại nick đã lưu</button></>}
      {inspected.platform === "android" && <AccountReadControl udid={inspected.udid} handle={p.handles[inspected.udid] ?? ""} disabled={Boolean(p.savingHandles?.[inspected.udid] || p.handleErrors?.[inspected.udid])} />}
    </div>}
  </section>;
}
