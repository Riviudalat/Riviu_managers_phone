import {
  Children,
  cloneElement,
  isValidElement,
  useId,
  useState,
  type ReactElement,
  type ReactNode,
} from "react";

type NoticeElement = ReactElement<{ tone?: string; children?: ReactNode; action?: ReactNode }>;

/** The words a notice says, so two producers reporting one condition count as one notice. */
function textOf(node: ReactNode): string {
  if (node == null || typeof node === "boolean") return "";
  if (typeof node === "string" || typeof node === "number") return String(node);
  if (Array.isArray(node)) return node.map(textOf).join("");
  if (isValidElement<{ children?: ReactNode }>(node)) return textOf(node.props.children);
  return "";
}

/**
 * App-level system notices (`Banner`s) as one strip. Identical notices merge with a count;
 * the most severe stays fully visible with its own action, and the rest fold behind
 * "+N thông báo hệ thống" instead of stacking 60–180 px of banners over every page at 768 px.
 * Workflow blockers do not belong here: they stay inline next to the CTA they block.
 */
export function SystemNotices({ children }: { children: ReactNode }) {
  const [open, setOpen] = useState(false);
  const restId = useId();
  const merged: { key: string; element: NoticeElement; count: number }[] = [];
  for (const child of Children.toArray(children)) {
    if (!isValidElement(child)) continue;
    const element = child as NoticeElement;
    const key = `${element.props.tone ?? ""}:${textOf(element.props.children)}`;
    const existing = merged.find((entry) => entry.key === key);
    if (existing) existing.count += 1;
    else merged.push({ key, element, count: 1 });
  }
  // Stable: errors first, otherwise the order the shell lists them in.
  merged.sort((a, b) => Number(b.element.props.tone === "error") - Number(a.element.props.tone === "error"));
  if (!merged.length) return null;
  const render = ({ element, count }: (typeof merged)[number], extraAction?: ReactNode) =>
    cloneElement(element, {
      action: element.props.action || extraAction ? <>{element.props.action}{extraAction}</> : undefined,
      children: <>{element.props.children}{count > 1 && <span className="system-notice-repeat"> (×{count})</span>}</>,
    });
  const [first, ...rest] = merged;
  if (!rest.length) return render(first);
  return (
    <section className="system-notices" aria-label={`Thông báo hệ thống (${merged.length})`}>
      {render(first, (
        <button type="button" className="ghost system-notices-toggle" aria-expanded={open}
          aria-controls={restId} onClick={() => setOpen((value) => !value)}>
          {open ? "Thu gọn" : `+${rest.length} thông báo hệ thống`}
        </button>
      ))}
      <div id={restId} className="system-notices-rest" hidden={!open}>
        {rest.map((entry) => <div key={entry.key}>{render(entry)}</div>)}
      </div>
    </section>
  );
}
