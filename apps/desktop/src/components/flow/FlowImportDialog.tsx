import { Upload, X } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { flowImportLegacy } from "../../api";
import { describeError } from "../../describeError";
import { GENFARMER_ACTION_INVENTORY, importGenFarmerWorkflow, importMacroWorkflow, MAX_FLOW_IMPORT_BYTES, parseWorkflowImport, type MacroFlowCalibration, type WorkflowImportResult } from "../../flowImport";
import type { Macro } from "../../macro";
import { useSavedMacros } from "../../macroStore";
import type { FlowDocumentV2, LegacyImportResult } from "../../types";
import { useModalFocus } from "../useModalFocus";

// The same ceiling `FlowJsonDialog.tsx` enforces for V2 documents, refused before the string
// crosses IPC: the backend re-checks, but by then React has already held the whole paste.
const MAX_LEGACY_JSON_BYTES = MAX_FLOW_IMPORT_BYTES;

export interface FlowImportDialogProps {
  onImport: (document: FlowDocumentV2) => void;
  onClose: () => void;
  importLegacy?: (scriptJson: string) => Promise<LegacyImportResult>;
}

export function FlowImportDialog({
  onImport,
  onClose,
  importLegacy = flowImportLegacy,
}: FlowImportDialogProps) {
  const [raw, setRaw] = useState("");
  const [result, setResult] = useState<LegacyImportResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [format, setFormat] = useState<"legacy" | "genfarmer" | "macro">("legacy");
  const [preview, setPreview] = useState<WorkflowImportResult | null>(null);
  const [filePathsRaw,setFilePathsRaw]=useState("{}");
  const [windowsTextNewlines,setWindowsTextNewlines]=useState(false);
  const savedMacros = useSavedMacros();
  const generation = useRef(0);
  // Both close controls stay enabled while the conversion is running, and the textarea stays
  // editable. Without a guard, clicking Import and then Hủy still replaced the open document when
  // the backend answered -- an explicitly cancelled import applying itself.
  const live = useRef(true);
  // Set on mount as well as cleared on unmount. Only clearing it is wrong under StrictMode, which
  // mounts, unmounts and remounts every effect: the cleanup ran, nothing set the flag back, and the
  // guard then rejected every result for the rest of the component's life. The e2e import stopped
  // applying entirely; jsdom tests do not wrap in StrictMode, so they never saw it.
  useEffect(() => {
    live.current = true;
    return () => {
      live.current = false;
    };
  }, []);

  const close = () => {
    live.current = false;
    generation.current += 1;
    onClose();
  };
  const dialogRef = useModalFocus<HTMLElement>(close);

  const submit = async () => {
    const request = ++generation.current;
    setBusy(true);
    setError(null);
    setResult(null);
    setPreview(null);
    try {
      if (new TextEncoder().encode(raw).byteLength > MAX_LEGACY_JSON_BYTES) {
        throw new Error("FlowImportTooLarge");
      }
      if (format === "genfarmer") {
        const filePaths=parseWorkflowImport(filePathsRaw);
        if(!filePaths||typeof filePaths!=="object"||Array.isArray(filePaths)||Object.values(filePaths).some(value=>typeof value!=="string"))throw new Error("Ánh xạ đường dẫn phải là JSON {ID bước: đường dẫn tương đối}.");
        setPreview(importGenFarmerWorkflow(raw,{filePaths:filePaths as Record<string,string>,windowsTextNewlines}));
        return;
      }
      if (format === "macro") {
        const parsed = parseWorkflowImport(raw) as Macro | { macro: Macro; calibration?: Record<string, MacroFlowCalibration> };
        const wrapped = parsed !== null && typeof parsed === "object" && "macro" in parsed;
        setPreview(wrapped ? importMacroWorkflow(parsed.macro, parsed.calibration) : importMacroWorkflow(parsed as Macro));
        return;
      }
      const imported = await importLegacy(raw);
      if (!live.current || request !== generation.current) return;
      setResult(imported);
      if (imported.document !== null && imported.diagnostics.length === 0) {
        onImport(imported.document);
      }
    } catch (reason) {
      // `flow_import_legacy` rejects with a `CommandError` object (`flow_commands.rs:111`), so
      // `String(reason)` printed `[object Object]` over the reason the JSON was refused.
      if (live.current && request === generation.current) setError(describeError(reason));
    } finally {
      if (live.current && request === generation.current) setBusy(false);
    }
  };

  const replaceRaw = (value: string) => {
    generation.current += 1;
    setRaw(value);
    setResult(null);
    setPreview(null);
    setError(null);
    setBusy(false);
  };

  const loadFile = async (file: File | undefined) => {
    if (!file) return;
    const request = ++generation.current;
    setPreview(null);
    setResult(null);
    setBusy(true);
    setError(null);
    try {
      if (file.size > MAX_FLOW_IMPORT_BYTES) throw new Error("FlowImportTooLarge");
      const text = await file.text();
      if (live.current && request === generation.current) replaceRaw(text);
    } catch (reason) {
      if (live.current && request === generation.current) {
        setError(describeError(reason));
        setBusy(false);
      }
    }
  };

  return (
    <section ref={dialogRef} tabIndex={-1} role="dialog" aria-modal="true" aria-label="Nhập Flow" className="flow-dialog">
      <header>
        <strong>Nhập Flow</strong>
        <button type="button" aria-label="Đóng hộp thoại nhập" title="Đóng" onClick={close}>
          <X aria-hidden="true" size={16} />
        </button>
      </header>
      <label className="flow-field">
        <span>Định dạng nguồn</span>
        <select value={format} onChange={(event) => {
          setFormat(event.currentTarget.value as typeof format);
          replaceRaw("");
        }}>
          <option value="legacy">Riviu script cũ</option>
          <option value="genfarmer">GenFarmer JSON export</option>
          <option value="macro">Macro Riviu</option>
        </select>
      </label>
      {format === "macro" && savedMacros.length > 0 && (
        <label className="flow-field">
          <span>Macro đã lưu</span>
          <select defaultValue="" onChange={(event) => {
            const macro = savedMacros.find((item) => item.id === event.currentTarget.value);
            if (macro) replaceRaw(JSON.stringify(macro, null, 2));
          }}>
            <option value="">Chọn bản ghi</option>
            {savedMacros.map((macro) => <option key={macro.id} value={macro.id}>{macro.name} · {macro.steps.length} bước</option>)}
          </select>
        </label>
      )}
      <label className="flow-field">
        <span>Tệp JSON (tối đa 1 MiB)</span>
        <input type="file" accept=".json,application/json" onChange={(event) => {
          void loadFile(event.currentTarget.files?.[0]);
          event.currentTarget.value = "";
        }} />
      </label>
      <label className="flow-field">
        <span>{format === "legacy" ? "JSON script cũ" : "JSON nguồn"}</span>
        <textarea
          value={raw}
          rows={14}
          spellCheck={false}
          onChange={(event) => replaceRaw(event.currentTarget.value)}
        />
      </label>
      {format !== "legacy" && <p>Chuyển thành bản nháp để kiểm tra trước khi lưu. Các bước chưa tương thích sẽ được liệt kê theo ID nguồn.</p>}
      {format === "genfarmer" && <details><summary>Khả năng chuyển đổi theo hành động nguồn</summary><p>Start được chuyển thành Bắt đầu. {GENFARMER_ACTION_INVENTORY.length} hành động trong registry nguồn được phân loại bên dưới.</p><ul>{GENFARMER_ACTION_INVENTORY.map((item) => <li key={item.action}><strong>{item.action}: {item.status === "supportedSubset" ? "có chuyển đổi giới hạn" : "cần adapter"}.</strong> {item.reason}</li>)}</ul></details>}
      {format === "genfarmer" && <details><summary>Chuyển dữ liệu tệp</summary><label className="flow-field"><span>Đường dẫn tệp mới (ID bước: đường dẫn trong flow-data)</span><textarea rows={3} value={filePathsRaw} onChange={event=>{generation.current++;setBusy(false);setPreview(null);setFilePathsRaw(event.currentTarget.value);}} /></label><label><input type="checkbox" checked={windowsTextNewlines} onChange={event=>{generation.current++;setBusy(false);setPreview(null);setWindowsTextNewlines(event.currentTarget.checked);}} />Kịch bản TXT nguồn dùng xuống dòng Windows (CRLF)</label></details>}
      {error && (
        <div role="alert">
          <details>
            <summary>Không thể nhập Flow.</summary>
            <code>{error}</code>
          </details>
        </div>
      )}
      {result && result.diagnostics.length > 0 && (
        <section aria-label="Chẩn đoán nhập">
          <strong>{result.diagnostics.length} lỗi nhập</strong>
          <ul>
            {result.diagnostics.map((diagnostic, index) => (
              <li key={`${diagnostic.stepIndex}-${diagnostic.code}-${index}`}>
                <details>
                  {/* `step_index` comes from `enumerate()`, so it is zero-based. Printing it raw
                      sent the operator to the step before the broken one. */}
                  <summary>Bước {diagnostic.stepIndex + 1}: Không thể nhập hành động này.</summary>
                  <code>
                    {diagnostic.code}: {diagnostic.message}
                    {diagnostic.field ? ` (${diagnostic.field})` : ""}
                  </code>
                </details>
              </li>
            ))}
          </ul>
        </section>
      )}
      {preview && (
        <section aria-label="Xem trước chuyển đổi">
          <strong>{preview.metadata.sourceName} · {preview.metadata.sourceNodeCount} bước nguồn</strong>
          <p>{preview.document ? `${preview.document.nodes.length} node trong bản nháp; cần kiểm tra và lưu trước khi chạy.` : "Cần xử lý các mục bên dưới để chuyển đầy đủ kịch bản."}</p>
          {preview.diagnostics.length > 0 && <ul>
            {preview.diagnostics.map((diagnostic, index) => <li key={`${diagnostic.code}-${index}`}>
              <strong>{diagnostic.stepIndex === null ? "Script" : `Bước ${diagnostic.stepIndex + 1}`}{diagnostic.sourceNodeId ? ` [${diagnostic.sourceNodeId}]` : ""}: </strong>
              {diagnostic.message}
              <details><summary>Chi tiết</summary><code>{diagnostic.code}{diagnostic.field ? ` · ${diagnostic.field}` : ""}</code></details>
            </li>)}
          </ul>}
          {preview.document && <details>
            <summary>Ánh xạ node nguồn → bản nháp</summary>
            <ul>{preview.metadata.nodeMap.map((mapping) => <li key={mapping.sourceNodeId}><code>{mapping.sourceNodeId} → {mapping.targetNodeIds.join(", ")}</code></li>)}</ul>
          </details>}
        </section>
      )}
      <footer>
        <button type="button" onClick={close}>Hủy</button>
        <button type="button" disabled={busy || raw.trim() === ""} onClick={() => void submit()}>
          <Upload aria-hidden="true" size={15} />
          {busy ? "Đang nhập…" : format === "legacy" ? "Nhập" : "Xem trước"}
        </button>
        {preview?.document && <button type="button" disabled={busy} onClick={() => {
          // An import is a new flow even when the same source was imported previously.
          // Keep deterministic node mappings, but never reuse an existing flow/revision ID.
          if (preview.document) onImport({ ...preview.document, id: crypto.randomUUID() });
        }}>Mở bản nháp</button>}
      </footer>
    </section>
  );
}
