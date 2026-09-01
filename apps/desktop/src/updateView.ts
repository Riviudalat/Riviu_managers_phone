import type { UpdateStatus } from "./types";

type StatusTone = "ok" | "warn" | "danger" | "info";

export interface UpdateView {
  tone: StatusTone;
  headline: string;
  detail: string | null;
  /**
   * Whether the install button may be enabled.
   *
   * Never true while `busyReason` is set, and never true without a published version.
   * The backend re-checks both before it downloads anything, so this is the courtesy
   * half of the guard — but it is the half the operator sees, and a button that looks
   * available and then refuses teaches people to distrust the panel.
   */
  canInstall: boolean;
}

/**
 * Turn an update check into something to read, keeping the two answers separate.
 *
 * The backend owns the evidence — is there a version, is the fleet busy — and this owns
 * the wording. Deliberately no "check on mount" anywhere in the chain: `status === null`
 * is the normal resting state for a machine nobody has asked, not an error.
 */
export function updateView(
  status: UpdateStatus | null,
  error: string | null,
  installing: boolean,
): UpdateView {
  if (installing) {
    return {
      tone: "info",
      headline: "Đang tải và cài bản mới",
      detail:
        "App sẽ tự đóng để chạy bộ cài. Mọi phiên đã được dừng và các máy đã được nhả trước khi cài.",
      canInstall: false,
    };
  }
  if (error) {
    return {
      tone: "warn",
      headline: "Không kiểm được bản mới",
      detail: error,
      canInstall: false,
    };
  }
  if (!status) {
    return {
      tone: "info",
      headline: "Chưa kiểm bản mới",
      detail: 'Bấm "Kiểm bản mới" để kiểm tra theo yêu cầu.',
      canInstall: false,
    };
  }
  if (!status.available) {
    return {
      tone: "ok",
      headline: `Đang chạy bản mới nhất (${status.current})`,
      detail: null,
      canInstall: false,
    };
  }
  const version = status.version ?? "?";
  if (status.busyReason) {
    return {
      tone: "warn",
      headline: `Có bản ${version} — chưa cài được`,
      detail: status.busyReason,
      canInstall: false,
    };
  }
  return {
    tone: "info",
    headline: `Có bản ${version}`,
    detail: `Đang chạy ${status.current}. Fleet đang rảnh, cài được ngay.`,
    canInstall: true,
  };
}
