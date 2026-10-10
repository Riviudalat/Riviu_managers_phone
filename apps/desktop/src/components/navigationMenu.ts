import type { PageId } from "../types";

/** The sidebar's pages, in its order and with its labels; the command palette lists the same. */
export const MENU: { label: string; items: { id: PageId; label: string }[] }[] = [
  {
    label: "Thiết bị",
    items: [
      { id: "control", label: "Control Center" },
      { id: "deviceSettings", label: "Cài đặt máy" },
    ],
  },
  {
    label: "Automation",
    items: [
      { id: "nurture", label: "Nuôi TikTok" },
      { id: "interaction", label: "Tương tác" },
      { id: "publish", label: "Đăng bài" },
      { id: "myApps", label: "My Apps" },
      { id: "jobs", label: "Lượt chạy" },
      { id: "savedTasks", label: "Tác vụ đã lưu" },
      { id: "scripts", label: "Flow thiết bị" },
    ],
  },
  {
    label: "Tài nguyên",
    items: [
      { id: "accounts", label: "Quản lý tài khoản" },
      { id: "schedules", label: "Lịch chạy" },
      { id: "material", label: "Kho nội dung" },
      { id: "apps", label: "Trung tâm ứng dụng" },
    ],
  },
  {
    label: "Hệ thống",
    items: [
      { id: "api", label: "API" },
      { id: "diagnostics", label: "Chẩn đoán" },
      { id: "settings", label: "Cài đặt" },
      { id: "help", label: "Trợ giúp" },
    ],
  },
];
