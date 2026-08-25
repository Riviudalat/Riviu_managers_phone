import { useMemo, useState } from "react";
import { groupInput } from "../../api";
import { groupInputOutcome } from "../../groupInput";
import { getGroupSync } from "../../groupSync";
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
} from "../../quickPhrases";
import { pushToast, toastError } from "../../toastStore";
import { newId } from "./toolHelpers";

export function QuickReplyTool({ targets, scopeLabel }: { targets: string[]; scopeLabel: string }) {
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
