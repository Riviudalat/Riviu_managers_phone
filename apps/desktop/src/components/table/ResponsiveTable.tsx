import { useId, useState, type ReactNode } from "react";
import { ArrowDown, ArrowUp, Columns3, RotateCcw, Search } from "lucide-react";
import { compareTableValues, readTableView, saveTableView, type TableView } from "./tableView";

export interface ResponsiveTableColumn<Row> {
  id: string;
  label: string;
  render: (row: Row) => ReactNode;
  className?: string;
  sortValue?: (row: Row) => string | number;
  required?: boolean;
}

interface ResponsiveTableProps<Row> {
  label: string;
  columns: ResponsiveTableColumn<Row>[];
  rows: Row[];
  keyForRow: (row: Row) => string;
  labelForRow?: (row: Row) => string;
  onRowOpen?: (row: Row) => void;
  empty?: ReactNode;
  viewKey?: string;
  searchText?: (row: Row) => string;
}

export function ResponsiveTable<Row>(props: ResponsiveTableProps<Row>) {
  return <ScopedTable key={props.viewKey ?? ""} {...props} />;
}

function ScopedTable<Row>({
  label, columns, rows, keyForRow, labelForRow, onRowOpen, empty, viewKey, searchText,
}: ResponsiveTableProps<Row>) {
  const tableId = useId();
  const [view, setView] = useState(() => readTableView(viewKey));
  const [query, setQuery] = useState("");
  const update = (next: TableView) => { setView(next); saveTableView(viewKey, next); };
  const visible = columns.filter((column, index) => index === 0 || column.required || !view.hidden.includes(column.id));
  const filtered = rows.filter((row) => !searchText || searchText(row).toLocaleLowerCase("vi").includes(query.trim().toLocaleLowerCase("vi")));
  const sortColumn = columns.find((column) => column.id === view.sort?.column);
  const displayed = sortColumn?.sortValue ? [...filtered].sort((left, right) =>
    compareTableValues(sortColumn.sortValue!(left), sortColumn.sortValue!(right)) * (view.sort?.direction === "desc" ? -1 : 1)) : filtered;
  if (!rows.length && !viewKey) return <>{empty ?? null}</>;
  return (
    <section className="table-surface" aria-label={`${label}: hiển thị`}>
      {viewKey && <div className="table-toolbar">
        {searchText && <label className="search-field">
          <Search size={15} aria-hidden="true" />
          <span className="visually-hidden">Tìm trong {label}</span>
          <input type="search" placeholder="Tìm kiếm…" value={query} onChange={(event) => setQuery(event.target.value)} />
        </label>}
        <span className="table-result-count" role="status">{displayed.length}/{rows.length}</span>
        <details className="table-columns" onKeyDown={(event) => {
          if (event.key === "Escape" && event.currentTarget.open) {
            event.preventDefault();
            event.stopPropagation();
            event.currentTarget.open = false;
            event.currentTarget.querySelector("summary")?.focus();
          }
        }}>
          <summary aria-label={`Chọn cột: ${label}`} title="Chọn cột"><Columns3 size={16} aria-hidden="true" /></summary>
          <div className="table-columns-menu">
            {columns.map((column, index) => <label key={column.id}>
              <input type="checkbox" checked={visible.includes(column)} disabled={index === 0 || column.required}
                onChange={(event) => update({ ...view, hidden: event.target.checked ? view.hidden.filter((id) => id !== column.id) : [...view.hidden, column.id] })} />
              {column.label}
            </label>)}
            <button type="button" className="ghost" onClick={() => update({ hidden: [], sort: null })}>
              <RotateCcw size={14} aria-hidden="true" /> Mặc định
            </button>
          </div>
        </details>
      </div>}
      {!displayed.length ? <div className="table-empty" role="status">{rows.length ? "Không có kết quả phù hợp" : empty === undefined ? "Chưa có dữ liệu" : empty}</div> :
        <div className="responsive-table-wrap" tabIndex={0} role="region" aria-label={label}>
          <table id={tableId} className="responsive-table" aria-label={label}>
            <thead><tr>{visible.map((column) => <th key={column.id} scope="col" className={column.className}
              aria-sort={view.sort?.column === column.id ? view.sort.direction === "asc" ? "ascending" : "descending" : undefined}>
              {column.sortValue ? <button type="button" className="table-sort" onClick={() => update({ ...view,
                sort: { column: column.id, direction: view.sort?.column === column.id && view.sort.direction === "asc" ? "desc" : "asc" } })}>
                {column.label}{view.sort?.column === column.id && (view.sort.direction === "asc" ? <ArrowUp size={13} /> : <ArrowDown size={13} />)}
              </button> : column.label}
            </th>)}</tr></thead>
            <tbody>{displayed.map((row) => <tr key={keyForRow(row)} tabIndex={onRowOpen ? 0 : undefined} aria-label={labelForRow?.(row)}
              onClick={onRowOpen ? (event) => {
                if (!(event.target as HTMLElement).closest('button, a, input, select, textarea, summary, [role="button"]')) onRowOpen(row);
              } : undefined}
              onKeyDown={onRowOpen ? (event) => {
                if (event.target === event.currentTarget && event.key === "Enter") { event.preventDefault(); onRowOpen(row); }
              } : undefined}>
              {visible.map((column) => <td key={column.id} data-label={column.label} className={column.className}>{column.render(row)}</td>)}
            </tr>)}</tbody>
          </table>
        </div>}
    </section>
  );
}
