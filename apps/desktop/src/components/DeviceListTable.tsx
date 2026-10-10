import { useState } from "react";
import { MoreHorizontal } from "lucide-react";
import { machineNumberLabel } from "../deviceNaming";
import type { DeviceOperationalView } from "../deviceWork";
import type { DeviceEvidence } from "../deviceEvidence";
import { timeAgoVi } from "../timeAgo";
import type { DeviceInfo } from "../types";
import { TableColumnChooser } from "./table/ResponsiveTable";
import { readTableView, saveTableView, type TableView } from "./table/tableView";

export interface DeviceListRow {
  device: DeviceInfo;
  machineNumber: number;
  name: string;
  handle?: string;
  status: DeviceOperationalView;
  evidence: DeviceEvidence;
}

interface Props {
  rows: DeviceListRow[];
  selected: string[];
  nowMs: number;
  onSelect: (udid: string, additive: boolean) => void;
  onToggle: (udid: string) => void;
  onOpen: (udid: string) => void;
  onDetails: (udid: string) => void;
  onHelperMaintenance: (udid: string) => void;
}

const COLUMNS = [
  { id: "machine", label: "Máy" },
  { id: "account", label: "Tài khoản" },
  { id: "work", label: "Việc đang chạy" },
  { id: "attention", label: "Cần xử lý" },
  // The header says "Bằng chứng" so it fits its 14% column; the chooser and tooltip say it in full.
  { id: "evidence", label: "Bằng chứng gần nhất", header: "Bằng chứng" },
  { id: "connection", label: "Kết nối" },
] as const;
type ColumnId = (typeof COLUMNS)[number]["id"];

const VIEW_KEY = "devices";

function evidenceText(evidence: DeviceEvidence, nowMs: number): string {
  if (evidence.state === "missing") return "Chưa có";
  const age = timeAgoVi(evidence.at, new Date(nowMs));
  return evidence.state === "stale" ? `Cũ · ${age}` : age;
}

/**
 * The compact device list: one 34 px line per phone, so 15+ phones fit a 1366×768 window.
 * Every cell truncates on one line; the row's accessible name and each cell's tooltip keep
 * the full text. The tile wall's vocabulary is reused as is: the run label and step are the
 * work, and only a phone that needs the operator carries an attention reason.
 */
export function DeviceListTable({ rows, selected, nowMs, onSelect, onToggle, onOpen, onDetails, onHelperMaintenance }: Props) {
  const [view, setView] = useState(() => readTableView(VIEW_KEY));
  const update = (next: TableView) => { setView(next); saveTableView(VIEW_KEY, next); };
  const shown = (id: ColumnId) => id === "machine" || !view.hidden.includes(id);

  return (
    <table className="device-table" aria-label="Danh sách thiết bị">
      <caption className="visually-hidden">Danh sách thiết bị đang hiển thị</caption>
      <thead>
        <tr>
          <th scope="col" className="device-table-check">
            <span className="visually-hidden">Chọn</span>
          </th>
          {COLUMNS.filter((column) => shown(column.id)).map((column) => (
            <th key={column.id} scope="col" className={`device-table-${column.id}`} title={column.label}>
              {"header" in column ? column.header : column.label}
            </th>
          ))}
          <th scope="col" className="device-table-actions">
            <span className="visually-hidden">Thao tác</span>
            <TableColumnChooser label="Danh sách thiết bị" columns={[...COLUMNS]} view={view} onChange={update} />
          </th>
        </tr>
      </thead>
      <tbody>
        {rows.map(({ device, machineNumber, name, handle, status, evidence }) => {
          const sel = selected.includes(device.udid);
          const label = machineNumberLabel(machineNumber);
          const statusLabel = status.ownerLabel ? `${status.label} · ${status.ownerLabel}` : status.label;
          const username = handle?.trim().replace(/^@+/, "");
          const account = username ? `@${username}` : "";
          // Same rule as the tile strip: a healthy phone's reason only explains how control
          // gets checked, so it stays in the status tooltip and out of "Cần xử lý".
          const attention = status.kind === "warning" ? status.reason ?? statusLabel : "";
          const evidenceLabel = evidenceText(evidence, nowMs);
          const evidenceTitle = evidence.state === "missing"
            ? "Chưa có bản ghi lượt chạy hay mẫu đo tải nào cho máy này"
            : `${evidence.state === "stale" ? "Đã cũ: " : ""}${evidence.source}, ${new Date(evidence.at).toLocaleString("vi-VN")}`;
          const accessibleName = [
            label,
            name,
            statusLabel,
            status.step,
            account,
            attention && attention !== statusLabel ? attention : undefined,
            `bằng chứng gần nhất: ${evidenceLabel}`,
            sel ? "đã chọn" : undefined,
          ].filter(Boolean).join(", ");
          return (
            <tr
              key={device.udid}
              className={`${sel ? "selected" : ""} ${status.kind === "warning" ? "attention" : ""}`.trim() || undefined}
              tabIndex={0}
              aria-label={accessibleName}
              onClick={(e) => onSelect(device.udid, e.metaKey || e.ctrlKey)}
              onKeyDown={(event) => {
                if (event.target !== event.currentTarget) return;
                if (event.key !== "Enter" && event.key !== " ") return;
                event.preventDefault();
                onSelect(device.udid, event.metaKey || event.ctrlKey || event.shiftKey);
              }}
              onDoubleClick={() => onOpen(device.udid)}
            >
              <td className="device-table-check">
                <input
                  type="checkbox"
                  aria-label={`Chọn ${label}`}
                  checked={sel}
                  onChange={() => onToggle(device.udid)}
                  onClick={(e) => e.stopPropagation()}
                />
              </td>
              {shown("machine") && (
                <td className="device-table-machine" title={`${label} · ${name}`}>
                  <strong>{label}</strong>
                  <span className="device-table-alias"> · {name}</span>
                </td>
              )}
              {shown("account") && (
                <td className="device-table-account" title={account || "Chưa gắn tài khoản"}>
                  {account || <span className="device-table-missing">Chưa gắn</span>}
                </td>
              )}
              {shown("work") && (
                <td className="device-table-work" title={[statusLabel, status.step].filter(Boolean).join(" · ")}>
                  <span className={`chip ${status.tone}`} title={status.reason}>{statusLabel}</span>
                  {status.step && <span className="device-table-step"> {status.step}</span>}
                </td>
              )}
              {shown("attention") && (
                <td className="device-table-attention" title={attention || undefined}>
                  {status.helperRecovery ? (
                    <button type="button" className="link" title={status.reason} onClick={(event) => {
                      event.stopPropagation();
                      onHelperMaintenance(device.udid);
                    }}>Khôi phục helper</button>
                  ) : attention}
                </td>
              )}
              {shown("evidence") && (
                <td className={`device-table-evidence is-${evidence.state}`} title={evidenceTitle}>
                  {evidence.state === "missing" ? <span className="device-table-missing">{evidenceLabel}</span> : evidenceLabel}
                </td>
              )}
              {shown("connection") && <td className="device-table-connection">{device.connection.toUpperCase()}</td>}
              <td className="device-table-actions">
                <div className="device-row-actions">
                  <button
                    type="button"
                    className="link"
                    onClick={(e) => {
                      e.stopPropagation();
                      onOpen(device.udid);
                    }}
                  >
                    Mở
                  </button>
                  <button
                    type="button"
                    className="icon-button"
                    aria-label={`Xem chi tiết ${label}`}
                    title="Xem chi tiết"
                    onClick={(event) => {
                      event.stopPropagation();
                      onDetails(device.udid);
                    }}
                  >
                    <MoreHorizontal size={16} />
                  </button>
                </div>
              </td>
            </tr>
          );
        })}
      </tbody>
    </table>
  );
}
