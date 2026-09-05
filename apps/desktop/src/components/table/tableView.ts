export interface TableView {
  hidden: string[];
  sort: { column: string; direction: "asc" | "desc" } | null;
}

export function readTableView(key?: string): TableView {
  const empty: TableView = { hidden: [], sort: null };
  if (!key) return empty;
  try {
    const value: unknown = JSON.parse(localStorage.getItem(`riviu.table.${key}.v1`) ?? "null");
    if (!value || typeof value !== "object") return empty;
    const record = value as Record<string, unknown>;
    const hidden = Array.isArray(record.hidden)
      ? record.hidden.filter((id): id is string => typeof id === "string")
      : [];
    const sort = record.sort as Partial<NonNullable<TableView["sort"]>> | null;
    return { hidden, sort: sort && typeof sort.column === "string" &&
      (sort.direction === "asc" || sort.direction === "desc")
      ? { column: sort.column, direction: sort.direction } : null };
  } catch { return empty; }
}

export function saveTableView(key: string | undefined, view: TableView): void {
  if (!key) return;
  try { localStorage.setItem(`riviu.table.${key}.v1`, JSON.stringify(view)); }
  catch { /* View preferences are optional; the operation state is not stored here. */ }
}

export function compareTableValues(left: string | number, right: string | number): number {
  return typeof left === "number" && typeof right === "number"
    ? left - right
    : String(left).localeCompare(String(right), "vi", { numeric: true, sensitivity: "base" });
}
