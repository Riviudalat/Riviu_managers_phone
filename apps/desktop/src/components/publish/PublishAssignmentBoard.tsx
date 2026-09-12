import { useEffect, useRef, useState, type ReactNode } from "react";
import { Check, Pencil, Plus, Redo2, Search, Undo2 } from "lucide-react";
import type { DeviceInfo, DeviceMeta, PublishBundle } from "../../types";
import { PublishMedia } from "./PublishMedia";
import { PublishPager } from "./PublishPager";
import { usePublishPageSize } from "./usePublishPageSize";
import { PublishDialog } from "./PublishDialog";
import { assignDevice, fillAssignments } from "./publishAssignments";
import { orderDevicesByNumber, tileNumber } from "../../deviceNaming";
import { MachineChoice } from "../MachineChoice";

type Props = {
  scopeControl?: ReactNode;
  bundles: PublishBundle[];
  assignments: Record<string, string>;
  eligible: string[];
  devices: DeviceInfo[];
  metas: Map<string, DeviceMeta>;
  disabled: boolean;
  onChange: (map: Record<string, string>) => void;
  onEdit: (bundle: PublishBundle) => void;
};
type Drag = {
  id: string;
  pointer: number;
  x: number;
  y: number;
  active: boolean;
  source: HTMLElement;
  ghost?: HTMLElement;
  target?: HTMLElement;
  frame?: number;
};
export function PublishAssignmentBoard(p: Props) {
  const [compact, setCompact] = useState(
    () =>
      typeof window.matchMedia === "function" &&
      window.matchMedia("(max-height:700px)").matches,
  );
  useEffect(() => {
    if (typeof window.matchMedia !== "function") return;
    const query = window.matchMedia("(max-height:700px)");
    const update = () => setCompact(query.matches);
    query.addEventListener("change", update);
    return () => query.removeEventListener("change", update);
  }, []);
  const [active, setActive] = useState(p.bundles[0]?.id),
    [query, setQuery] = useState(""),
    [deviceQuery, setDeviceQuery] = useState("");
  const [filter, setFilter] = useState("all"),
    [page, setPage] = useState(0);
  const [ref, size] = usePublishPageSize(compact ? 64 : 86),
    grid = useRef<HTMLDivElement>(null),
    stage = useRef<HTMLDivElement>(null),
    ghost = useRef<Drag | null>(null),
    suppress = useRef(0);
  const [preview, setPreview] = useState<{ id: string; udid: string } | null>(null);
  const [history, setHistory] = useState<{
    undo: Record<string, string>[];
    redo: Record<string, string>[];
  }>({ undo: [], redo: [] });
  const [swap, setSwap] = useState<{ id: string; udid: string } | null>(null),
    [quick, setQuick] = useState(false),
    [start, setStart] = useState(0),
    [replace, setReplace] = useState(false);
  const historyKey = JSON.stringify([p.bundles.map((b) => b.id), p.eligible]);
  useEffect(() => {
    setHistory({ undo: [], redo: [] });
  }, [historyKey]);
  const visibleBundles = p.bundles.filter(
    (b) =>
      `${b.name} ${b.caption}`.toLowerCase().includes(query.toLowerCase()) &&
      (filter === "all" ||
        (filter === "assigned" ? !!p.assignments[b.id] : !p.assignments[b.id])),
  );
  const current = Math.min(
      page,
      Math.max(0, Math.ceil(visibleBundles.length / size) - 1),
    ),
    rows = visibleBundles.slice(current * size, (current + 1) * size);
  const chosen = rows.find((b) => b.id === active) ?? rows[0];
  const available = orderDevicesByNumber(p.devices, p.metas).filter((d) =>
    p.eligible.includes(d.udid),
  );
  const name = (udid: string) => {
    const meta = p.metas.get(udid),
      i = p.devices.findIndex((d) => d.udid === udid);
    return `Máy ${meta?.number ?? i + 1}${meta?.alias ? ` · ${meta.alias}` : ""}`;
  };
  const machineRows = available.filter((d) =>
    name(d.udid).toLowerCase().includes(deviceQuery.toLowerCase()),
  );
  const owner = (udid: string) =>
    p.bundles.find((b) => p.assignments[b.id] === udid);
  const commit = (next: Record<string, string>) => {
    if (p.disabled || JSON.stringify(next) === JSON.stringify(p.assignments))
      return;
    setHistory((h) => ({
      undo: [...h.undo.slice(-49), p.assignments],
      redo: [],
    }));
    p.onChange(next);
  };
  const imageRect = (udid: string) =>
    Array.from(grid.current?.querySelectorAll<HTMLElement>("[data-slot]") ?? [])
      .find((el) => el.dataset.slot === udid)
      ?.querySelector("img")
      ?.getBoundingClientRect();
  const fly = (
    source: DOMRect | undefined,
    url: string | undefined,
    to: string,
  ) => {
    if (
      !source ||
      !url ||
      matchMedia("(prefers-reduced-motion: reduce)").matches
    )
      return;
    requestAnimationFrame(() => {
      const end = imageRect(to);
      if (!end) return;
      const img = document.createElement("img");
      img.src = url;
      img.alt = "";
      img.className = "pw-flight";
      Object.assign(img.style, {
        left: `${source.x}px`,
        top: `${source.y}px`,
        width: `${source.width}px`,
        height: `${source.height}px`,
      });
      stage.current?.append(img);
      const a = img.animate(
        [
          { transform: "translate(0,0)" },
          {
            transform: `translate(${end.x - source.x}px,${end.y - source.y}px) scale(${end.width / source.width},${end.height / source.height})`,
          },
        ],
        { duration: 240, easing: "cubic-bezier(.2,.8,.2,1)", fill: "forwards" },
      );
      void a.finished.catch(() => {}).finally(() => img.remove());
    });
  };
  const assign = (id: string, udid: string, source?: DOMRect, url?: string) => {
    if (!p.bundles.some((b) => b.id === id) || !p.eligible.includes(udid))
      return;
    const previous = p.assignments[id];
    const displacedImage = Array.from(
      grid.current?.querySelectorAll<HTMLElement>("[data-slot]") ?? [],
    )
      .find((el) => el.dataset.slot === udid)
      ?.querySelector("img");
    const displacedRect = displacedImage?.getBoundingClientRect();
    const displacedUrl = displacedImage?.src;
    commit(assignDevice(p.assignments, id, udid));
    setActive(id);
    setPreview(null);
    fly(source, url, udid);
    if (previous && previous !== udid)
      fly(displacedRect, displacedUrl, previous);
  };
  const cancelDrag = () => {
    const d = ghost.current;
    if (!d) return;
    ghost.current = null;
    if (d.frame) cancelAnimationFrame(d.frame);
    d.ghost?.remove();
    d.source.classList.remove("is-drag-source");
    stage.current
      ?.querySelectorAll(".is-drop-target")
      .forEach((el) => el.classList.remove("is-drop-target"));
    if (stage.current?.hasPointerCapture(d.pointer))
      stage.current.releasePointerCapture(d.pointer);
    setPreview(null);
  };
  useEffect(() => {
    const blur = () => cancelDrag();
    window.addEventListener("blur", blur);
    return () => {
      window.removeEventListener("blur", blur);
      cancelDrag();
    };
  }, []);
  useEffect(() => {
    if (p.disabled) cancelDrag();
  }, [p.disabled]);
  const onPointerDown = (e: React.PointerEvent) => {
    if (p.disabled || e.button !== 0) return;
    const source = (e.target as HTMLElement).closest<HTMLElement>(
      "[data-post-drag]",
    );
    if (!source) return;
    cancelDrag();
    ghost.current = {
      id: source.dataset.postDrag!,
      pointer: e.pointerId,
      x: e.clientX,
      y: e.clientY,
      active: false,
      source,
    };
  };
  const onPointerMove = (e: React.PointerEvent) => {
    const d = ghost.current;
    if (!d || d.pointer !== e.pointerId) return;
    if (!d.active && Math.hypot(e.clientX - d.x, e.clientY - d.y) < 7) return;
    e.preventDefault();
    if (!d.active) {
      d.active = true;
      stage.current?.setPointerCapture(e.pointerId);
      d.source.classList.add("is-drag-source");
      d.ghost = d.source.cloneNode(true) as HTMLElement;
      d.ghost.className = "pw-drag-ghost";
      d.ghost.setAttribute("aria-hidden", "true");
      d.ghost
        .querySelectorAll("[id]")
        .forEach((el) => el.removeAttribute("id"));
      document.body.append(d.ghost);
    }
    const hit = document.elementFromPoint(e.clientX, e.clientY);
    const target = hit?.closest<HTMLElement>("[data-slot]");
    if (target !== d.target) {
      d.target?.classList.remove("is-drop-target");
      d.target = target ?? undefined;
      target?.classList.add("is-drop-target");
      setPreview(target ? { id: d.id, udid: target.dataset.slot! } : null);
    }
    const x = Math.min(e.clientX + 12, innerWidth - 175),
      y = Math.max(4, Math.min(e.clientY - 40, innerHeight - 160));
    if (d.frame) cancelAnimationFrame(d.frame);
    d.frame = requestAnimationFrame(() => {
      if (d.ghost) d.ghost.style.transform = `translate3d(${x}px,${y}px,0)`;
    });
  };
  const onPointerUp = (e: React.PointerEvent) => {
    const d = ghost.current;
    if (!d) return;
    const target = d.active
      ? document
          .elementFromPoint(e.clientX, e.clientY)
          ?.closest<HTMLElement>("[data-slot]")
      : null;
    const rect = d.ghost?.querySelector("img")?.getBoundingClientRect(),
      url = d.ghost?.querySelector("img")?.src;
    if (d.active) {
      suppress.current = Date.now() + 300;
      e.preventDefault();
    }
    cancelDrag();
    if (target) assign(d.id, target.dataset.slot!, rect, url);
  };
  const previewPost = p.bundles.find((b) => b.id === preview?.id),
    displaced = preview ? owner(preview.udid) : undefined;
  const missing = p.bundles.filter((b) => !p.assignments[b.id]);
  const selectAllMap = fillAssignments(
    p.bundles.map(b => b.id), p.assignments,
    available.filter(device => device.status === "ready" || owner(device.udid)).map(device => device.udid),
  );
  const planned = fillAssignments(
    p.bundles.map((b) => b.id),
    replace ? {} : p.assignments,
    available.map((d) => d.udid),
    start,
  );
  return (
    <section
      className="pw-board"
      aria-label="Ghép bài với máy"
      ref={stage}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onPointerCancel={cancelDrag}
      onLostPointerCapture={(e) => {
        if (e.target === stage.current) cancelDrag();
      }}
      onDragStart={(e) => e.preventDefault()}
      onKeyDown={(e) => {
        if (e.key === "Escape" && ghost.current) {
          e.preventDefault();
          suppress.current = Date.now() + 300;
          cancelDrag();
        }
      }}
    >
      <header>
        <div>
          <h3>Mỗi máy đăng một bài</h3>
          <small>
            {p.bundles.length - missing.length} / {p.bundles.length} bài đã có
            máy
          </small>
        </div>
        <div className="pw-board-actions">
          <button
            type="button"
            className="ghost icon-only"
            aria-label="Hoàn tác ghép máy"
            disabled={p.disabled || !history.undo.length}
            onClick={() => {
              const value = history.undo.at(-1)!;
              setHistory((h) => ({
                undo: h.undo.slice(0, -1),
                redo: [...h.redo, p.assignments],
              }));
              p.onChange(value);
            }}
          >
            <Undo2 size={16} />
          </button>
          <button
            type="button"
            className="ghost icon-only"
            aria-label="Làm lại ghép máy"
            disabled={p.disabled || !history.redo.length}
            onClick={() => {
              const value = history.redo.at(-1)!;
              setHistory((h) => ({
                undo: [...h.undo, p.assignments],
                redo: h.redo.slice(0, -1),
              }));
              p.onChange(value);
            }}
          >
            <Redo2 size={16} />
          </button>
          <button
            type="button"
            disabled={
              p.disabled || !missing.length || missing.length > available.length
            }
            onClick={() => {
              const next = fillAssignments(
                p.bundles.map((b) => b.id),
                p.assignments,
                available.map((d) => d.udid),
              );
              if (next) commit(next);
            }}
          >
            Ghép tự động {missing.length || ""}
          </button>
          <button
            type="button"
            className="ghost"
            disabled={p.disabled}
            onClick={() => {
              setStart(0);
              setReplace(false);
              setQuick(true);
            }}
          >
            Chọn dải máy
          </button>
        </div>
      </header>
      <div className="pw-board-columns">
        <div className="pw-post-library">
          <div className="pw-tools">
            <h3>Bài đã chọn</h3>
            <select
              aria-label="Lọc trạng thái ghép"
              value={filter}
              onChange={(e) => {
                setFilter(e.target.value);
                setPage(0);
              }}
            >
              <option value="all">Tất cả</option>
              <option value="missing">Chưa ghép</option>
              <option value="assigned">Đã ghép</option>
            </select>
          </div>
          <label className="pw-search">
            <Search size={14} />
            <input
              aria-label="Tìm bài cần ghép"
              value={query}
              onChange={(e) => {
                setQuery(e.target.value);
                setPage(0);
              }}
              placeholder="Tìm bài…"
            />
          </label>
          <div className="pw-page-rows" ref={ref}>
            {rows.map((b) => (
              <article
                key={b.id}
                className={`pw-post ${chosen?.id === b.id ? "is-active" : ""}`}
              >
                <button
                  type="button"
                  disabled={p.disabled}
                  className="pw-post-pick"
                  data-post-drag={b.id}
                  aria-pressed={chosen?.id === b.id}
                  aria-label={`Chọn bài ${b.name}`}
                  onClick={(e) => {
                    if (!e.detail || Date.now() > suppress.current)
                      setActive(b.id);
                  }}
                >
                  <PublishMedia bundle={b} />
                  <span>
                    <strong>{b.name}</strong>
                    <small>
                      {b.mediaKind === "video"
                        ? "Video"
                        : `${b.images.length} ảnh`}
                    </small>
                    <em>
                      {p.assignments[b.id]
                        ? name(p.assignments[b.id])
                        : "Chưa ghép"}
                    </em>
                  </span>
                </button>
                <button
                  type="button"
                  className="ghost icon-only"
                  disabled={p.disabled}
                  aria-label={`Sửa nội dung ${b.name}`}
                  onClick={() => p.onEdit(b)}
                >
                  <Pencil size={14} />
                </button>
              </article>
            ))}
          </div>
          <PublishPager
            label="Bài ghép"
            page={current}
            size={size}
            total={visibleBundles.length}
            onPage={setPage}
          />
        </div>
        <div className="pw-fleet">
          <header>
            <h3>Máy nhận bài · {available.length}</h3>
            <label className="pw-search">
              <Search size={14} />
              <input
                aria-label="Tìm số máy"
                placeholder="Tìm máy…"
                value={deviceQuery}
                onChange={(e) => {
                  setDeviceQuery(e.target.value);
                }}
              />
            </label>
          </header>
          <div className="pw-machine-tools">
            <button type="button" className="ghost" title="Ghép các bài chưa có máy với máy sẵn sàng còn trống, mỗi máy một bài" disabled={p.disabled || !missing.length || !selectAllMap} onClick={() => {
              if (selectAllMap) commit(selectAllMap);
            }}>Chọn tất cả</button>
            <button type="button" className="ghost" disabled={p.disabled || missing.length === p.bundles.length} onClick={() => commit({})}>Bỏ chọn</button>
            {p.scopeControl}
            <span>{p.bundles.length - missing.length} / {available.length} máy · {machineRows.length} hiển thị</span>
          </div>
          <div
            ref={grid}
            className={`pw-machine-grid machine-choice-grid${available.length > 12 ? " is-compact" : ""}`}
            role="group"
            aria-label="Danh sách máy nhận bài"
          >
            {machineRows.map((d) => {
                const post = owner(d.udid);
                return (
                  <article
                    className={`pw-slot ${post ? "is-filled" : ""}`}
                    key={d.udid}
                    data-slot={d.udid}
                  >
                    <MachineChoice number={tileNumber(p.devices.findIndex(device => device.udid === d.udid) + 1, p.metas.get(d.udid))}
                      name={p.metas.get(d.udid)?.alias || d.name} status={d.status} reason={d.lastError} label={`Chọn ${name(d.udid)}`} checked={Boolean(post)}
                      disabled={p.disabled || (!post && (d.status !== "ready" || !missing.length))}
                      onChange={(checked) => {
                        if (!checked && post) {
                          const next = { ...p.assignments };
                          delete next[post.id];
                          commit(next);
                        } else if (checked) {
                          const nextPost = chosen && !p.assignments[chosen.id] ? chosen : missing[0];
                          if (nextPost) assign(nextPost.id, d.udid);
                        }
                      }} detail={
                    <button
                      type="button"
                      className="pw-slot-body"
                      disabled={p.disabled || !chosen}
                      data-post-drag={post?.id}
                      aria-label={`${name(d.udid)}: ${post?.name ?? "chưa có bài"}`}
                      onClick={(e) => {
                        if (e.detail && Date.now() < suppress.current) return;
                        if (!chosen || p.assignments[chosen.id] === d.udid)
                          return;
                        if (post) setSwap({ id: chosen.id, udid: d.udid });
                        else assign(chosen.id, d.udid);
                      }}
                    >
                      {post ? (
                        <>
                          <PublishMedia bundle={post} />
                          <span>
                            <strong>{post.name}</strong>
                            <small>
                              {post.mediaKind === "video"
                                ? "Video"
                                : `${post.images.length} ảnh`}
                            </small>
                          </span>
                        </>
                      ) : (
                        <>
                          <Plus size={20} />
                          <small>Bấm để gán bài</small>
                        </>
                      )}
                    </button>
                    } />
                    {preview?.udid === d.udid &&
                      previewPost &&
                      previewPost.id !== post?.id && (
                        <div className="pw-slot-preview" aria-hidden="true">
                          <PublishMedia bundle={previewPost} />
                          <strong>{post ? "Đổi bài" : "Nhận bài"}</strong>
                        </div>
                      )}
                  </article>
                );
              })}
            {!machineRows.length && (
              <div className="pw-empty">
                Chưa có máy trong phạm vi. Chọn nhóm máy hoặc kiểm tra kết nối.
              </div>
            )}
          </div>
        </div>
      </div>
      <div className="pw-drop-summary" role="status">
        {previewPost && preview ? (
          <>
            <PublishMedia bundle={previewPost} />
            <span>
              {previewPost.name}
              <strong>→ {name(preview.udid)}</strong>
            </span>
            {displaced && displaced.id !== preview.id && (
              <>
                <PublishMedia bundle={displaced} />
                <span>
                  {displaced.name}
                  <strong>
                    →{" "}
                    {p.assignments[preview.id]
                      ? name(p.assignments[preview.id])
                      : "Trả về bài chưa ghép"}
                  </strong>
                </span>
              </>
            )}
          </>
        ) : (
          <>
            <span>
              <small>Bài đang chọn</small>
              <strong>{chosen?.name ?? "Chọn bài bên trái"}</strong>
            </span>
            <small>
              Ghép tự động hoặc bấm ô máy. Kéo ảnh để đổi riêng từng bài.
            </small>
          </>
        )}
      </div>
      {swap && (
        <PublishDialog
          title="Xác nhận đổi bài"
          onClose={() => setSwap(null)}
          actions={
            <>
              <button type="button" onClick={() => setSwap(null)}>
                Hủy
              </button>
              <button
                type="button"
                className="primary"
                disabled={p.disabled}
                onClick={() => {
                  assign(swap.id, swap.udid);
                  setSwap(null);
                }}
              >
                Đổi bài
              </button>
            </>
          }
        >
          <p>
            {p.bundles.find((b) => b.id === swap.id)?.name} → {name(swap.udid)}
          </p>
          <p>
            {owner(swap.udid)?.name} →{" "}
            {p.assignments[swap.id]
              ? name(p.assignments[swap.id])
              : "bài chưa ghép"}
          </p>
        </PublishDialog>
      )}
      {quick && (
        <PublishDialog
          title="Ghép theo dải máy"
          onClose={() => setQuick(false)}
          actions={
            <>
              <button type="button" onClick={() => setQuick(false)}>
                Hủy
              </button>
              <button
                type="button"
                className="primary"
                disabled={!planned || p.disabled}
                onClick={() => {
                  if (planned) commit(planned);
                  setQuick(false);
                }}
              >
                <Check size={15} /> Áp dụng
              </button>
            </>
          }
        >
          <label className="pw-field">
            Máy bắt đầu
            <select
              value={start}
              onChange={(e) => setStart(Number(e.target.value))}
            >
              {available.map((d, i) => (
                <option value={i} key={d.udid}>
                  {name(d.udid)}
                </option>
              ))}
            </select>
          </label>
          <label className="pw-field">
            Phạm vi
            <select
              value={String(replace)}
              onChange={(e) => setReplace(e.target.value === "true")}
            >
              <option value="false">Chỉ bài chưa có máy</option>
              <option value="true">Ghép lại toàn bộ</option>
            </select>
          </label>
          {planned ? (
            <div className="pw-quick-list">
              {p.bundles.map((b) => (
                <p key={b.id}>
                  <span>{b.name}</span>
                  <strong>{name(planned[b.id])}</strong>
                </p>
              ))}
            </div>
          ) : (
            <p role="alert">Dải này không đủ máy cho {p.bundles.length} bài.</p>
          )}
        </PublishDialog>
      )}
    </section>
  );
}
