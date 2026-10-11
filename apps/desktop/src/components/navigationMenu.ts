import type { PageId } from "../types";

export interface MenuTab {
  id: PageId;
  label: string;
  /** Old names Ctrl+K still finds (GenFarmer's, or an earlier Riviu label). */
  aliases?: string;
}

export interface MenuItem {
  id: PageId;
  /** The page's name: the sidebar's accessible name and the system bar title. */
  label: string;
  /** Visible text where the label does not fit: the pinned row and the narrow rail. */
  short?: string;
  /** Old names Ctrl+K still finds (GenFarmer's, or an earlier Riviu label). */
  aliases?: string;
  /** Separate pages behind one entry, switched by a tab row above them; the first is the entry's target. */
  tabs?: MenuTab[];
}

/** The sidebar's pages, in its order and with its labels; the command palette lists the same. */
export const MENU: { label: string; items: MenuItem[] }[] = [
  {
    label: "Dàn máy",
    items: [
      { id: "control", label: "Trung tâm điều khiển", short: "Trung tâm", aliases: "control center thiet bi" },
      { id: "deviceSettings", label: "Cài đặt máy" },
    ],
  },
  {
    label: "Công việc",
    items: [
      { id: "nurture", label: "Nuôi TikTok", aliases: "automation" },
      { id: "interaction", label: "Tương tác", aliases: "automation" },
      { id: "publish", label: "Đăng bài", aliases: "automation" },
      { id: "jobs", label: "Lượt chạy", aliases: "automation runs" },
      { id: "schedules", label: "Lịch chạy", aliases: "automation schedules" },
      { id: "savedTasks", label: "Tác vụ đã lưu", short: "Tác vụ lưu", aliases: "automation tasks" },
      { id: "scripts", label: "Flow thiết bị", short: "Flow", aliases: "automation" },
    ],
  },
  {
    label: "Nội dung",
    items: [
      { id: "material", label: "Kho nội dung", short: "Nội dung" },
      { id: "accounts", label: "Quản lý tài khoản", short: "Tài khoản" },
      {
        id: "myApps",
        label: "Ứng dụng",
        tabs: [
          { id: "myApps", label: "Quy trình ứng dụng", aliases: "my apps automation" },
          { id: "apps", label: "Gói cài đặt", aliases: "trung tam ung dung apk ipa" },
        ],
      },
    ],
  },
  {
    label: "Hệ thống",
    items: [
      { id: "diagnostics", label: "Chẩn đoán" },
      { id: "api", label: "API" },
      { id: "settings", label: "Cài đặt" },
      { id: "help", label: "Trợ giúp" },
    ],
  },
];

/**
 * The six daily destinations, kept above the groups so they stay on screen at 1093×614 with
 * every group folded. A pinned page is listed here only, never again inside its group.
 */
export const PINNED: PageId[] = ["control", "nurture", "interaction", "publish", "jobs", "help"];

/** True when `page` is the item's own page or one of its tabs. */
export function menuItemOwns(item: MenuItem, page: PageId): boolean {
  return item.id === page || !!item.tabs?.some((tab) => tab.id === page);
}

/** The sidebar entry that owns `page`, if any. */
export function menuItemFor(page: PageId): MenuItem | undefined {
  for (const group of MENU) {
    const item = group.items.find((candidate) => menuItemOwns(candidate, page));
    if (item) return item;
  }
  return undefined;
}
