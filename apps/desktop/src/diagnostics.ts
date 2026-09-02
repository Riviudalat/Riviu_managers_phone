import { describeError } from "./describeError";
import type { DeviceHealthReport, DeviceInfo } from "./types";

/** The only words a health check may use for its conclusion. */
export type HealthStatus = "pass" | "warning" | "fail" | "unknown" | "notApplicable";

export interface DeviceHealthCheck {
  id:
    | "roster"
    | "transport"
    | "adb"
    | "agentCache"
    | "agentLive"
    | "agentCapabilities"
    | "helperInstalled"
    | "helperReachable"
    | "root"
    | "geometry"
    | "stream"
    | "tiktok";
  label: string;
  status: HealthStatus;
  /** Human-readable answer. `unknown` is always an unanswered state, never a negative. */
  summary: string;
  /** Backend evidence retained for the expandable, accessible details view. */
  detail?: string;
}

export interface FleetHealthRow {
  device: DeviceInfo;
  report?: DeviceHealthReport;
  checks?: DeviceHealthCheck[];
  error?: string;
  /** Raw error evidence stays out of the primary surface and is exposed in disclosure only. */
  errorDetail?: string;
}

const AGENT_CACHE: Record<DeviceHealthReport["agent"]["state"], Pick<DeviceHealthCheck, "status" | "summary">> = {
  ready: { status: "pass", summary: "Sẵn sàng trong cache" },
  starting: { status: "warning", summary: "Đang khởi động" },
  missing: { status: "fail", summary: "Agent chưa có" },
  repairRequired: { status: "fail", summary: "Agent cần sửa" },
  error: { status: "fail", summary: "Agent báo lỗi" },
  unknown: { status: "unknown", summary: "Chưa có câu trả lời trong cache" },
};

function check(
  id: DeviceHealthCheck["id"],
  label: string,
  status: HealthStatus,
  summary: string,
  detail?: string,
): DeviceHealthCheck {
  return { id, label, status, summary, ...(detail ? { detail } : {}) };
}

function appliesToAndroid(
  device: DeviceInfo,
): boolean {
  // Platform is roster provenance. A cached response can be stale, wrong, or represent a
  // different backend; it must never promote an iOS device into Android diagnostics.
  return device.platform === "android";
}

/**
 * Converts the command's optional, probe-specific fields into one UI contract.
 *
 * This deliberately does not infer `false` from `null`: an unanswered command says what the
 * app learned (unknown/not applicable), not what an operator might fear happened to the phone.
 */
export function normalizeDeviceHealth(
  device: DeviceInfo,
  report: DeviceHealthReport,
): DeviceHealthCheck[] {
  const android = appliesToAndroid(device);
  const cache = AGENT_CACHE[report.agent.state];
  const notes = report.notes.length ? report.notes.join(" ") : undefined;
  const roster = report.rosterStatus;

  const rosterCheck = !roster
    ? check("roster", "Danh sách máy", "unknown", "Máy chưa có trong danh sách")
    : roster === "ready"
      ? check("roster", "Danh sách máy", "pass", "Sẵn sàng")
      : roster === "error"
        ? check("roster", "Danh sách máy", "fail", "Danh sách máy báo lỗi")
        : roster === "disconnected"
          ? check("roster", "Danh sách máy", "warning", "Máy đang ngoại tuyến")
          : check("roster", "Danh sách máy", "warning", `Trạng thái: ${roster}`);

  const transport = !android
    ? check("transport", "Kết nối", "notApplicable", "Chỉ đọc qua backend Android")
    : roster === "ready" || roster === "connected"
      ? check("transport", "Kết nối", "pass", "Đã thấy qua transport")
      : roster === "disconnected" || roster === "error"
        ? check("transport", "Kết nối", "fail", "Chưa có transport hoạt động", notes)
        : check("transport", "Kết nối", "unknown", "Chưa có bằng chứng transport", notes);

  const adb = !android
    ? check("adb", "ADB", "notApplicable", "Chỉ có ở Android")
    : report.adbPath && report.adbVersion
      ? check("adb", "ADB", "pass", "Đã nhận diện", [report.adbOrigin, report.adbPath, report.adbVersion].filter(Boolean).join("\n"))
      : report.adbPath
        ? check("adb", "ADB", "warning", "Chưa đọc được phiên bản", report.adbPath)
        : check("adb", "ADB", "unknown", "Chưa có Android backend", notes);

  const agentLive = !android
    ? check("agentLive", "Agent đang chạy", "notApplicable", "Chỉ có ở Android")
    : report.agentReadyNow === true
      ? check("agentLive", "Agent đang chạy", "pass", "Đang trả lời")
      : report.agentReadyNow === false
        ? check("agentLive", "Agent đang chạy", "fail", "Chưa trả lời", notes)
        : check("agentLive", "Agent đang chạy", "unknown", "Chưa hỏi được", notes);

  const helperInstalled = !android
    ? check("helperInstalled", "Riviu helper", "notApplicable", "Chỉ có ở Android")
    : report.helperInstalled === true
      ? check("helperInstalled", "Riviu helper", "pass", "Đã cài")
      : report.helperInstalled === false
        ? check("helperInstalled", "Riviu helper", "fail", "Chưa cài", notes)
        : check("helperInstalled", "Riviu helper", "unknown", "Chưa hỏi được", notes);

  const helperReachable = !android
    ? check("helperReachable", "Kết nối helper", "notApplicable", "Chỉ có ở Android")
    : report.helperReachable === true
      ? check("helperReachable", "Kết nối helper", "pass", "Đang trả lời")
      : report.helperReachable === false
        ? check("helperReachable", "Kết nối helper", "warning", "Chưa với tới được", notes)
        : check("helperReachable", "Kết nối helper", "unknown", "Chưa hỏi trong phiên này", notes);

  const agentCapabilities = !android
    ? check("agentCapabilities", "Khả năng Riviu", "notApplicable", "Chỉ có ở Android")
    : report.agentFeatures === null || report.agentFeatures === undefined
      ? check("agentCapabilities", "Khả năng Riviu", "unknown", "Chưa có bằng chứng từ agent", notes)
      : report.agentAuthReady === false
        ? check("agentCapabilities", "Khả năng Riviu", "warning", "Agent chưa xác thực", report.agentFeatures.join(", "))
        : check(
          "agentCapabilities",
          "Khả năng Riviu",
          report.agentFeatures.length ? "pass" : "warning",
          report.agentFeatures.length ? "Đã đọc capability" : "Chưa có capability",
          report.agentFeatures.join(", ") || undefined,
        );

  const root = !android
    ? check("root", "Quyền root", "notApplicable", "Chỉ có ở Android")
    : !report.root
      ? check("root", "Quyền root", "unknown", "Chưa hỏi được", notes)
      : report.root.hasSu
        ? check("root", "Quyền root", "pass", "Có su (Magisk)")
        : report.root.shellIsRoot
          ? check("root", "Quyền root", "pass", "adb shell là root")
          : check("root", "Quyền root", "warning", "Chưa có root");

  const tiktok = !android
    ? check("tiktok", "TikTok", "notApplicable", "Chỉ có ở Android")
    : report.tiktokPackage
      ? check(
        "tiktok",
        "TikTok",
        "pass",
        [report.tiktokPackage, report.tiktokVersion, report.tiktokLocale].filter(Boolean).join(" · "),
      )
      : check("tiktok", "TikTok", "unknown", "Chưa đọc được build", notes);

  const geometry = !android
    ? check("geometry", "Màn hình", "notApplicable", "Chỉ có ở Android")
    : report.geometry
      ? check(
        "geometry",
        "Màn hình",
        "pass",
        `${report.geometry.width}×${report.geometry.height} · xoay ${report.geometry.rotation}`,
        `density ${report.geometry.density}; stream generation ${report.streamGeneration ?? "chưa có"}`,
      )
      : check("geometry", "Màn hình", "unknown", "Chưa đọc được kích thước hoặc hướng", notes);

  const stream = report.streamGeneration === null || report.streamGeneration === undefined
    ? check("stream", "Luồng xem", "unknown", "Chưa đọc được thế hệ luồng", notes)
    : report.streamGeneration === 0
      ? check("stream", "Luồng xem", "warning", "Chưa có luồng xem hoạt động", "generation 0")
      : check("stream", "Luồng xem", "pass", `Thế hệ ${report.streamGeneration}`);

  return [
    rosterCheck,
    transport,
    adb,
    check("agentCache", "Agent đã ghi nhận", cache.status, cache.summary, report.agent.message ?? undefined),
    agentLive,
    agentCapabilities,
    helperInstalled,
    helperReachable,
    root,
    geometry,
    stream,
    tiktok,
  ];
}

export type ReadDeviceHealth = (udid: string) => Promise<DeviceHealthReport>;

export interface HealthLimiter {
  run<T>(operation: () => Promise<T>): Promise<T>;
}

const HEALTH_RESPONSE_DEADLINE_MS = 35_000;

/** The UI must leave Loading even when IPC or a backend probe never settles. */
export async function withHealthDeadline<T>(
  operation: () => Promise<T>,
  deadlineMs = HEALTH_RESPONSE_DEADLINE_MS,
): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  try {
    return await Promise.race([
      operation(),
      new Promise<T>((_resolve, reject) => {
        timer = setTimeout(
          () => reject(new Error(`Chẩn đoán máy không trả lời sau ${Math.ceil(deadlineMs / 1000)} giây`)),
          deadlineMs,
        );
      }),
    ]);
  } finally {
    if (timer !== undefined) clearTimeout(timer);
  }
}

/** One queue is shared by the fleet's initial pass and row retries. */
export function createHealthLimiter(maxConcurrent = 4): HealthLimiter {
  const limit = Math.max(1, maxConcurrent);
  let active = 0;
  const queued: Array<() => void> = [];

  const release = () => {
    active -= 1;
    queued.shift()?.();
  };

  return {
    async run<T>(operation: () => Promise<T>): Promise<T> {
      if (active >= limit) await new Promise<void>((resolve) => queued.push(resolve));
      active += 1;
      try {
        return await operation();
      } finally {
        release();
      }
    },
  };
}

/**
 * Reads a fleet gradually with a fixed ceiling. No callback or command here mutates a phone;
 * rows become available in completion order while the returned snapshot keeps roster order.
 */
export async function loadFleetHealth(
  devices: DeviceInfo[],
  read: ReadDeviceHealth,
  onRow?: (row: FleetHealthRow) => void,
  maxConcurrent = 4,
  limiter: HealthLimiter = createHealthLimiter(maxConcurrent),
): Promise<FleetHealthRow[]> {
  const rows: FleetHealthRow[] = Array.from(devices, (device) => ({ device }));
  await Promise.all(devices.map(async (device, index) => {
    let row: FleetHealthRow;
    try {
      const report = await limiter.run(() => withHealthDeadline(() => read(device.udid)));
      row = { device, report, checks: normalizeDeviceHealth(device, report) };
    } catch (error) {
      row = {
        device,
        error: "Không đọc được trạng thái máy. Hãy kiểm lại.",
        errorDetail: describeError(error),
      };
    }
    rows[index] = row;
    onRow?.(row);
  }));
  return rows;
}

/** A stable, explicit export payload rather than a DOM scrape. */
export function fleetHealthJson(rows: FleetHealthRow[]): string {
  return JSON.stringify({ generatedAt: new Date().toISOString(), rows }, null, 2);
}
