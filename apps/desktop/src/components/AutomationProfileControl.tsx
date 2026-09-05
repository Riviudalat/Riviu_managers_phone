import { useCallback, useEffect, useImperativeHandle, useMemo, useRef, useState, type Ref } from "react";
import { Archive, Plus, RefreshCw, Save } from "lucide-react";

import {
  automationArchive,
  automationCreate,
  automationGet,
  automationList,
  automationRevise,
} from "../api";
import { requestConfirm } from "../confirmStore";
import { requestWorkspaceLeave, useWorkspaceDraft } from "../workspaceDraft";
import { describeError } from "../describeError";
import type {
  AutomationDefinition,
  AutomationDefinitionRecord,
  AutomationKind,
  JsonValue,
  TargetRef,
} from "../types";
import { LoadingState, StatusNotice } from "./States";
import { AutomationScheduleControl } from "./AutomationScheduleControl";

const KIND_LABEL: Record<AutomationKind, string> = {
  nurture: "Nuôi TikTok",
  interaction: "Tương tác",
  publish: "Đăng bài",
};

type Props = {
  kind: AutomationKind;
  target: TargetRef;
  config: JsonValue;
  defaultName: string;
  disabled?: boolean;
  disabledReason?: string;
  disabledReasonId?: string;
  confirmSave?: () => Promise<boolean>;
  ref?: Ref<AutomationProfileHandle>;
  onApply?: (record: AutomationDefinitionRecord) => void | Promise<void>;
  onSaved?: (record: AutomationDefinitionRecord) => void | Promise<void>;
  dirty?: boolean;
  draftId?: string;
};

export interface AutomationProfileHandle {
  save: () => Promise<boolean>;
}

/** Stores the current setup as an immutable, target-bound automation revision. */
export function AutomationProfileControl({
  kind,
  target,
  config,
  defaultName,
  disabled = false,
  disabledReason,
  disabledReasonId,
  confirmSave,
  ref,
  onApply,
  onSaved,
  dirty = false,
  draftId,
}: Props) {
  const label = KIND_LABEL[kind];
  const [profiles, setProfiles] = useState<AutomationDefinition[]>([]);
  const [selectedId, setSelectedId] = useState("");
  const [name, setName] = useState(defaultName);
  const [savedName, setSavedName] = useState(defaultName);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const requestEpoch = useRef(0);
  const busyRef = useRef(false);
  const editableKey = JSON.stringify([name, target, config]);
  const latestKey = useRef(editableKey);
  const latestName = useRef(name);
  const successfulSaveKeys = useRef(new Set<string>());
  latestKey.current = editableKey;
  latestName.current = name;
  const nameDirty = name !== savedName;
  const nameDraftId = `profile-name-${kind}`;

  const selectedProfile = useMemo(
    () => profiles.find((profile) => profile.id === selectedId) ?? null,
    [profiles, selectedId],
  );

  const load = useCallback(async () => {
    const epoch = ++requestEpoch.current;
    setLoading(true);
    setError(null);
    try {
      const next = (await automationList()).filter(
        (profile) => profile.kind === kind && !profile.archived,
      );
      if (requestEpoch.current !== epoch) return;
      setProfiles(next);
    } catch (cause) {
      if (requestEpoch.current === epoch) setError(describeError(cause));
    } finally {
      if (requestEpoch.current === epoch) setLoading(false);
    }
  }, [kind]);

  useEffect(() => {
    void load();
    return () => { requestEpoch.current += 1; };
  }, [load]);

  const save = useCallback(async () => {
    if (disabled || busyRef.current) return false;
    if (successfulSaveKeys.current.has(editableKey)) return true;
    const trimmedName = name.trim();
    if (!trimmedName) {
      setError("Tên hồ sơ không được để trống.");
      return false;
    }
    busyRef.current = true;
    const snapshotKey = editableKey;
    setBusy(true);
    setError(null);
    setNotice(null);
    try {
      if (selectedProfile) {
        const confirmed = await requestConfirm({
          title: "Lưu thiết lập hiện tại thành bản mới?",
          message: `Thiết lập và phạm vi máy đang hiển thị sẽ trở thành bản ${selectedProfile.latestRevision + 1} của ${selectedProfile.name}. Các bản đã ghim trước đó vẫn được giữ nguyên.`,
          confirmLabel: "Lưu bản mới",
          cancelLabel: "Hủy",
        });
        if (!confirmed) return false;
      }
      if (confirmSave && !(await confirmSave())) return false;
      const record = selectedProfile
        ? await automationRevise(
            selectedProfile.id,
            selectedProfile.latestRevision,
            target,
            config,
          )
        : await automationCreate(trimmedName, kind, target, config);
      setProfiles((current) => {
        const without = current.filter((profile) => profile.id !== record.definition.id);
        return [...without, record.definition].sort((left, right) =>
          left.name.localeCompare(right.name, "vi"),
        );
      });
      if (latestName.current === name) {
        setSelectedId(record.definition.id);
        setName(record.definition.name);
        setSavedName(record.definition.name);
      }
      setNotice(
        `${selectedProfile ? "Đã lưu" : "Đã tạo"} ${record.definition.name} · bản ${record.revision.revision}`,
      );
      if (latestKey.current !== snapshotKey) {
        setNotice("Đã lưu bản trước. Thiết lập vừa sửa vẫn chưa được lưu.");
        return false;
      }
      await onSaved?.(record);
      successfulSaveKeys.current = new Set([snapshotKey, JSON.stringify([record.definition.name, target, config])]);
      return true;
    } catch (cause) {
      setError(describeError(cause));
      return false;
    } finally {
      busyRef.current = false;
      setBusy(false);
    }
  }, [config, confirmSave, disabled, editableKey, kind, name, onSaved, selectedProfile, target]);

  useImperativeHandle(ref, () => ({ save }), [save]);

  useWorkspaceDraft({
    id: nameDraftId,
    label: `Tên hồ sơ ${label}`,
    dirty: nameDirty,
    snapshotKey: name,
    save,
    discard: () => setName(savedName),
  });

  const selectProfile = async (id: string) => {
    if (busyRef.current || id === selectedId) return;
    if ((dirty || nameDirty) && !(await requestWorkspaceLeave(draftId ? [draftId, nameDraftId] : [nameDraftId]))) return;
    const profile = profiles.find((entry) => entry.id === id);
    if (!profile) {
      successfulSaveKeys.current.clear();
      setSelectedId("");
      setName(defaultName);
      setSavedName(defaultName);
      setNotice(null);
      return;
    }
    busyRef.current = true;
    setBusy(true);
    setError(null);
    const epoch = ++requestEpoch.current;
    const snapshotKey = latestKey.current;
    try {
      const record = await automationGet(id, profile.latestRevision);
      if (epoch !== requestEpoch.current || latestKey.current !== snapshotKey) return;
      if (!record || record.definition.kind !== kind || record.revision.revision !== profile.latestRevision) {
        throw new Error("Hồ sơ đã thay đổi. Tải lại danh sách trước khi chọn.");
      }
      await onApply?.(record);
      successfulSaveKeys.current.clear();
      setSelectedId(id);
      setName(profile.name);
      setSavedName(profile.name);
      setNotice(`Đã nạp ${profile.name} · bản ${record.revision.revision}`);
    } catch (cause) {
      if (epoch === requestEpoch.current) setError(describeError(cause));
    } finally {
      busyRef.current = false;
      if (epoch === requestEpoch.current) setBusy(false);
    }
  };

  const archive = useCallback(async () => {
    if (!selectedProfile) return;
    if ((dirty || nameDirty) && !(await requestWorkspaceLeave(draftId ? [draftId, nameDraftId] : [nameDraftId]))) return;
    const confirmed = await requestConfirm({
      title: "Lưu trữ hồ sơ?",
      message: `${selectedProfile.name} sẽ không còn xuất hiện khi tạo điều phối mới. Các revision đã ghim vẫn được giữ.`,
      confirmLabel: "Lưu trữ",
      cancelLabel: "Hủy",
      danger: true,
    });
    if (!confirmed) return;
    setBusy(true);
    setError(null);
    try {
      await automationArchive(selectedProfile.id);
      successfulSaveKeys.current.clear();
      setNotice(`Đã lưu trữ ${selectedProfile.name}.`);
      setSelectedId("");
      setName(defaultName);
      setSavedName(defaultName);
      await load();
    } catch (cause) {
      setError(describeError(cause));
    } finally {
      setBusy(false);
    }
  }, [defaultName, dirty, draftId, load, nameDirty, nameDraftId, selectedProfile]);

  if (loading) return <LoadingState label={`Đang tải hồ sơ ${label}…`} />;

  return (
    <section className="automation-profile-control" aria-label={`Quản lý hồ sơ ${label}`}>
      <div className="automation-profile-fields">
        <label>
          <span>Hồ sơ</span>
          <select
            aria-label={`Hồ sơ ${label}`}
            value={selectedId}
            disabled={busy}
            onChange={(event) => void selectProfile(event.currentTarget.value)}
          >
            <option value="">Hồ sơ mới</option>
            {profiles.map((profile) => (
              <option key={profile.id} value={profile.id}>
                {profile.name} · bản {profile.latestRevision}
              </option>
            ))}
          </select>
        </label>
        <label className="automation-profile-name">
          <span>Tên</span>
          <input
            aria-label={`Tên hồ sơ ${label}`}
            value={name}
            disabled={Boolean(selectedProfile)}
            onChange={(event) => setName(event.currentTarget.value)}
          />
        </label>
        {selectedProfile && (
          <button
            type="button"
            className="icon-btn"
            aria-label="Tạo hồ sơ mới"
            title="Tạo hồ sơ mới"
            disabled={busy}
            onClick={() => void selectProfile("")}
          >
            <Plus size={16} />
          </button>
        )}
        <button
          type="button"
          className="ghost automation-profile-save"
          disabled={busy || disabled}
          title={disabled ? disabledReason : undefined}
          aria-describedby={disabled ? disabledReasonId : undefined}
          onClick={() => void save()}
        >
          {selectedProfile ? <Save size={16} /> : <Plus size={16} />}
          {selectedProfile ? "Lưu bản mới" : "Tạo hồ sơ"}
        </button>
        {selectedProfile && (
          <button
            type="button"
            className="icon-btn"
            aria-label="Lưu trữ hồ sơ"
            title="Lưu trữ hồ sơ"
            disabled={busy}
            onClick={() => void archive()}
          >
            <Archive size={16} />
          </button>
        )}
      </div>
      {disabled && disabledReason && !disabledReasonId && <p className="hint">{disabledReason}</p>}
      {error && (
        <StatusNotice
          tone="error"
          action={
            <button type="button" onClick={() => void load()}>
              <RefreshCw size={15} /> Thử lại hồ sơ
            </button>
          }
        >
          {error}
        </StatusNotice>
      )}
      {notice && <StatusNotice tone="success">{notice}</StatusNotice>}
      <AutomationScheduleControl profile={selectedProfile} />
    </section>
  );
}
