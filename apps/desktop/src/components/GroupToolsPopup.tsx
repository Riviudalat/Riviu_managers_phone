import { useEffect, useMemo, useRef, useState } from "react";

import type { DeviceInfo, HardwareKey } from "../types";
import {
  distributeFiles,
  distributeText,
  factoryReset,
  groupInput,
  isRooted,
  listSerialPorts,
  relayPulseChannel,
  relaySetChannel,
  rootShell,
  setDeviceIdentity,
  setMockLocation,
  setScreenLocked,
  setWallpaper,
  setWallpaperBytes,
  stopMockLocation,
  type SerialPortInfo,
} from "../api";
import { requestConfirm } from "../confirmStore";
import {
  defaultGamepadBindings,
  REFERENCE,
  resolveButtonAction,
  risingEdges,
  toReference,
  type PeripheralAction,
} from "../peripheralMap";
import { pickFiles } from "../pickFile";
import { assign, leftover, splitText, type SplitMode } from "../textDistribution";
import { groupInputOutcome } from "../groupInput";
import { getGroupSync } from "../groupSync";
import {
  addQuickPhrase,
  DEFAULT_QUICK_GROUP,
  exportQuickPhrases,
  groupsOf,
  importQuickPhrases,
  loadQuickPhrases,
  phrasesInGroup,
  removeQuickPhrase,
  storeQuickPhrases,
  type QuickPhrase,
} from "../quickPhrases";
import { expand, stepSummary, totalWaitMs, type Macro } from "../macro";
import {
  clearRecording,
  deleteMacro,
  recordedSteps,
  saveMacro,
  startRecording,
  stopRecording,
  useMacroRecording,
  useRecordedSteps,
  useSavedMacros,
} from "../macroStore";
import { targetsOf } from "./SelectionStrip";
import { pushToast, toastError } from "../toastStore";
import { IconClose } from "./Icons";
import { describeError } from "../describeError";

interface Props {
  devices: DeviceInfo[];
  selected: string[];
  onClose: () => void;
}

type Tool = "text" | "files" | "reply" | "keys" | "macro" | "gps" | "root" | "peripherals";

const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

function newId(): string {
  try {
    return crypto.randomUUID();
  } catch {
    // A webview old enough to lack randomUUID still needs a unique-enough id.
    return `qp-${Date.now()}-${Math.round(Math.random() * 1e9)}`;
  }
}

/**
 * Group Tools — batch operations scoped to the current selection (xiaowei device
 * context-menu tools). Text Distribution (A2) and Quick Replies (A6) so far; more tools dock
 * into the same tabbed panel as they land.
 */
export function GroupToolsPopup({ devices, selected, onClose }: Props) {
  const [tool, setTool] = useState<Tool>("text");
  const targets = useMemo(() => targetsOf(selected, devices), [selected, devices]);
  const targetDevices = useMemo(
    () =>
      targets
        .map((udid) => devices.find((d) => d.udid === udid))
        .filter((d): d is DeviceInfo => Boolean(d)),
    [targets, devices],
  );
  const scopeLabel = selected.length ? `${selected.length} máy` : `Tất cả ${devices.length}`;

  return (
    <div className="nurture-float-layer" aria-label="Công cụ nhóm">
      <div className="nurture-float group-tools">
        <div className="nurture-float-title" style={{ cursor: "default" }}>
          <strong>Công cụ nhóm</strong>
          <span className="hint">{scopeLabel}</span>
          <div className="grow" />
          <button type="button" className="close" title="Đóng" onClick={onClose}>
            <IconClose size={14} />
          </button>
        </div>

        <div className="nurture-float-body">
          <div className="group-tools-tabs">
            <button
              type="button"
              className={`tb-btn ${tool === "text" ? "active" : ""}`}
              onClick={() => setTool("text")}
            >
              Phân phối văn bản
            </button>
            <button
              type="button"
              className={`tb-btn ${tool === "files" ? "active" : ""}`}
              onClick={() => setTool("files")}
            >
              Phân phối tệp
            </button>
            <button
              type="button"
              className={`tb-btn ${tool === "reply" ? "active" : ""}`}
              onClick={() => setTool("reply")}
            >
              Câu trả lời nhanh
            </button>
            <button
              type="button"
              className={`tb-btn ${tool === "keys" ? "active" : ""}`}
              onClick={() => setTool("keys")}
            >
              Thao tác nhanh
            </button>
            <button
              type="button"
              className={`tb-btn ${tool === "macro" ? "active" : ""}`}
              onClick={() => setTool("macro")}
            >
              Macro
            </button>
            <button
              type="button"
              className={`tb-btn ${tool === "gps" ? "active" : ""}`}
              onClick={() => setTool("gps")}
            >
              Vị trí (GPS)
            </button>
            <button
              type="button"
              className={`tb-btn ${tool === "root" ? "active" : ""}`}
              onClick={() => setTool("root")}
            >
              Root / Máy mới
            </button>
            <button
              type="button"
              className={`tb-btn ${tool === "peripherals" ? "active" : ""}`}
              onClick={() => setTool("peripherals")}
            >
              Ngoại vi
            </button>
          </div>

          {tool === "text" && (
            <TextDistributionTool devices={devices} targets={targets} targetDevices={targetDevices} />
          )}
          {tool === "files" && (
            <FileDistributionTool devices={devices} targets={targets} targetDevices={targetDevices} />
          )}
          {tool === "reply" && <QuickReplyTool targets={targets} scopeLabel={scopeLabel} />}
          {tool === "keys" && <QuickActionsTool targets={targets} scopeLabel={scopeLabel} />}
          {tool === "macro" && <MacroTool targets={targets} scopeLabel={scopeLabel} />}
          {tool === "gps" && <GpsTool targets={targets} scopeLabel={scopeLabel} />}
          {tool === "root" && <RootTool targets={targets} scopeLabel={scopeLabel} />}
          {tool === "peripherals" && <PeripheralsTool targets={targets} scopeLabel={scopeLabel} />}
        </div>
      </div>
    </div>
  );
}

function TextDistributionTool({
  devices,
  targets,
  targetDevices,
}: {
  devices: DeviceInfo[];
  targets: string[];
  targetDevices: DeviceInfo[];
}) {
  const [raw, setRaw] = useState("");
  const [modeKind, setModeKind] = useState<SplitMode["kind"]>("lines");
  const [separator, setSeparator] = useState(",");
  const [pattern, setPattern] = useState("\\s*\\n\\s*");
  const [busy, setBusy] = useState(false);

  const mode: SplitMode = useMemo(() => {
    if (modeKind === "separator") return { kind: "separator", separator };
    if (modeKind === "regex") return { kind: "regex", pattern };
    return { kind: "lines" };
  }, [modeKind, separator, pattern]);

  const { items, error } = useMemo(() => {
    try {
      return { items: splitText(raw, mode), error: null as string | null };
    } catch (e) {
      return { items: [] as string[], error: describeError(e) };
    }
  }, [raw, mode]);

  const pairs = useMemo(() => assign(items, targets), [items, targets]);
  const spare = useMemo(() => leftover(items, targets), [items, targets]);

  const nameFor = (udid: string): string => {
    const d = devices.find((x) => x.udid === udid);
    return d?.name || d?.model || udid.slice(-6);
  };

  const send = async () => {
    if (!pairs.length) {
      pushToast("warn", "Chưa có gì để gửi", "Cần văn bản đã tách và ít nhất một máy.");
      return;
    }
    setBusy(true);
    try {
      const report = await distributeText(pairs);
      const outcome = groupInputOutcome(report);
      if (outcome.kind === "ok") pushToast("ok", "Đã phân phối văn bản", `${pairs.length} máy`);
      else if (outcome.kind === "partial") pushToast("warn", outcome.title, outcome.detail);
      else pushToast("error", outcome.title, outcome.detail);
    } catch (e) {
      toastError("Phân phối văn bản thất bại", e);
    } finally {
      setBusy(false);
    }
  };

  return (
    <>
      <p className="hint">
        Chia một khối văn bản thành nhiều phần và gõ mỗi phần vào một máy, theo thứ tự máy đang
        hiển thị. Đường gõ Unicode (dấu tiếng Việt an toàn), chạy cả Android lẫn iOS.
      </p>
      <div className="group-tools-field">
        <label className="hint" htmlFor="gt-text">
          Văn bản
        </label>
        <textarea
          id="gt-text"
          value={raw}
          placeholder={"Mỗi dòng một máy…\nXin chào\nBạn khỏe không"}
          onChange={(e) => setRaw(e.target.value)}
        />
      </div>
      <div className="row">
        <label>
          Cách tách
          <select value={modeKind} onChange={(e) => setModeKind(e.target.value as SplitMode["kind"])}>
            <option value="lines">Theo dòng</option>
            <option value="separator">Ký tự phân tách</option>
            <option value="regex">Biểu thức chính quy</option>
          </select>
        </label>
        {modeKind === "separator" && (
          <label>
            Ký tự
            <input type="text" value={separator} onChange={(e) => setSeparator(e.target.value)} />
          </label>
        )}
        {modeKind === "regex" && (
          <label>
            Mẫu regex
            <input type="text" value={pattern} onChange={(e) => setPattern(e.target.value)} />
          </label>
        )}
      </div>
      {error && <p className="error">Regex không hợp lệ: {error}</p>}
      <p className="hint">
        {items.length} phần → {pairs.length}/{targetDevices.length} máy nhận
        {spare.extraItems > 0 && ` · dư ${spare.extraItems} phần`}
        {spare.extraDevices > 0 && ` · thiếu cho ${spare.extraDevices} máy`}
      </p>
      {pairs.length > 0 && (
        <div className="group-tools-preview">
          {pairs.map((p, i) => (
            <div className="row-item" key={p.udid}>
              <span className="who">
                #{i + 1} {nameFor(p.udid)}
              </span>
              <span className="what">{p.text}</span>
            </div>
          ))}
        </div>
      )}
      <div className="nurture-float-actions" style={{ marginTop: "0.7rem" }}>
        <button
          type="button"
          className="primary"
          disabled={busy || !pairs.length || Boolean(error)}
          onClick={() => void send()}
        >
          {busy ? "Đang gửi…" : `Gửi tới ${pairs.length} máy`}
        </button>
      </div>
    </>
  );
}

function QuickReplyTool({ targets, scopeLabel }: { targets: string[]; scopeLabel: string }) {
  const [phrases, setPhrases] = useState<QuickPhrase[]>(() => loadQuickPhrases());
  const groups = useMemo(() => groupsOf(phrases), [phrases]);
  const [group, setGroup] = useState<string>(DEFAULT_QUICK_GROUP);
  const activeGroup = groups.includes(group) ? group : groups[0] ?? DEFAULT_QUICK_GROUP;
  const inGroup = useMemo(() => phrasesInGroup(phrases, activeGroup), [phrases, activeGroup]);

  const [newName, setNewName] = useState("");
  const [newContent, setNewContent] = useState("");
  const [newGroup, setNewGroup] = useState("");
  const [io, setIo] = useState("");
  const [busy, setBusy] = useState(false);

  const commit = (next: QuickPhrase[]) => {
    setPhrases(next);
    storeQuickPhrases(next);
  };

  const add = () => {
    const result = addQuickPhrase(phrases, newName, newContent, newId(), newGroup || activeGroup);
    if (result.error) {
      pushToast("warn", "Không thêm được", result.error);
      return;
    }
    commit(result.phrases);
    setNewName("");
    setNewContent("");
  };

  const del = (id: string) => commit(removeQuickPhrase(phrases, id));

  const send = async (phrase: QuickPhrase) => {
    if (!targets.length) {
      pushToast("warn", "Chưa có máy", "Chọn máy rồi gửi.");
      return;
    }
    setBusy(true);
    try {
      const report = await groupInput({
        udids: targets,
        kind: "type",
        text: phrase.content,
        sync: getGroupSync(),
      });
      const outcome = groupInputOutcome(report);
      if (outcome.kind === "ok") pushToast("ok", "Đã gõ câu trả lời", phrase.name);
      else if (outcome.kind === "partial") pushToast("warn", outcome.title, outcome.detail);
      else pushToast("error", outcome.title, outcome.detail);
    } catch (e) {
      toastError("Gõ câu trả lời thất bại", e);
    } finally {
      setBusy(false);
    }
  };

  const doExport = () => setIo(exportQuickPhrases(phrases));
  const doImport = () => {
    if (!io.trim()) {
      pushToast("warn", "Chưa có gì để nhập", "Dán nội dung xuất ra vào ô rồi bấm Nhập.");
      return;
    }
    const out = importQuickPhrases(phrases, io, newId);
    commit(out.phrases);
    pushToast("ok", "Đã nhập câu trả lời", `Thêm ${out.added}, bỏ ${out.skipped}`);
  };

  return (
    <>
      <p className="hint">
        Kho câu trả lời có nhóm. Bấm "Gửi" để gõ một câu vào {scopeLabel}. Xuất/nhập bằng cách
        dán văn bản (mỗi dòng: nhóm ⇥ tên ⇥ nội dung).
      </p>
      <div className="row">
        <label>
          Nhóm
          <select value={activeGroup} onChange={(e) => setGroup(e.target.value)}>
            {groups.length === 0 && <option value={DEFAULT_QUICK_GROUP}>{DEFAULT_QUICK_GROUP}</option>}
            {groups.map((g) => (
              <option key={g} value={g}>
                {g} ({phrasesInGroup(phrases, g).length})
              </option>
            ))}
          </select>
        </label>
      </div>

      <div className="group-tools-preview">
        {inGroup.length === 0 ? (
          <div className="row-item">
            <span className="hint">Nhóm này chưa có câu nào.</span>
          </div>
        ) : (
          inGroup.map((p) => (
            <div className="row-item" key={p.id}>
              <span className="who" title={p.name}>
                {p.name}
              </span>
              <span className="what">{p.content}</span>
              <span className="grow" />
              <button
                type="button"
                className="ghost"
                disabled={busy}
                onClick={() => void send(p)}
                title="Gõ câu này vào các máy đang chọn"
              >
                Gửi
              </button>
              <button type="button" className="ghost" onClick={() => del(p.id)} title="Xoá câu">
                Xoá
              </button>
            </div>
          ))
        )}
      </div>

      <div className="group-tools-field" style={{ marginTop: "0.6rem" }}>
        <label className="hint">Thêm câu mới</label>
        <div className="row">
          <input
            type="text"
            placeholder="Nhóm (trống = đang chọn)"
            value={newGroup}
            onChange={(e) => setNewGroup(e.target.value)}
          />
          <input
            type="text"
            placeholder="Tên (tuỳ chọn)"
            value={newName}
            onChange={(e) => setNewName(e.target.value)}
          />
        </div>
        <textarea
          placeholder="Nội dung câu trả lời…"
          value={newContent}
          onChange={(e) => setNewContent(e.target.value)}
          style={{ minHeight: 64 }}
        />
        <div className="nurture-float-actions">
          <button type="button" className="primary" onClick={add} disabled={!newContent.trim()}>
            Thêm câu
          </button>
        </div>
      </div>

      <details style={{ marginTop: "0.5rem" }}>
        <summary className="hint">Xuất / Nhập</summary>
        <textarea
          value={io}
          placeholder={"Dán để Nhập, hoặc bấm Xuất để đổ ra đây…"}
          onChange={(e) => setIo(e.target.value)}
          style={{ minHeight: 80, marginTop: "0.4rem" }}
        />
        <div className="nurture-float-actions">
          <button type="button" className="ghost" onClick={doExport}>
            Xuất ra ô
          </button>
          <button type="button" className="ghost" onClick={doImport}>
            Nhập từ ô
          </button>
        </div>
      </details>
    </>
  );
}

function QuickActionsTool({ targets, scopeLabel }: { targets: string[]; scopeLabel: string }) {
  const [busy, setBusy] = useState<string | null>(null);

  const KEYS: { label: string; key: HardwareKey }[] = [
    { label: "Home", key: "home" },
    { label: "Back", key: "back" },
    { label: "Đa nhiệm", key: "recents" },
    { label: "Nguồn (khoá/mở)", key: "power" },
    { label: "Âm lượng +", key: "volumeUp" },
    { label: "Âm lượng −", key: "volumeDown" },
    { label: "Thông báo", key: "notification" },
  ];

  const fire = async (label: string, key: HardwareKey) => {
    if (!targets.length) {
      pushToast("warn", "Chưa có máy", "Chọn máy rồi thao tác.");
      return;
    }
    setBusy(key);
    try {
      const report = await groupInput({ udids: targets, kind: "key", key, sync: getGroupSync() });
      const outcome = groupInputOutcome(report);
      if (outcome.kind === "ok") pushToast("ok", label, `${targets.length} máy`);
      else if (outcome.kind === "partial") pushToast("warn", outcome.title, outcome.detail);
      else pushToast("error", outcome.title, outcome.detail);
    } catch (e) {
      toastError(`${label} thất bại`, e);
    } finally {
      setBusy(null);
    }
  };

  const numberWallpapers = async () => {
    if (!targets.length) {
      pushToast("warn", "Chưa có máy", "Chọn máy rồi đặt hình nền.");
      return;
    }
    setBusy("wall-num");
    const results = await Promise.allSettled(
      targets.map(async (udid, i) => {
        const png = await numberWallpaperPng(String(i + 1));
        await setWallpaperBytes(udid, Array.from(png));
      }),
    );
    setBusy(null);
    const ok = results.filter((r) => r.status === "fulfilled").length;
    if (ok === targets.length) pushToast("ok", "Đã đặt số làm hình nền", `${ok} máy`);
    else
      pushToast(
        "warn",
        `Đặt hình nền ${ok}/${targets.length} máy`,
        "Máy còn lại cần Riviu helper.",
      );
  };

  const lock = async (locked: boolean) => {
    if (!targets.length) {
      pushToast("warn", "Chưa có máy", "Chọn máy rồi thao tác.");
      return;
    }
    setBusy(locked ? "lock" : "unlock");
    const results = await Promise.allSettled(targets.map((u) => setScreenLocked(u, locked)));
    setBusy(null);
    const ok = results.filter((r) => r.status === "fulfilled").length;
    const label = locked ? "Đã khoá màn hình" : "Đã mở khoá";
    if (ok === targets.length) pushToast("ok", label, `${ok} máy`);
    else
      pushToast("warn", `${label} ${ok}/${targets.length} máy`, "Máy còn lại không hỗ trợ hoặc bận.");
  };

  const customWallpaper = async () => {
    if (!targets.length) {
      pushToast("warn", "Chưa có máy", "Chọn máy rồi đặt hình nền.");
      return;
    }
    const picked = await pickFiles({
      title: "Chọn ảnh nền chung",
      filters: [{ name: "Ảnh", extensions: ["jpg", "jpeg", "png", "webp"] }],
    });
    if (!picked.length) return;
    const path = picked[0];
    setBusy("wall-img");
    const results = await Promise.allSettled(targets.map((udid) => setWallpaper(udid, path)));
    setBusy(null);
    const ok = results.filter((r) => r.status === "fulfilled").length;
    if (ok === targets.length) pushToast("ok", "Đã đặt ảnh nền", `${ok} máy`);
    else pushToast("warn", `Đặt ảnh nền ${ok}/${targets.length} máy`, "Máy còn lại cần Riviu helper.");
  };

  return (
    <>
      <p className="hint">
        Bấm một phím phần cứng cho {scopeLabel} cùng lúc (áp cả độ trễ/so le nếu đã bật ở Cài
        đặt). "Nguồn" bật/tắt màn hình luân phiên.
      </p>
      <div className="group-tools-keys">
        {KEYS.map((k) => (
          <button
            type="button"
            key={k.key}
            className="tb-btn"
            disabled={busy !== null}
            onClick={() => void fire(k.label, k.key)}
          >
            {busy === k.key ? "…" : k.label}
          </button>
        ))}
      </div>
      <p className="hint" style={{ marginTop: "0.7rem" }}>
        Khoá / mở khoá màn hình đồng loạt (iOS qua WDA; Android tắt/bật màn hình). Máy đặt mã
        PIN sẽ dừng ở màn khoá của nó — đây là bật/tắt màn, không phải vượt khoá.
      </p>
      <div className="nurture-float-actions">
        <button type="button" className="ghost" disabled={busy !== null} onClick={() => void lock(true)}>
          {busy === "lock" ? "…" : "Khoá màn hình"}
        </button>
        <button type="button" className="ghost" disabled={busy !== null} onClick={() => void lock(false)}>
          {busy === "unlock" ? "…" : "Mở khoá"}
        </button>
      </div>
      <p className="hint" style={{ marginTop: "0.7rem" }}>
        Hình nền (Android, cần Riviu helper) — đánh số máy để nhận diện, hoặc đặt một ảnh
        chung.
      </p>
      <div className="nurture-float-actions">
        <button
          type="button"
          className="ghost"
          disabled={busy !== null}
          onClick={() => void numberWallpapers()}
        >
          {busy === "wall-num" ? "…" : "Đặt số làm hình nền"}
        </button>
        <button
          type="button"
          className="ghost"
          disabled={busy !== null}
          onClick={() => void customWallpaper()}
        >
          {busy === "wall-img" ? "…" : "Chọn ảnh nền chung…"}
        </button>
      </div>
    </>
  );
}

function FileDistributionTool({
  devices,
  targets,
  targetDevices,
}: {
  devices: DeviceInfo[];
  targets: string[];
  targetDevices: DeviceInfo[];
}) {
  const [paths, setPaths] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);

  const pairs = useMemo(
    () => assign(paths, targets).map((a) => ({ udid: a.udid, path: a.text })),
    [paths, targets],
  );
  const spare = useMemo(() => leftover(paths, targets), [paths, targets]);

  const nameFor = (udid: string): string => {
    const d = devices.find((x) => x.udid === udid);
    return d?.name || d?.model || udid.slice(-6);
  };
  const baseName = (p: string): string => p.split(/[/\\]/).pop() || p;

  const pick = async () => {
    const picked = await pickFiles({
      title: "Chọn tệp phân phối (mỗi máy một tệp, theo thứ tự)",
      filters: [
        { name: "Media", extensions: ["jpg", "jpeg", "png", "gif", "webp", "heic", "mp4", "mov", "m4v"] },
        { name: "Tất cả", extensions: ["*"] },
      ],
    });
    if (picked.length) setPaths(picked);
  };

  const send = async () => {
    if (!pairs.length) {
      pushToast("warn", "Chưa có gì để gửi", "Chọn tệp và ít nhất một máy.");
      return;
    }
    setBusy(true);
    try {
      const report = await distributeFiles(pairs);
      const outcome = groupInputOutcome(report);
      if (outcome.kind === "ok") pushToast("ok", "Đã phân phối tệp", `${pairs.length} máy`);
      else if (outcome.kind === "partial") pushToast("warn", outcome.title, outcome.detail);
      else pushToast("error", outcome.title, outcome.detail);
    } catch (e) {
      toastError("Phân phối tệp thất bại", e);
    } finally {
      setBusy(false);
    }
  };

  return (
    <>
      <p className="hint">
        Đưa mỗi tệp vào một máy (theo thứ tự máy đang hiển thị), lưu vào thư viện ảnh/video của
        máy. Chọn nhiều tệp một lần; tệp thứ i vào máy thứ i.
      </p>
      <div className="nurture-float-actions">
        <button type="button" className="ghost" onClick={() => void pick()}>
          Chọn tệp…
        </button>
      </div>
      <p className="hint">
        {paths.length} tệp → {pairs.length}/{targetDevices.length} máy nhận
        {spare.extraItems > 0 && ` · dư ${spare.extraItems} tệp`}
        {spare.extraDevices > 0 && ` · thiếu cho ${spare.extraDevices} máy`}
      </p>
      {pairs.length > 0 && (
        <div className="group-tools-preview">
          {pairs.map((p, i) => (
            <div className="row-item" key={p.udid}>
              <span className="who">
                #{i + 1} {nameFor(p.udid)}
              </span>
              <span className="what">{baseName(p.path)}</span>
            </div>
          ))}
        </div>
      )}
      <div className="nurture-float-actions" style={{ marginTop: "0.7rem" }}>
        <button type="button" className="primary" disabled={busy || !pairs.length} onClick={() => void send()}>
          {busy ? "Đang gửi…" : `Gửi tới ${pairs.length} máy`}
        </button>
      </div>
    </>
  );
}

function MacroTool({ targets, scopeLabel }: { targets: string[]; scopeLabel: string }) {
  const recording = useMacroRecording();
  const steps = useRecordedSteps();
  const macros = useSavedMacros();
  const [name, setName] = useState("");
  const [loops, setLoops] = useState(1);
  const [playing, setPlaying] = useState<string | null>(null);
  const stopRef = useRef(false);

  const save = () => {
    const macro = saveMacro(name, recordedSteps());
    if (macro) {
      pushToast("ok", "Đã lưu macro", `${macro.name} · ${macro.steps.length} bước`);
      setName("");
    } else {
      pushToast("warn", "Chưa có bước nào", "Bật ghi rồi thao tác trên overlay điều khiển.");
    }
  };

  const play = async (macro: Macro) => {
    if (!targets.length) {
      pushToast("warn", "Chưa có máy", "Chọn máy rồi chạy.");
      return;
    }
    const plan = expand(macro.steps, loops);
    setPlaying(macro.id);
    stopRef.current = false;
    let failed = 0;
    try {
      for (const step of plan) {
        if (stopRef.current) break;
        try {
          if (step.kind === "tap") {
            await groupInput({
              udids: targets,
              kind: "tap",
              x: step.x,
              y: step.y,
              imageW: step.iw,
              imageH: step.ih,
              sync: getGroupSync(),
            });
          } else if (step.kind === "swipe") {
            await groupInput({
              udids: targets,
              kind: "swipe",
              x: step.x,
              y: step.y,
              toX: step.toX,
              toY: step.toY,
              imageW: step.iw,
              imageH: step.ih,
              sync: getGroupSync(),
            });
          } else if (step.kind === "key") {
            await groupInput({ udids: targets, kind: "key", key: step.key, sync: getGroupSync() });
          }
        } catch {
          failed += 1;
        }
        if (step.afterMs > 0 && !stopRef.current) await sleep(step.afterMs);
      }
      if (stopRef.current) pushToast("warn", "Đã dừng macro", macro.name);
      else if (failed) pushToast("warn", "Macro chạy xong (có lỗi)", `${failed} bước lỗi`);
      else pushToast("ok", "Đã chạy macro", `${macro.name} × ${loops}`);
    } finally {
      setPlaying(null);
    }
  };

  return (
    <>
      <p className="hint">
        Ghi thao tác trên một máy rồi phát lại cho {scopeLabel}. Bật ghi, mở "Mở điều khiển"
        một máy rồi chạm/vuốt/bấm phím — mỗi bước được ghi theo toạ độ ảnh và phát lại đúng vị
        trí trên từng máy (kèm delay/offset nếu bật).
      </p>
      <div className="nurture-float-actions">
        {recording ? (
          <button type="button" className="primary" onClick={() => stopRecording()}>
            Dừng ghi ({steps.length})
          </button>
        ) : (
          <button type="button" className="primary" onClick={() => startRecording()}>
            Bắt đầu ghi
          </button>
        )}
        <button
          type="button"
          className="ghost"
          disabled={!steps.length}
          onClick={() => clearRecording()}
        >
          Xoá bản ghi
        </button>
      </div>
      {steps.length > 0 && (
        <>
          <div className="group-tools-preview" style={{ maxHeight: 140 }}>
            {steps.map((s, i) => (
              <div className="row-item" key={i}>
                <span className="who">#{i + 1}</span>
                <span className="what">{stepSummary(s)}</span>
              </div>
            ))}
          </div>
          <div className="row" style={{ marginTop: "0.4rem" }}>
            <input
              type="text"
              placeholder="Tên macro"
              value={name}
              onChange={(e) => setName(e.target.value)}
            />
            <button type="button" className="ghost" disabled={recording} onClick={save}>
              Lưu macro
            </button>
          </div>
        </>
      )}

      <div className="row" style={{ marginTop: "0.6rem" }}>
        <label>
          Số vòng lặp
          <input
            type="number"
            min={1}
            value={loops}
            onChange={(e) => setLoops(Math.max(1, Math.round(Number(e.target.value) || 1)))}
          />
        </label>
      </div>
      <p className="hint">Macro đã lưu ({macros.length})</p>
      <div className="group-tools-preview">
        {macros.length === 0 ? (
          <div className="row-item">
            <span className="hint">Chưa có macro nào.</span>
          </div>
        ) : (
          macros.map((m) => (
            <div className="row-item" key={m.id}>
              <span className="who" title={m.name}>
                {m.name}
              </span>
              <span className="what">
                {m.steps.length} bước · ~{Math.round(totalWaitMs(m.steps, loops) / 100) / 10}s chờ
              </span>
              <span className="grow" />
              {playing === m.id ? (
                <button
                  type="button"
                  className="ghost"
                  onClick={() => {
                    stopRef.current = true;
                  }}
                >
                  Dừng
                </button>
              ) : (
                <button
                  type="button"
                  className="ghost"
                  disabled={playing !== null || recording}
                  onClick={() => void play(m)}
                >
                  Chạy
                </button>
              )}
              <button type="button" className="ghost" onClick={() => deleteMacro(m.id)}>
                Xoá
              </button>
            </div>
          ))
        )}
      </div>
    </>
  );
}

function GpsTool({ targets, scopeLabel }: { targets: string[]; scopeLabel: string }) {
  const [coords, setCoords] = useState("");
  const [busy, setBusy] = useState(false);

  const parse = (): { lat: number; lng: number } | null => {
    const nums = coords
      .split(/[,\s]+/)
      .map(Number)
      .filter((n) => Number.isFinite(n));
    if (nums.length < 2) return null;
    const [lat, lng] = nums;
    if (Math.abs(lat) > 90 || Math.abs(lng) > 180) return null;
    return { lat, lng };
  };

  const apply = async () => {
    const c = parse();
    if (!c) {
      pushToast("warn", "Toạ độ không hợp lệ", "Nhập dạng: 21.028511, 105.804817");
      return;
    }
    if (!targets.length) {
      pushToast("warn", "Chưa có máy", "Chọn máy rồi đặt.");
      return;
    }
    setBusy(true);
    const results = await Promise.allSettled(targets.map((u) => setMockLocation(u, c.lat, c.lng)));
    setBusy(false);
    const ok = results.filter((r) => r.status === "fulfilled").length;
    if (ok === targets.length) pushToast("ok", "Đã đặt vị trí", `${ok} máy`);
    else
      pushToast(
        "warn",
        `Đặt vị trí ${ok}/${targets.length} máy`,
        "Máy còn lại cần Riviu helper + quyền mock-location.",
      );
  };

  const stop = async () => {
    if (!targets.length) return;
    setBusy(true);
    await Promise.allSettled(targets.map((u) => stopMockLocation(u)));
    setBusy(false);
    pushToast("ok", "Đã tắt giả lập vị trí", `${targets.length} máy`);
  };

  return (
    <>
      <p className="hint">
        Giả lập vị trí GPS cho {scopeLabel} (Android, cần Riviu helper — cấp quyền
        mock-location tự động). Copy toạ độ từ Google Maps (chuột phải → bấm vào toạ độ để
        chép) rồi dán vào đây.
      </p>
      <div className="row">
        <label style={{ flex: 1 }}>
          Toạ độ (vĩ độ, kinh độ)
          <input
            type="text"
            placeholder="21.028511, 105.804817"
            value={coords}
            onChange={(e) => setCoords(e.target.value)}
          />
        </label>
      </div>
      <div className="nurture-float-actions">
        <button type="button" className="primary" disabled={busy} onClick={() => void apply()}>
          {busy ? "Đang đặt…" : "Đặt vị trí"}
        </button>
        <button type="button" className="ghost" disabled={busy} onClick={() => void stop()}>
          Tắt giả lập
        </button>
      </div>
    </>
  );
}

/** Random bytes, from the CSPRNG where present, else `Math.random` on old webviews. */
function randomBytes(n: number): Uint8Array {
  const bytes = new Uint8Array(n);
  try {
    crypto.getRandomValues(bytes);
  } catch {
    for (let i = 0; i < n; i += 1) bytes[i] = Math.floor(Math.random() * 256);
  }
  return bytes;
}

/** 16 hex chars — the shape of a `Settings.Secure` android_id. */
function randomAndroidId(): string {
  return Array.from(randomBytes(8), (b) => b.toString(16).padStart(2, "0")).join("");
}

/** A plausible device serial: 12 uppercase alphanumerics (no ambiguous I/O/0/1). */
function randomSerial(): string {
  const alphabet = "ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
  return Array.from(randomBytes(12), (b) => alphabet[b % alphabet.length]).join("");
}

/** A locally-administered unicast MAC (first octet: bit 1 set, bit 0 clear). */
function randomMac(): string {
  const bytes = randomBytes(6);
  bytes[0] = (bytes[0] & 0xfe) | 0x02;
  return Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join(":");
}

/**
 * Root tier (C, xiaowei "ROOT 模式"): batch identity change ("一键新机"), factory reset and a
 * root shell, all scoped to the current selection. Each phone that lacks Magisk `su` reports
 * that per-field rather than half-applying — only Android ID changes without root (adb carries
 * WRITE_SECURE_SETTINGS). Every phone gets its *own* random identity: a farm of clones defeats
 * the point.
 */
function RootTool({ targets, scopeLabel }: { targets: string[]; scopeLabel: string }) {
  const [rootStatus, setRootStatus] = useState<"idle" | "checking" | "done">("idle");
  const [rootedCount, setRootedCount] = useState(0);
  const [changeAndroidId, setChangeAndroidId] = useState(true);
  const [changeSerial, setChangeSerial] = useState(true);
  const [changeMac, setChangeMac] = useState(true);
  const [shellCmd, setShellCmd] = useState("");
  const [busy, setBusy] = useState<null | "probe" | "identity" | "reset" | "shell">(null);
  const [logText, setLogText] = useState("");

  const noTargets = () => {
    pushToast("warn", "Chưa có máy", "Chọn máy rồi thử lại.");
    return true;
  };

  const probe = async () => {
    if (!targets.length) return void noTargets();
    setBusy("probe");
    setRootStatus("checking");
    const results = await Promise.allSettled(targets.map((u) => isRooted(u)));
    setBusy(null);
    setRootedCount(results.filter((r) => r.status === "fulfilled" && r.value).length);
    setRootStatus("done");
  };

  const applyIdentity = async () => {
    if (!targets.length) return void noTargets();
    if (!changeAndroidId && !changeSerial && !changeMac) {
      pushToast("warn", "Chưa chọn trường", "Tích ít nhất một mục để đổi.");
      return;
    }
    setBusy("identity");
    const results = await Promise.allSettled(
      targets.map((u) => {
        const identity: { androidId?: string; serialno?: string; mac?: string } = {};
        if (changeAndroidId) identity.androidId = randomAndroidId();
        if (changeSerial) identity.serialno = randomSerial();
        if (changeMac) identity.mac = randomMac();
        return setDeviceIdentity(u, identity);
      }),
    );
    setBusy(null);
    setLogText(
      results
        .map((r, i) =>
          r.status === "fulfilled" ? `${targets[i]}: ${r.value}` : `${targets[i]}: ✗ ${String(r.reason)}`,
        )
        .join("\n"),
    );
    const ok = results.filter((r) => r.status === "fulfilled").length;
    pushToast(ok === targets.length ? "ok" : "warn", "Đổi định danh", `${ok}/${targets.length} máy`);
  };

  const runReset = async () => {
    if (!targets.length) return void noTargets();
    const sure = await requestConfirm({
      title: `Khôi phục gốc ${targets.length} máy?`,
      message:
        "Toàn bộ dữ liệu trên các máy đã chọn sẽ bị xoá sạch và KHÔNG THỂ hoàn tác. Chỉ máy đã root mới thực thi được.",
      confirmLabel: "Khôi phục gốc",
      danger: true,
    });
    if (!sure) return;
    setBusy("reset");
    const results = await Promise.allSettled(targets.map((u) => factoryReset(u)));
    setBusy(null);
    setLogText(
      results
        .map((r, i) =>
          r.status === "fulfilled"
            ? `${targets[i]}: đã gửi lệnh khôi phục`
            : `${targets[i]}: ✗ ${String(r.reason)}`,
        )
        .join("\n"),
    );
    const ok = results.filter((r) => r.status === "fulfilled").length;
    if (ok === targets.length) pushToast("ok", "Đã gửi lệnh khôi phục gốc", `${ok} máy`);
    else pushToast("warn", `Khôi phục ${ok}/${targets.length} máy`, "Máy còn lại chưa root.");
  };

  const runShell = async () => {
    const cmd = shellCmd.trim();
    if (!cmd) {
      pushToast("warn", "Chưa có lệnh", "Nhập lệnh shell rồi chạy.");
      return;
    }
    if (!targets.length) return void noTargets();
    setBusy("shell");
    const results = await Promise.allSettled(targets.map((u) => rootShell(u, cmd)));
    setBusy(null);
    setLogText(
      results
        .map((r, i) =>
          r.status === "fulfilled"
            ? `${targets[i]}:\n${r.value.trim() || "(trống)"}`
            : `${targets[i]}: ✗ ${String(r.reason)}`,
        )
        .join("\n\n"),
    );
  };

  return (
    <>
      <p className="hint">
        Tầng ROOT cho {scopeLabel} (Android, cần Magisk <code>su</code>). Rủi ro cao, chỉ dùng
        trên thiết bị hợp pháp của bạn: đổi định danh chống trùng, khôi phục gốc, lệnh root.
      </p>

      <div className="row">
        <button type="button" className="ghost" disabled={busy !== null} onClick={() => void probe()}>
          {busy === "probe" ? "Đang kiểm tra…" : "Kiểm tra trạng thái root"}
        </button>
        {rootStatus === "done" && (
          <span className="hint">
            {rootedCount}/{targets.length} máy đã root
          </span>
        )}
      </div>

      <fieldset className="group-tools-fieldset">
        <legend>Máy mới — đổi định danh (mỗi máy một giá trị ngẫu nhiên)</legend>
        <p className="hint">
          Đổi định danh mà ứng dụng đọc được (Android ID / serial / MAC Wi-Fi), không đổi IMEI
          baseband. Android ID không cần root; serial &amp; MAC cần root.
        </p>
        <label className="check">
          <input
            type="checkbox"
            checked={changeAndroidId}
            onChange={(e) => setChangeAndroidId(e.target.checked)}
          />
          Android ID
        </label>
        <label className="check">
          <input type="checkbox" checked={changeSerial} onChange={(e) => setChangeSerial(e.target.checked)} />
          Serial (cần root)
        </label>
        <label className="check">
          <input type="checkbox" checked={changeMac} onChange={(e) => setChangeMac(e.target.checked)} />
          MAC Wi-Fi (cần root)
        </label>
        <div className="nurture-float-actions">
          <button type="button" className="primary" disabled={busy !== null} onClick={() => void applyIdentity()}>
            {busy === "identity" ? "Đang đổi…" : "Tạo ngẫu nhiên & áp mỗi máy"}
          </button>
        </div>
      </fieldset>

      <fieldset className="group-tools-fieldset">
        <legend>Lệnh root (nâng cao)</legend>
        <div className="row">
          <input
            type="text"
            style={{ flex: 1 }}
            placeholder="vd: getprop ro.serialno"
            value={shellCmd}
            onChange={(e) => setShellCmd(e.target.value)}
          />
          <button type="button" className="ghost" disabled={busy !== null} onClick={() => void runShell()}>
            {busy === "shell" ? "Đang chạy…" : "Chạy (su)"}
          </button>
        </div>
      </fieldset>

      {logText && <pre className="group-tools-log">{logText}</pre>}

      <fieldset className="group-tools-fieldset danger-zone">
        <legend>Vùng nguy hiểm</legend>
        <div className="nurture-float-actions">
          <button type="button" className="danger" disabled={busy !== null} onClick={() => void runReset()}>
            {busy === "reset" ? "Đang gửi…" : `Khôi phục gốc ${scopeLabel}`}
          </button>
        </div>
      </fieldset>
    </>
  );
}

/** Human label for a peripheral action, shown as "vừa gửi …" feedback. */
function describeAction(action: PeripheralAction): string {
  switch (action.kind) {
    case "key":
      return `phím ${action.key}`;
    case "tap":
      return "chạm";
    case "swipe":
      return "vuốt";
    case "macro":
      return `macro ${action.name}`;
  }
}

/**
 * Physical peripherals (D, xiaowei "外设"): USB relay power control + a gamepad→fleet bridge.
 *
 * The relay talks to the host serial port through the backend; the gamepad is read here with
 * the browser's Web Gamepad API (no host driver) and mapped to fleet gestures by
 * `peripheralMap`. Both act on the current selection.
 */
function PeripheralsTool({ targets, scopeLabel }: { targets: string[]; scopeLabel: string }) {
  const [ports, setPorts] = useState<SerialPortInfo[]>([]);
  const [port, setPort] = useState("");
  const [channel, setChannel] = useState(1);
  const [holdMs, setHoldMs] = useState(800);
  const [energize, setEnergize] = useState(true);
  const [busy, setBusy] = useState<string | null>(null);
  const [padOn, setPadOn] = useState(false);
  const [padId, setPadId] = useState<string | null>(null);
  const [lastFired, setLastFired] = useState<string | null>(null);

  // The poll loop reads the selection through a ref so it need not restart on every
  // selection change (which would reset edge-detection mid-press).
  const targetsRef = useRef(targets);
  targetsRef.current = targets;

  const refreshPorts = async () => {
    setBusy("ports");
    try {
      const found = await listSerialPorts();
      setPorts(found);
      if (!port && found.length) setPort(found[0].name);
    } catch (e) {
      toastError("Không liệt kê được cổng", e);
    } finally {
      setBusy(null);
    }
  };

  const relaySet = async (on: boolean) => {
    if (!port) {
      pushToast("warn", "Chưa chọn cổng", "Chọn cổng COM của bo relay.");
      return;
    }
    setBusy(on ? "on" : "off");
    try {
      await relaySetChannel(port, channel, on);
      pushToast("ok", on ? "Đã bật kênh relay" : "Đã tắt kênh relay", `${port} · kênh ${channel}`);
    } catch (e) {
      toastError("Lệnh relay thất bại", e);
    } finally {
      setBusy(null);
    }
  };

  const relayPulse = async () => {
    if (!port) {
      pushToast("warn", "Chưa chọn cổng", "Chọn cổng COM của bo relay.");
      return;
    }
    setBusy("pulse");
    try {
      await relayPulseChannel(port, channel, holdMs, energize);
      pushToast("ok", "Đã xung relay", `${port} · kênh ${channel} · ${holdMs}ms`);
    } catch (e) {
      toastError("Xung relay thất bại", e);
    } finally {
      setBusy(null);
    }
  };

  // Gamepad → fleet bridge. Polls on animation frames while enabled; fires each bound button
  // once on its rising edge (see `risingEdges`) to the current selection.
  useEffect(() => {
    if (!padOn) return;
    const bindings = defaultGamepadBindings();
    let previous: boolean[] = [];
    let frame = 0;

    const fire = (action: PeripheralAction) => {
      const udids = targetsRef.current;
      if (!udids.length) return;
      const sync = getGroupSync();
      if (action.kind === "key") {
        void groupInput({ udids, kind: "key", key: action.key, sync });
      } else if (action.kind === "tap") {
        void groupInput({
          udids,
          kind: "tap",
          x: toReference(action.fx),
          y: toReference(action.fy),
          imageW: REFERENCE,
          imageH: REFERENCE,
          sync,
        });
      } else if (action.kind === "swipe") {
        void groupInput({
          udids,
          kind: "swipe",
          x: toReference(action.fx1),
          y: toReference(action.fy1),
          toX: toReference(action.fx2),
          toY: toReference(action.fy2),
          imageW: REFERENCE,
          imageH: REFERENCE,
          sync,
        });
      }
      // macro bindings are not in the default set; nothing to fire here.
      setLastFired(describeAction(action));
    };

    const poll = () => {
      const pads = navigator.getGamepads ? navigator.getGamepads() : [];
      const pad = Array.from(pads).find((p): p is Gamepad => p !== null);
      if (pad) {
        setPadId(pad.id);
        const current = pad.buttons.map((b) => b.pressed);
        for (const index of risingEdges(previous, current)) {
          const action = resolveButtonAction(bindings, index);
          if (action) fire(action);
        }
        previous = current;
      } else {
        setPadId(null);
      }
      frame = requestAnimationFrame(poll);
    };
    frame = requestAnimationFrame(poll);
    return () => cancelAnimationFrame(frame);
  }, [padOn]);

  return (
    <>
      <p className="hint">
        Ngoại vi vật lý cho {scopeLabel} (xiaowei "外设"). Relay USB để bật/tắt nguồn hoặc khởi
        động cứng máy kẹt; tay cầm (Gamepad) điều khiển cả nhóm.
      </p>

      <fieldset className="group-tools-fieldset">
        <legend>USB Relay (nguồn / reboot cứng)</legend>
        <div className="row">
          <label style={{ flex: 1 }}>
            Cổng COM
            <select value={port} onChange={(e) => setPort(e.target.value)}>
              <option value="">— chọn cổng —</option>
              {ports.map((p) => (
                <option key={p.name} value={p.name}>
                  {p.name} ({p.kind})
                </option>
              ))}
            </select>
          </label>
          <label>
            Kênh
            <input
              type="number"
              min={1}
              max={16}
              value={channel}
              onChange={(e) => setChannel(Math.max(1, Number(e.target.value) || 1))}
              style={{ width: "5rem" }}
            />
          </label>
          <button type="button" className="ghost" disabled={busy !== null} onClick={() => void refreshPorts()}>
            {busy === "ports" ? "…" : "Quét cổng"}
          </button>
        </div>
        <div className="nurture-float-actions">
          <button type="button" className="ghost" disabled={busy !== null} onClick={() => void relaySet(true)}>
            {busy === "on" ? "…" : "Bật kênh"}
          </button>
          <button type="button" className="ghost" disabled={busy !== null} onClick={() => void relaySet(false)}>
            {busy === "off" ? "…" : "Tắt kênh"}
          </button>
        </div>
        <div className="row" style={{ marginTop: "0.4rem" }}>
          <label>
            Giữ (ms)
            <input
              type="number"
              min={50}
              max={10000}
              value={holdMs}
              onChange={(e) => setHoldMs(Math.max(50, Number(e.target.value) || 50))}
              style={{ width: "6rem" }}
            />
          </label>
          <label className="check">
            <input type="checkbox" checked={energize} onChange={(e) => setEnergize(e.target.checked)} />
            Nhấn (bật→tắt); bỏ tích = ngắt nguồn (tắt→bật)
          </label>
          <button type="button" className="primary" disabled={busy !== null} onClick={() => void relayPulse()}>
            {busy === "pulse" ? "…" : "Xung (reboot)"}
          </button>
        </div>
      </fieldset>

      <fieldset className="group-tools-fieldset">
        <legend>Tay cầm (Gamepad) → nhóm</legend>
        <p className="hint">
          Cắm tay cầm USB/Bluetooth vào PC. A→Home, B→Back, X→Đa nhiệm, D-pad→vuốt. Mỗi lần bấm
          gửi cho {scopeLabel}.
        </p>
        <label className="check">
          <input type="checkbox" checked={padOn} onChange={(e) => setPadOn(e.target.checked)} />
          Bật điều khiển bằng tay cầm
        </label>
        {padOn && (
          <p className="hint">
            {padId ? `Đã nhận: ${padId}` : "Chưa thấy tay cầm — bấm một nút để trình duyệt nhận."}
            {lastFired ? ` · vừa gửi ${lastFired}` : ""}
          </p>
        )}
      </fieldset>
    </>
  );
}

/** Render a tall PNG with a big number centred, for "set number as wallpaper" (A3). */
async function numberWallpaperPng(label: string): Promise<Uint8Array> {
  const canvas = document.createElement("canvas");
  canvas.width = 1080;
  canvas.height = 1920;
  const ctx = canvas.getContext("2d");
  if (!ctx) throw new Error("no 2d context");
  ctx.fillStyle = "#0b0b0f";
  ctx.fillRect(0, 0, canvas.width, canvas.height);
  ctx.fillStyle = "#ff6a00";
  ctx.textAlign = "center";
  ctx.textBaseline = "middle";
  ctx.font = "bold 620px system-ui, sans-serif";
  ctx.fillText(label, canvas.width / 2, canvas.height / 2);
  const blob: Blob = await new Promise((resolve, reject) =>
    canvas.toBlob((b) => (b ? resolve(b) : reject(new Error("toBlob failed"))), "image/png"),
  );
  return new Uint8Array(await blob.arrayBuffer());
}
