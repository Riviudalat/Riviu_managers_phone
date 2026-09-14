import { useEffect, useState } from "react";
import { X } from "lucide-react";
import type { AppWorkflowV1 } from "../appWorkflow";
import {
  automationGet,
  automationList,
  interactionResolveLinks,
  publishScanFolder,
} from "../api";
import type { AutomationDefinition, JsonObject, JsonValue } from "../types";
import { describeError } from "../describeError";
import { pickDirectory } from "../pickFile";

export function AppWorkflowConfig({
  document,
  onChange,
  onClose,
}: {
  document: AppWorkflowV1;
  onChange: (value: JsonValue) => void;
  onClose: () => void;
}) {
  const [profiles, setProfiles] = useState<AutomationDefinition[]>([]),
    [error, setError] = useState<string | null>(null),
    [text, setText] = useState(() =>
      JSON.stringify(document.profileConfig, null, 2),
    );
  const [form, setForm] = useState(
    () => structuredClone(document.profileConfig) as JsonObject,
  );
  const [links, setLinks] = useState(""),
    [busy, setBusy] = useState(false);
  const update = (field: string, value: JsonValue, nested?: string) => {
    const next = nested
      ? {
          ...form,
          [nested]: { ...(form[nested] as JsonObject), [field]: value },
        }
      : { ...form, [field]: value };
    setForm(next);
    setText(JSON.stringify(next, null, 2));
  };
  useEffect(() => {
    let alive = true;
    void automationList()
      .then((rows) => {
        if (alive) setProfiles(rows.filter((p) => p.kind === document.kind));
      })
      .catch((cause) => {
        if (alive) setError(describeError(cause));
      });
    return () => {
      alive = false;
    };
  }, [document.kind]);
  return (
    <section className="app-config-panel" aria-label="Cấu hình ứng dụng">
      <header>
        <strong>Đầu vào ứng dụng</strong>
        <button
          type="button"
          aria-label="Đóng cấu hình ứng dụng"
          onClick={onClose}
        >
          <X size={16} />
        </button>
      </header>
      <label>
        Nạp cấu hình đã lưu
        <select
          defaultValue=""
          onChange={async (e) => {
            const profile = profiles.find((p) => p.id === e.target.value);
            if (!profile) return;
            try {
              const record = await automationGet(
                profile.id,
                profile.latestRevision,
              );
              if (!record) throw new Error("Cấu hình đã bị xóa");
              setForm(structuredClone(record.revision.config) as JsonObject);
              setText(JSON.stringify(record.revision.config, null, 2));
              setError(null);
            } catch (cause) {
              setError(describeError(cause));
            }
          }}
        >
          <option value="">Chọn cấu hình…</option>
          {profiles.map((p) => (
            <option key={p.id} value={p.id}>
              {p.name} · bản {p.latestRevision}
            </option>
          ))}
        </select>
      </label>
      {document.kind === "nurture" && (
        <>
          <label>
            Phong cách nội dung
            <input
              value={String((form.settings as JsonObject)?.persona ?? "")}
              onChange={(e) => update("persona", e.target.value, "settings")}
            />
          </label>
          <label>
            Định hướng bình luận
            <textarea
              value={String((form.settings as JsonObject)?.aiDirections ?? "")}
              onChange={(e) =>
                update("aiDirections", e.target.value, "settings")
              }
            />
          </label>
          <p>
            Thời gian xem, số bài và hành động được chỉnh trực tiếp tại các bước
            trong canvas.
          </p>
        </>
      )}
      {document.kind === "interaction" && (
        <>
          <label>
            Link bài viết
            <textarea
              aria-label="Link bài viết của ứng dụng"
              placeholder="Mỗi dòng một link"
              value={links}
              onChange={(e) => setLinks(e.target.value)}
            />
          </label>
          <button
            type="button"
            disabled={busy || !links.trim()}
            onClick={async () => {
              setBusy(true);
              try {
                const rows = await interactionResolveLinks(links);
                if (rows.some((row) => !row.target))
                  throw new Error(
                    rows
                      .filter((row) => !row.target)
                      .map((row) => `Dòng ${row.lineNo}: ${row.error}`)
                      .join("; "),
                  );
                update(
                  "targets",
                  rows.map((row) => row.target) as unknown as JsonValue,
                  "request",
                );
                setError(null);
              } catch (cause) {
                setError(describeError(cause));
              } finally {
                setBusy(false);
              }
            }}
          >
            Kiểm tra và dùng link
          </button>
          <p>
            {Array.isArray((form.request as JsonObject)?.targets)
              ? ((form.request as JsonObject).targets as JsonValue[]).length
              : 0}{" "}
            bài đã chọn
          </p>
          <label>
            Nội dung bình luận
            <textarea
              value={
                Array.isArray((form.request as JsonObject)?.manualComments)
                  ? (
                      (form.request as JsonObject).manualComments as string[]
                    ).join("\n")
                  : ""
              }
              onChange={(e) =>
                update(
                  "manualComments",
                  e.target.value.split("\n").filter(Boolean),
                  "request",
                )
              }
            />
          </label>
          <label>
            Hướng dẫn nội dung
            <textarea
              value={String((form.request as JsonObject)?.instruction ?? "")}
              onChange={(e) => update("instruction", e.target.value, "request")}
            />
          </label>
        </>
      )}
      {document.kind === "publish" && (
        <>
          <label>
            Thư mục bài đăng
            <input
              value={String(form.sourceRoot ?? "")}
              onChange={(e) => update("sourceRoot", e.target.value)}
            />
          </label>
          <button
            type="button"
            disabled={busy}
            onClick={async () => {
              const folder = await pickDirectory("Chọn thư mục bài đăng");
              if (!folder) return;
              setBusy(true);
              try {
                const manifest = await publishScanFolder(folder);
                const next = {
                  ...form,
                  sourceRoot: folder,
                  bundleIds: manifest.bundles.map((bundle) => bundle.id),
                executionConfirmed: false,
                };
                setForm(next);
                setText(JSON.stringify(next, null, 2));
                setError(null);
              } catch (cause) {
                setError(describeError(cause));
              } finally {
                setBusy(false);
              }
            }}
          >
            Chọn thư mục và quét bài
          </button>
          <p>
            {Array.isArray(form.bundleIds) ? form.bundleIds.length : 0} bài
            trong nguồn
          </p>
        </>
      )}
      <details>
        <summary>JSON nâng cao</summary>
        <label>
          Thông số đầu vào
          <textarea
            aria-label="JSON cấu hình ứng dụng"
            spellCheck={false}
            rows={14}
            value={text}
            onChange={(e) => setText(e.target.value)}
          />
        </label>
      </details>
      {error && <p role="alert">{error}</p>}
      <button
        type="button"
        onClick={() => {
          try {
            const value = JSON.parse(text) as JsonValue;
            if (!value || typeof value !== "object" || Array.isArray(value))
              throw new Error("Cấu hình phải là một object JSON");
            onChange(value);
            setError(null);
            onClose();
          } catch (cause) {
            setError(describeError(cause));
          }
        }}
      >
        Áp dụng cấu hình
      </button>
    </section>
  );
}
