import type { LucideIcon } from "lucide-react";
import {
  Camera,
  CirclePlay,
  CircleStop,
  Crosshair,
  GitBranch,
  House,
  Keyboard,
  MousePointerClick,
  MoveUp,
  PowerOff,
  Rocket,
  ScanSearch,
  Timer,
} from "lucide-react";

import type { ActionKind, JsonObject, JsonValue } from "../../types";

/**
 * How each action kind is labelled and drawn, and how one summarises itself.
 *
 * Read by the node, the palette and the inspector, so it cannot live in any one of
 * them -- and a table plus a pure function exported from a component file is what costs
 * that file its Fast Refresh.
 */
export const ACTION_PRESENTATION: Partial<
  Record<ActionKind, { label: string; icon: LucideIcon }>
> = {
  start: { label: "Bắt đầu", icon: CirclePlay },
  end: { label: "Kết thúc", icon: CircleStop },
  launchApp: { label: "Mở ứng dụng", icon: Rocket },
  terminateApp: { label: "Tắt ứng dụng", icon: PowerOff },
  wait: { label: "Chờ", icon: Timer },
  tap: { label: "Chạm", icon: MousePointerClick },
  swipe: { label: "Vuốt", icon: MoveUp },
  autoSwipe: { label: "Tự động vuốt", icon: MoveUp },
  typeText: { label: "Gõ chữ", icon: Keyboard },
  screenshot: { label: "Chụp màn hình", icon: Camera },
  home: { label: "Về màn hình chính", icon: House },
  assertVisible: { label: "Kiểm tra hiển thị", icon: ScanSearch },
  tapVision: { label: "Chạm theo ảnh", icon: Crosshair },
  ifVision: { label: "Nếu thấy ảnh", icon: GitBranch },
};

export function summarizeAction(kind: ActionKind, config: JsonObject): string {
  const text = (key: string) => {
    const value = config[key];
    return typeof value === "string" ? value : "";
  };
  switch (kind) {
    case "launchApp":
    case "terminateApp":
      return text("bundleId");
    case "wait":
      return typeof config.durationMs === "number" ? `${config.durationMs} ms` : "";
    case "tap": {
      const coordinates = [objectNumber(config.point, "x"), objectNumber(config.point, "y")]
        .filter((value): value is number => value !== null)
        .join(", ");
      return text("accessibilityId") || coordinates;
    }
    case "swipe":
      return `Swipe${
        typeof config.durationMs === "number" ? ` ${config.durationMs} ms` : ""
      }`;
    case "autoSwipe":
      return typeof config.count === "number"
        ? `${config.count} lần`
        : typeof config.durationMs === "number"
          ? `${config.durationMs} ms`
          : "chưa chọn giới hạn";
    case "typeText":
      return `${text("text").length} characters`;
    case "screenshot":
      return text("label");
    case "assertVisible":
      return text("accessibilityId");
    case "tapVision":
    case "ifVision": {
      const hasTemplate = text("templatePngBase64").length > 0;
      // Not `toFixed(2)`. The field accepts any number in [0, 1], so a stored 0.854 rendered as
      // "0.85" -- and then a match score of 0.852 fails while the summary on the canvas implies it
      // passes. Print what is stored.
      const threshold =
        typeof config.threshold === "number" ? String(config.threshold) : "?";
      return hasTemplate ? `vision ≥ ${threshold}` : "no template";
    }
    default:
      return "";
  }
}

function objectNumber(value: JsonValue | undefined, key: string): number | null {
  if (typeof value !== "object" || value === null || Array.isArray(value)) return null;
  const field = value[key];
  return typeof field === "number" && Number.isFinite(field) ? field : null;
}
