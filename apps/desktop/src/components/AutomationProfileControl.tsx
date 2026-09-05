import { useCallback, useEffect, useMemo, useState } from "react";
import { Archive, Plus, RefreshCw, Save } from "lucide-react";

import {
  automationArchive,
  automationCreate,
  automationList,
  automationRevise,
} from "../api";
import { requestConfirm } from "../confirmStore";
import { describeError } from "../describeError";
import type {
  AutomationDefinition,
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
};

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
}: Props) {
  const label = KIND_LABEL[kind];
  const [profiles, setProfiles] = useState<AutomationDefinition[]>([]);
  const [selectedId, setSelectedId] = useState("");
  const [name, setName] = useState(defaultName);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  const selectedProfile = useMemo(
    () => profiles.find((profile) => profile.id === selectedId) ?? null,
    [profiles, selectedId],
  );

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const next = (await automationList()).filter(
        (profile) => profile.kind === kind && !profile.archived,
      );
      setProfiles(next);
      setSelectedId("");
      setName(defaultName);
    } catch (cause) {
      setError(describeError(cause));
    } finally {
      setLoading(false);
    }
  }, [defaultName, kind]);

  useEffect(() => {
    void load();
  }, [load]);

  const save = useCallback(async () => {
    const trimmedName = name.trim();
    if (!trimmedName) {
      setError("Tên hồ sơ không được để trống.");
      return;
    }
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
        if (!confirmed) return;
      }
      if (confirmSave && !(await confirmSave())) return;
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
      setSelectedId(record.definition.id);
      setName(record.definition.name);
      setNotice(
        `${selectedProfile ? "Đã lưu" : "Đã tạo"} ${record.definition.name} · bản ${record.revision.revision}`,
      );
    } catch (cause) {
      setError(describeError(cause));
    } finally {
      setBusy(false);
    }
  }, [config, confirmSave, kind, name, selectedProfile, target]);

  const archive = useCallback(async () => {
    if (!selectedProfile) return;
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
      setNotice(`Đã lưu trữ ${selectedProfile.name}.`);
      setSelectedId("");
      setName(defaultName);
      await load();
    } catch (cause) {
      setError(describeError(cause));
    } finally {
      setBusy(false);
    }
  }, [defaultName, load, selectedProfile]);

  if (loading) return <LoadingState label={`Đang tải hồ sơ ${label}…`} />;

  return (
    <section className="automation-profile-control" aria-label={`Quản lý hồ sơ ${label}`}>
      <div className="automation-profile-fields">
        <label>
          <span>Hồ sơ</span>
          <select
            aria-label={`Hồ sơ ${label}`}
            value={selectedId}
            onChange={(event) => {
              const id = event.currentTarget.value;
              setSelectedId(id);
              const profile = profiles.find((candidate) => candidate.id === id);
              setName(profile?.name ?? defaultName);
              setNotice(null);
            }}
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
            onClick={() => {
              setSelectedId("");
              setName(defaultName);
              setNotice(null);
            }}
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
