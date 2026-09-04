import { RefreshCw, RotateCcw } from "lucide-react";
import { useEffect, useState } from "react";

import { publicCleanupExecute, publicCleanupPreflight } from "../../api";
import { requestConfirm } from "../../confirmStore";
import { describeError } from "../../describeError";
import type {
  InteractionActionState,
  PublicCleanupCapability,
  PublicCleanupExecutionReport,
  PublicCleanupKind,
} from "../../types";
import { LoadingState, StatusNotice } from "../States";

type ReversibleKind = Extract<PublicCleanupKind, "like" | "save">;

const COPY: Record<ReversibleKind, { noun: string; command: string; done: string }> = {
  like: { noun: "Tim", command: "Bỏ Tim", done: "Đã bỏ Tim" },
  save: { noun: "Lưu", command: "Bỏ Lưu", done: "Đã bỏ Lưu" },
};

type CleanupView =
  | { state: "idle" }
  | { state: "checking" }
  | { state: "preflightError"; message: string }
  | { state: "blocked"; capability: PublicCleanupCapability }
  | { state: "ready"; capability: PublicCleanupCapability }
  | { state: "executing"; capability: PublicCleanupCapability }
  | { state: "settled"; report: PublicCleanupExecutionReport }
  | { state: "executeUnknown"; message: string };

function newRequestId(): string {
  if (typeof crypto !== "undefined" && typeof crypto.randomUUID === "function") {
    return crypto.randomUUID();
  }
  return `cleanup-${Date.now()}-${Math.random().toString(16).slice(2)}`;
}

function capabilityVi(capability: PublicCleanupCapability): string {
  switch (capability.status) {
    case "sourceNotConfirmed":
      return "Chiến dịch chưa chứng minh hành động này đã tạo hiệu ứng công khai.";
    case "hierarchyRequired":
      return "Máy không cung cấp cây giao diện cần thiết để kiểm tra đúng trạng thái.";
    case "unsupportedUnmeasured":
      return "Chưa có bộ đo đủ tin cậy cho thao tác hoàn tác này.";
    case "readyForTargetProof":
      return "Có thể mở lại bài và kiểm tra đúng card ngay trước thao tác.";
  }
}

function verdictVi(report: PublicCleanupExecutionReport, command: string): string {
  switch (report.evidence?.verdict) {
    case "cleared":
      return `${command} và đã đọc lại trạng thái.`;
    case "alreadyClear":
      return "Hiệu ứng đã được bỏ từ trước; không phát sinh tap.";
    case "noControl":
      return "Không tìm thấy đúng nút trên card; không phát sinh tap.";
    case "stateUnreadable":
      return "Không đọc được trạng thái hiện tại; không phát sinh tap.";
    case "targetChangedBeforeEffect":
      return "Card đã đổi trước thao tác; không phát sinh tap.";
    case "failedBeforeEffect":
      return "Dừng trước thao tác; chưa thay đổi trạng thái công khai.";
    case "targetChangedAfterEffect":
      return "Card đổi sau thao tác; kết quả chưa chắc chắn.";
    case "notConfirmed":
      return "Đã thao tác nhưng chưa xác nhận được trạng thái cuối.";
    case "uncertainAfterEffect":
      return "Đã qua biên hiệu ứng; kết quả chưa chắc chắn.";
    default:
      return "Không nhận được bằng chứng kết thúc đầy đủ.";
  }
}

function isSettledSuccess(report: PublicCleanupExecutionReport): boolean {
  return report.run?.state === "cleared" || report.run?.state === "alreadyClear";
}

function retryIsProvenSafe(report: PublicCleanupExecutionReport): boolean {
  return Boolean(
    report.evidence &&
      !report.evidence.effectBoundaryCrossed &&
      report.run?.state === "failedBeforeEffect",
  );
}

function TechnicalEvidence({
  campaignId,
  assignmentId,
  targetKey,
  sourceEvidence,
  capability,
  report,
}: {
  campaignId: string;
  assignmentId: string;
  targetKey: string;
  sourceEvidence: string | null;
  capability: PublicCleanupCapability;
  report?: PublicCleanupExecutionReport;
}) {
  return (
    <details className="public-cleanup-evidence">
      <summary>Danh tính đích và chứng cứ</summary>
      <dl>
        <div><dt>Chiến dịch</dt><dd><code>{campaignId}</code></dd></div>
        <div><dt>Assignment</dt><dd><code>{assignmentId}</code></dd></div>
        <div><dt>Đích bất biến</dt><dd><code>{targetKey}</code></dd></div>
        {capability.deviceUdid && (
          <div><dt>Thiết bị</dt><dd><code>{capability.deviceUdid}</code></dd></div>
        )}
      </dl>
      {sourceEvidence && <><strong>Chứng cứ nguồn</strong><code>{sourceEvidence}</code></>}
      {report?.run?.targetJson && <><strong>Snapshot đích</strong><code>{report.run.targetJson}</code></>}
      {report?.evidence && (
        <><strong>Readback hoàn tác</strong><code>{JSON.stringify(report.evidence, null, 2)}</code></>
      )}
      <strong>Lý do capability</strong>
      <code>{capability.reason}</code>
    </details>
  );
}

export function PublicCleanupControl({
  campaignId,
  assignmentId,
  targetKey,
  actorLabel,
  kind,
  sourceState,
  sourceEvidence,
}: {
  campaignId: string;
  assignmentId: string;
  targetKey: string;
  actorLabel: string;
  kind: ReversibleKind;
  sourceState: InteractionActionState;
  sourceEvidence: string | null;
}) {
  const [view, setView] = useState<CleanupView>({ state: "idle" });
  const copy = COPY[kind];

  useEffect(() => {
    setView({ state: "idle" });
  }, [assignmentId, campaignId, kind, sourceState, targetKey]);

  const check = async () => {
    setView({ state: "checking" });
    try {
      const capability = await publicCleanupPreflight(campaignId, assignmentId, kind);
      setView({
        state: capability.status === "readyForTargetProof" ? "ready" : "blocked",
        capability,
      });
    } catch (cause) {
      setView({ state: "preflightError", message: describeError(cause) });
    }
  };

  const execute = async (capability: PublicCleanupCapability) => {
    const confirmed = await requestConfirm({
      title: `${copy.command} trên ${actorLabel}?`,
      message:
        "Riviu sẽ mở lại đúng bài, kiểm tra card và trạng thái ngay trước một tap. " +
        "Nếu kết quả sau tap không chắc chắn, tác vụ sẽ dừng và không tự thử lại.",
      confirmLabel: copy.command,
      cancelLabel: "Giữ nguyên",
      danger: true,
    });
    if (!confirmed) return;
    setView({ state: "executing", capability });
    try {
      const report = await publicCleanupExecute(
        newRequestId(),
        campaignId,
        assignmentId,
        kind,
      );
      setView({ state: "settled", report });
    } catch (cause) {
      // The command may have crossed the effect boundary before the response was lost. A
      // transport error here is therefore not a retry button.
      setView({ state: "executeUnknown", message: describeError(cause) });
    }
  };

  if (sourceState !== "confirmed") {
    const reason = sourceState === "uncertain"
      ? "Kết quả nguồn chưa chắc chắn; không được phát sinh thao tác đảo."
      : "Không có hiệu ứng đã xác nhận do chiến dịch này tạo ra.";
    return (
      <span className="public-cleanup-control">
        <button type="button" className="ghost" disabled title={reason}>
          <RotateCcw size={13} aria-hidden="true" /> {copy.command}
        </button>
        <small>{reason}</small>
      </span>
    );
  }

  return (
    <div className="public-cleanup-control" aria-label={`Hoàn tác ${copy.noun}`}>
      {view.state === "idle" && (
        <button type="button" className="ghost" onClick={() => void check()}>
          <RotateCcw size={13} aria-hidden="true" /> Kiểm tra {copy.command.toLocaleLowerCase("vi")}
        </button>
      )}
      {view.state === "checking" && <LoadingState label={`Đang kiểm tra ${copy.noun}…`} />}
      {view.state === "preflightError" && (
        <StatusNotice
          tone="error"
          action={(
            <button type="button" className="ghost" onClick={() => void check()}>
              <RefreshCw size={13} aria-hidden="true" /> Thử lại
            </button>
          )}
        >
          Không kiểm tra được điều kiện hoàn tác: {view.message}
        </StatusNotice>
      )}
      {view.state === "blocked" && (
        <StatusNotice
          tone="warning"
          action={(
            <button type="button" className="ghost" onClick={() => void check()}>
              <RefreshCw size={13} aria-hidden="true" /> Kiểm tra lại
            </button>
          )}
        >
          <strong>{copy.command} đang bị khóa.</strong> {capabilityVi(view.capability)}
          <TechnicalEvidence
            campaignId={campaignId}
            assignmentId={assignmentId}
            targetKey={targetKey}
            sourceEvidence={sourceEvidence}
            capability={view.capability}
          />
        </StatusNotice>
      )}
      {view.state === "ready" && (
        <StatusNotice
          tone="warning"
          action={(
            <button type="button" className="danger" onClick={() => void execute(view.capability)}>
              <RotateCcw size={13} aria-hidden="true" /> Xác nhận {copy.command.toLocaleLowerCase("vi")}
            </button>
          )}
        >
          <strong>Sẵn sàng kiểm tra lại đúng card.</strong> Chưa có thao tác nào được thực hiện.
          <TechnicalEvidence
            campaignId={campaignId}
            assignmentId={assignmentId}
            targetKey={targetKey}
            sourceEvidence={sourceEvidence}
            capability={view.capability}
          />
        </StatusNotice>
      )}
      {view.state === "executing" && <LoadingState label={`Đang ${copy.command.toLocaleLowerCase("vi")} và đọc lại…`} />}
      {view.state === "executeUnknown" && (
        <StatusNotice tone="warning">
          <strong>Không xác định được kết quả.</strong> {view.message}. Không tự chạy lại; hãy kiểm tra trực tiếp trên máy.
        </StatusNotice>
      )}
      {view.state === "settled" && (() => {
        const success = isSettledSuccess(view.report);
        const safeRetry = retryIsProvenSafe(view.report);
        return (
          <StatusNotice
            tone={success ? "success" : view.report.evidence?.effectBoundaryCrossed ? "warning" : "error"}
            action={safeRetry ? (
              <button type="button" className="ghost" onClick={() => void check()}>
                <RefreshCw size={13} aria-hidden="true" /> Kiểm tra lại
              </button>
            ) : undefined}
          >
            <strong>{success ? copy.done : "Hoàn tác chưa hoàn tất"}.</strong>{" "}
            {verdictVi(view.report, copy.done)}
            {view.report.sessionCleanupWarning && (
              <span className="public-cleanup-session-warning">
                Phiên điều khiển đóng chưa sạch: {view.report.sessionCleanupWarning}
              </span>
            )}
            <TechnicalEvidence
              campaignId={campaignId}
              assignmentId={assignmentId}
              targetKey={targetKey}
              sourceEvidence={sourceEvidence}
              capability={view.report.capability}
              report={view.report}
            />
          </StatusNotice>
        );
      })()}
    </div>
  );
}
