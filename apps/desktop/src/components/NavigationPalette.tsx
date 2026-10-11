import { CommandPalette, type PaletteGroup } from "./ui/CommandPalette";
import { MENU as SIDEBAR_MENU } from "./navigationMenu";
import type { PageId } from "../types";

export interface PaletteDevice {
  udid: string;
  /** "Máy N · tên", the label every other surface uses for this phone. */
  label: string;
  /** The tile's status line: "Sẵn sàng", "Đang chạy: Nuôi TikTok", "Cần xử lý: …" */
  status: string;
  handle?: string;
}

interface Props {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  devices: PaletteDevice[];
  /** Phones that need the operator; `null` while the control state is still unread. */
  attentionUdids: string[] | null;
  onPage: (page: PageId) => void;
  onOrchestration: () => void;
  onGoToDevice: (udid: string) => void;
  onSelectDevices: (udids: string[]) => void;
}

/**
 * Ctrl+K, phase 1: go to a page, go to a phone, select the phones that need attention.
 *
 * Nothing here runs work on a phone. "Chạy tác vụ đã lưu" opens the saved-task list, whose own
 * Chạy button asks for confirmation as it always has; the per-phone function menu stays the
 * place for phone actions.
 */
export function NavigationPalette({
  open,
  onOpenChange,
  devices,
  attentionUdids,
  onPage,
  onOrchestration,
  onGoToDevice,
  onSelectDevices,
}: Props) {
  const attention = attentionUdids;
  const groups: PaletteGroup[] = [
    {
      label: "Lệnh",
      items: [
        attention === null
          ? { id: "attention", label: "Chọn máy cần xử lý", hint: "Đang đọc trạng thái máy…", keywords: "can xem loi", disabled: true, run: () => {} }
          : attention.length === 0
            ? { id: "attention", label: "Chọn máy cần xử lý", hint: "Không có máy nào cần xử lý", keywords: "can xem loi", disabled: true, run: () => {} }
            : {
                id: "attention",
                label: `Chọn ${attention.length} máy cần xử lý`,
                hint: "Chỉ chọn, không chạy gì",
                keywords: "can xem loi",
                run: () => onSelectDevices(attention),
              },
        {
          id: "saved-task",
          label: "Chạy tác vụ đã lưu…",
          hint: "Mở danh sách; Chạy luôn hỏi xác nhận",
          keywords: "tac vu da luu",
          run: () => onPage("savedTasks"),
        },
      ],
    },
    {
      label: "Trang",
      items: [
        ...SIDEBAR_MENU.flatMap((group) =>
          group.items.map((item) => ({
            id: `page-${item.id}`,
            label: item.label,
            hint: group.label,
            keywords: "trang di toi",
            run: () => onPage(item.id),
          })),
        ),
        {
          id: "page-orchestration",
          label: "Điều phối",
          hint: "Automation",
          keywords: "trang di toi orchestration flow",
          run: onOrchestration,
        },
      ],
    },
    {
      label: "Máy",
      items: devices.map((device) => ({
        id: `device-${device.udid}`,
        label: `${device.label} · ${device.status}`,
        hint: "Đi tới và chọn",
        keywords: device.handle ? `@${device.handle.replace(/^@+/, "")}` : undefined,
        run: () => onGoToDevice(device.udid),
      })),
    },
  ];

  return (
    <CommandPalette
      open={open}
      onOpenChange={onOpenChange}
      label="Bảng lệnh"
      placeholder="Gõ số máy, tên trang hoặc lệnh…"
      groups={groups}
    />
  );
}
