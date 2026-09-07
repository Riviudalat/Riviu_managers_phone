import { useEffect, useRef, useState, type ReactNode } from "react";
import {
  ArrowLeft,
  ArrowRight,
  Check,
  FolderOpen,
  History,
  Image as ImageIcon,
  Music2,
  Pencil,
  Search,
  Settings,
} from "lucide-react";
import type {
  DeviceInfo,
  DeviceMeta,
  PublishBundle,
  PublishFolderManifest,
  PublishPreflightReport,
  PublishSoundPolicy,
} from "../../types";
import { pickDirectory } from "../../pickFile";
import { describeError } from "../../describeError";
import { PublishDialog } from "./PublishDialog";
import { PublishMedia } from "./PublishMedia";
import { PublishPager } from "./PublishPager";
import { usePublishPageSize } from "./usePublishPageSize";
import { PublishAssignmentBoard } from "./PublishAssignmentBoard";

export interface PublishWizardProps {
  active?: boolean;
  sourceRoot: string;
  manifest: PublishFolderManifest | null;
  selectedIds: string[];
  assignments: Record<string, string>;
  captions: Record<string, string>;
  devices: DeviceInfo[];
  metas: Map<string, DeviceMeta>;
  eligible: string[];
  busy: boolean;
  scanning: boolean;
  preflightLoading: boolean;
  preflight: PublishPreflightReport | null;
  preflightError: string | null;
  sound: PublishSoundPolicy;
  sheet: boolean;
  cleanup: boolean;
  runAt: string;
  onSource: (path: string) => void;
  onScan: (path: string) => Promise<void>;
  onSelect: (ids: string[]) => void;
  onAssign: (value: Record<string, string>) => void;
  onCaption: (id: string, value: string) => void;
  onSheet: (value: boolean) => void;
  onCleanup: (value: boolean) => void;
  onRunAt: (value: string) => void;
  onPreflight: () => Promise<void>;
  onExecute: () => Promise<void>;
  onHistory: () => void;
  settings: ReactNode;
  notices?: ReactNode;
}

export function PublishWizard(p: PublishWizardProps) {
  const [checkPage, setCheckPage] = useState(0);
  const [step, setStep] = useState(1),
    [query, setQuery] = useState(""),
    [onlySelected, setOnlySelected] = useState(false);
  const [page, setPage] = useState(0),
    [activeId, setActiveId] = useState<string>(),
    [photo, setPhoto] = useState(0);
  const [dialog, setDialog] = useState<
    "count" | "caption" | "settings" | "check" | "music" | null
  >(null);
  const [count, setCount] = useState(1),
    [text, setText] = useState(""),
    [discard, setDiscard] = useState(false);
  const [sourceError, setSourceError] = useState<string>();
  useEffect(() => {
    if (p.active === false) setDialog(null);
  }, [p.active]);
  const [listRef, size] = usePublishPageSize(72);
  const live = useRef(true);
  useEffect(() => {
    live.current = true;
    return () => {
      live.current = false;
    };
  }, []);
  const bundles = p.manifest?.bundles ?? [],
    selected = bundles.filter((b) => p.selectedIds.includes(b.id));
  const filtered = bundles.filter(
    (b) =>
      (!onlySelected || p.selectedIds.includes(b.id)) &&
      `${b.name} ${p.captions[b.id] ?? b.caption}`
        .toLocaleLowerCase()
        .includes(query.toLocaleLowerCase()),
  );
  const currentPage = Math.min(
    page,
    Math.max(0, Math.ceil(filtered.length / size) - 1),
  );
  const visible = filtered.slice(currentPage * size, (currentPage + 1) * size);
  const active = bundles.find((b) => b.id === activeId) ?? bundles[0];
  const mapped = selected.filter(
    (b) => p.assignments[b.id] && p.eligible.includes(p.assignments[b.id]),
  ).length;
  const complete =
    selected.length > 0 &&
    mapped === selected.length &&
    new Set(selected.map((b) => p.assignments[b.id])).size === selected.length;
  const captionsValid = selected.every((b) =>
    Boolean((p.captions[b.id] ?? b.caption).trim()),
  );
  const locked = p.busy || p.scanning || p.preflightLoading;
  const changeStep = (n: number) => {
    if (locked) return;
    setStep(n);
  };
  const choose = async () => {
    setSourceError(undefined);
    try {
      const path = await pickDirectory();
      if (path && live.current) await p.onScan(path);
    } catch (e) {
      if (live.current) setSourceError(describeError(e));
    }
  };
  const edit = (b: PublishBundle) => {
    setActiveId(b.id);
    setText(p.captions[b.id] ?? b.caption);
    setDiscard(false);
    setDialog("caption");
  };
  const closeDialog = () => {
    if (
      dialog === "caption" &&
      active &&
      text !== (p.captions[active.id] ?? active.caption) &&
      !discard
    ) {
      setDiscard(true);
      return;
    }
    setDialog(null);
    setDiscard(false);
  };
  const selectedTotal = p.selectedIds.length;
  const footer =
    step === 1
      ? "Mỗi thư mục là một bài, gồm toàn bộ ảnh"
      : step === 2
        ? `${mapped} / ${selected.length} máy đã ghép · Chưa đăng`
        : "Chọn nhạc sau khi mở TikTok, trước thao tác Đăng";
  const toggle = (id: string) =>
    p.onSelect(
      p.selectedIds.includes(id)
        ? p.selectedIds.filter((b) => b !== id)
        : [...p.selectedIds, id],
    );

  return (
    <div className="publish-wizard" hidden={p.active === false}>
      <header className="pw-header">
        <strong>
          {selected.length
            ? `${selected.length} bài đang thiết lập`
            : "Chiến dịch mới"}
        </strong>
        <div>
          <button
            type="button"
            className="ghost"
            onClick={() => setDialog("settings")}
          >
            <Settings size={16} /> Hồ sơ & cài đặt
          </button>
          <button type="button" className="ghost" onClick={p.onHistory}>
            <History size={16} /> Theo dõi
          </button>
        </div>
      </header>
      <nav className="pw-steps" aria-label="Quy trình đăng bài">
        {["Chọn bài", "Chọn máy", "Kiểm tra & đăng"].map((label, i) => (
          <button
            type="button"
            key={label}
            aria-current={step === i + 1 ? "step" : undefined}
            disabled={
              locked || (i === 1 && !selected.length) || (i === 2 && !complete)
            }
            onClick={() => changeStep(i + 1)}
          >
            <span>{step > i + 1 ? <Check size={15} /> : i + 1}</span>
            <strong>{label}</strong>
          </button>
        ))}
      </nav>
      {p.notices && <div className="pw-notices">{p.notices}</div>}
      <div className="pw-stage">
        {step === 1 && (
          <section className="pw-content" aria-label="Chọn bài đăng">
            <div className="pw-source-list">
              <div className="pw-source-bar">
                <label>
                  <span className="visually-hidden">Thư mục nguồn</span>
                  <input
                    aria-label="Thư mục nguồn"
                    value={p.sourceRoot}
                    placeholder="Chọn thư mục nội dung"
                    onChange={(e) => p.onSource(e.target.value)}
                    disabled={p.busy}
                  />
                </label>
                <button
                  type="button"
                  disabled={locked}
                  onClick={() => void choose()}
                >
                  <FolderOpen size={16} /> Chọn thư mục
                </button>
                <button
                  type="button"
                  className="ghost"
                  disabled={locked || !p.sourceRoot.trim()}
                  onClick={() => void p.onScan(p.sourceRoot)}
                >
                  Quét nguồn
                </button>
              </div>
              {sourceError && <p role="alert">{sourceError}</p>}
              <div className="pw-tools">
                <label className="pw-search">
                  <Search size={15} />
                  <input
                    aria-label="Tìm bài đăng"
                    placeholder="Tìm bài đăng…"
                    value={query}
                    onChange={(e) => {
                      setQuery(e.target.value);
                      setPage(0);
                    }}
                  />
                </label>
                <button
                  type="button"
                  disabled={!filtered.length || locked}
                  onClick={() => {
                    setCount(
                      Math.min(10, filtered.length, p.eligible.length) || 1,
                    );
                    setDialog("count");
                  }}
                >
                  Chọn nhanh
                </button>
                <select
                  aria-label="Lọc bài"
                  value={String(onlySelected)}
                  onChange={(e) => {
                    setOnlySelected(e.target.value === "true");
                    setPage(0);
                  }}
                >
                  <option value="false">Tất cả</option>
                  <option value="true">Đã chọn</option>
                </select>
              </div>
              <div className="pw-table-head">
                <input
                  type="checkbox"
                  aria-label="Chọn trang hiện tại"
                  disabled={!visible.length || locked}
                  checked={
                    visible.length > 0 &&
                    visible.every((b) => p.selectedIds.includes(b.id))
                  }
                  onChange={(e) =>
                    p.onSelect(
                      e.target.checked
                        ? [
                            ...new Set([
                              ...p.selectedIds,
                              ...visible.map((b) => b.id),
                            ]),
                          ]
                        : p.selectedIds.filter(
                            (id) => !visible.some((b) => b.id === id),
                          ),
                    )
                  }
                />
                <span>{bundles.length} bài trong thư mục</span>
                <button
                  type="button"
                  className="ghost"
                  disabled={locked || !selectedTotal}
                  onClick={() => p.onSelect([])}
                >
                  Bỏ chọn
                </button>
              </div>
              <div
                ref={listRef}
                className="pw-page-rows"
                aria-label="Gói nội dung"
              >
                {p.scanning ? (
                  <div className="pw-empty" role="status">
                    Đang quét nội dung…
                  </div>
                ) : !filtered.length ? (
                  <div className="pw-empty">
                    <FolderOpen size={28} />
                    {p.manifest
                      ? "Không có bài phù hợp"
                      : "Chọn thư mục để xem các bài đăng"}
                  </div>
                ) : (
                  visible.map((b) => (
                    <article
                      className={`pw-content-row ${active?.id === b.id ? "is-active" : ""}`}
                      key={b.id}
                    >
                      <input
                        type="checkbox"
                        aria-label={`Chọn ${b.name}`}
                        checked={p.selectedIds.includes(b.id)}
                        disabled={locked}
                        onChange={() => toggle(b.id)}
                      />
                      <button
                        type="button"
                        className="pw-bundle"
                        onClick={() => {
                          setActiveId(b.id);
                          setPhoto(0);
                        }}
                      >
                        <PublishMedia bundle={b} />
                        <span>
                          <strong>{b.name}</strong>
                          <small>
                            {b.mediaKind === "video"
                              ? "Video MP4"
                              : `${b.images.length} ảnh`}{" "}
                            · {(b.totalBytes / 1048576).toFixed(1)} MB
                          </small>
                        </span>
                      </button>
                      <button
                        className="ghost icon-only"
                        aria-label={`Sửa nội dung ${b.name}`}
                        type="button"
                        onClick={() => edit(b)}
                        disabled={locked}
                      >
                        <Pencil size={15} />
                      </button>
                    </article>
                  ))
                )}
              </div>
              <PublishPager
                label="Bài đăng"
                page={currentPage}
                size={size}
                total={filtered.length}
                onPage={setPage}
              />
            </div>
            <aside className="pw-preview">
              <div>
                <h3>Xem trước bài đăng</h3>
                <small>
                  {active
                    ? active.mediaKind === "video"
                      ? "Video MP4"
                      : `${active.images.length} ảnh`
                    : ""}
                </small>
              </div>
              <div className="pw-preview-image">
                {active ? (
                  <PublishMedia
                    bundle={active}
                    index={Math.min(
                      photo,
                      Math.max(0, active.images.length - 1),
                    )}
                    expanded
                  />
                ) : (
                  <ImageIcon size={38} />
                )}
              </div>
              <div className="pw-photo-controls">
                <button
                  type="button"
                  className="ghost"
                  aria-label="Ảnh trước"
                  disabled={photo === 0}
                  onClick={() => setPhoto((n) => n - 1)}
                >
                  ‹
                </button>
                <span>
                  {active?.mediaKind === "video"
                    ? "Video"
                    : active?.images.length
                      ? `Ảnh ${Math.min(photo + 1, active.images.length)} / ${active.images.length}`
                      : "Chưa có ảnh"}
                </span>
                <button
                  type="button"
                  className="ghost"
                  aria-label="Ảnh tiếp"
                  disabled={!active || photo + 1 >= active.images.length}
                  onClick={() => setPhoto((n) => n + 1)}
                >
                  ›
                </button>
              </div>
              <p>{active && (p.captions[active.id] ?? active.caption)}</p>
              <button
                type="button"
                className="ghost"
                disabled={!active || locked}
                onClick={() => active && edit(active)}
              >
                <Pencil size={14} /> Sửa nội dung chữ
              </button>
            </aside>
          </section>
        )}
        {step === 2 && (
          <PublishAssignmentBoard
            bundles={selected}
            assignments={p.assignments}
            eligible={p.eligible}
            devices={p.devices}
            metas={p.metas}
            disabled={locked}
            onChange={p.onAssign}
            onEdit={edit}
          />
        )}
        {step === 3 && (
          <section className="pw-review" aria-label="Kiểm tra trước khi đăng">
            <div className="pw-options">
              <h3>Thiết lập đăng</h3>
              <fieldset>
                <legend>
                  <Music2 size={15} /> Nhạc khi mở TikTok
                </legend>
                <p>Tự chọn từ nhạc đề xuất trên tài khoản.</p>
                <button
                  type="button"
                  className="ghost pw-detail-link"
                  onClick={() => setDialog("music")}
                >
                  Có thể trùng giữa các máy
                </button>
              </fieldset>
              <fieldset>
                <legend>Thời điểm đăng</legend>
                <select
                  aria-label="Thời điểm đăng"
                  value={p.runAt ? "later" : "now"}
                  disabled={locked}
                  onChange={(e) =>
                    p.onRunAt(
                      e.target.value === "now"
                        ? ""
                        : new Date(
                            Date.now() +
                              3600000 -
                              new Date().getTimezoneOffset() * 60000,
                          )
                            .toISOString()
                            .slice(0, 16),
                    )
                  }
                >
                  <option value="now">Đăng ngay</option>
                  <option value="later">Hẹn giờ</option>
                </select>
                {p.runAt && (
                  <input
                    type="datetime-local"
                    aria-label="Ngày giờ đăng"
                    value={p.runAt}
                    disabled={locked}
                    onChange={(e) => p.onRunAt(e.target.value)}
                  />
                )}
              </fieldset>
              <fieldset>
                <legend>Báo cáo</legend>
                <label>
                  <input
                    type="checkbox"
                    checked={p.sheet}
                    disabled={locked}
                    onChange={(e) => p.onSheet(e.target.checked)}
                  />{" "}
                  Ghi kết quả lên Sheet
                </label>
                <button
                  type="button"
                  className="ghost pw-detail-link"
                  onClick={() => setDialog("settings")}
                >
                  Cấu hình Sheet
                </button>
              </fieldset>
              <fieldset>
                <legend>Sau khi đăng</legend>
                <label>
                  <input
                    type="checkbox"
                    checked={p.cleanup}
                    disabled={locked}
                    onChange={(e) => p.onCleanup(e.target.checked)}
                  />{" "}
                  Xóa ảnh đã chuyển trên máy
                </label>
                <small>
                  Chỉ xóa bản chuyển sau khi xác nhận đăng thành công.
                </small>
              </fieldset>
            </div>
            <ReviewList
              bundles={selected}
              assignments={p.assignments}
              devices={p.devices}
              metas={p.metas}
            />
          </section>
        )}
      </div>
      <footer className="pw-footer">
        <div>
          <strong>
            {selectedTotal
              ? `${selectedTotal} bài được chọn`
              : "Chọn bài muốn đăng"}
          </strong>
          <small>{footer}</small>
        </div>
        <div>
          {step > 1 && (
            <button
              type="button"
              className="ghost"
              disabled={locked}
              onClick={() => changeStep(step - 1)}
            >
              <ArrowLeft size={15} /> Quay lại
            </button>
          )}
          <button
            type="button"
            className="primary"
            disabled={
              locked ||
              !selected.length ||
              !captionsValid ||
              (step > 1 && !complete)
            }
            onClick={() => {
              if (step < 3) changeStep(step + 1);
              else {
                setDialog("check");
                void p.onPreflight();
              }
            }}
          >
            {step === 1
              ? "Chọn máy"
              : step === 2
                ? "Xem lại & kiểm tra"
                : p.preflightLoading
                  ? "Đang kiểm tra…"
                  : `Kiểm tra ${selected.length} bài`}
            <ArrowRight size={15} />
          </button>
        </div>
      </footer>
      {dialog === "count" && (
        <PublishDialog
          title="Chọn số bài muốn đăng"
          onClose={closeDialog}
          actions={
            <>
              <button type="button" onClick={closeDialog}>
                Hủy
              </button>
              <button
                type="button"
                className="primary"
                disabled={
                  !Number.isInteger(count) ||
                  count < 1 ||
                  count > filtered.length ||
                  locked
                }
                onClick={() => {
                  p.onSelect(filtered.slice(0, count).map((b) => b.id));
                  setDialog(null);
                }}
              >
                Chọn {count} bài
              </button>
            </>
          }
        >
          <label className="pw-count">
            Số bài
            <input
              type="number"
              min={1}
              max={filtered.length}
              value={count}
              onChange={(e) => setCount(Number(e.target.value))}
            />
          </label>
          <p>
            Chọn {count} bài đầu trong {filtered.length} kết quả. Mỗi thư mục
            gồm toàn bộ ảnh.
          </p>
        </PublishDialog>
      )}
      {dialog === "caption" && active && (
        <PublishDialog
          title={`Nội dung · ${active.name}`}
          onClose={closeDialog}
          actions={
            discard ? (
              <>
                <span>Chưa lưu thay đổi</span>
                <button
                  type="button"
                  onClick={() => {
                    setDialog(null);
                    setDiscard(false);
                  }}
                >
                  Bỏ thay đổi
                </button>
                <button
                  type="button"
                  className="primary"
                  onClick={() => setDiscard(false)}
                >
                  Tiếp tục sửa
                </button>
              </>
            ) : (
              <>
                <button type="button" onClick={closeDialog}>
                  Hủy
                </button>
                <button
                  type="button"
                  className="primary"
                  disabled={!text.trim() || [...text].length > 2200 || locked}
                  onClick={() => {
                    p.onCaption(active.id, text.trim());
                    setDialog(null);
                  }}
                >
                  Lưu nội dung
                </button>
              </>
            )
          }
        >
          <label className="pw-caption">
            Nội dung bài đăng
            <textarea
              aria-label={`Chú thích cho ${active.name}`}
              value={text}
              onChange={(e) => setText(e.target.value)}
            />
          </label>
          <small>
            {[...text].length} / 2200 ký tự · File gốc được giữ nguyên
          </small>
        </PublishDialog>
      )}
      <PublishDialog
        title="Hồ sơ & cài đặt Đăng bài"
        onClose={closeDialog}
        wide
        isOpen={dialog === "settings"}
      >
        {p.settings}
      </PublishDialog>
      {dialog === "music" && (
        <PublishDialog title="Nhạc khi chạy" onClose={closeDialog}>
          <p>
            Ứng dụng mở TikTok, chọn ảnh, chọn nhạc từ thư viện của tài khoản
            rồi đọc lại tên nhạc trước Đăng.
          </p>
          <p>
            Phiên bản này cho phép trùng nhạc giữa các máy. Chế độ không trùng
            chưa được hỗ trợ.
          </p>
        </PublishDialog>
      )}
      {dialog === "check" && (
        <PublishDialog
          title="Xem lại trước khi bắt đầu"
          onClose={closeDialog}
          wide
          actions={
            <>
              <button type="button" disabled={p.busy} onClick={closeDialog}>
                Sửa thiết lập
              </button>
              <button
                type="button"
                className="primary"
                disabled={!p.preflight?.canExecute || locked}
                onClick={() => void p.onExecute()}
              >
                {p.runAt
                  ? "Xác nhận lịch đăng"
                  : `Xác nhận đăng ${selected.length} bài`}
              </button>
            </>
          }
        >
          <p>
            {selected.length} bài sẽ đăng công khai trên {mapped} máy. Nhạc được
            chọn sau khi mở TikTok.
          </p>
          {p.preflightLoading ? (
            <p role="status">
              Đang kiểm tra nội dung, kết nối và phiên bản TikTok…
            </p>
          ) : p.preflightError ? (
            <div role="alert">
              {p.preflightError}
              <button type="button" onClick={() => void p.onPreflight()}>
                Thử lại
              </button>
            </div>
          ) : p.preflight ? (
            <>
              <p className={p.preflight.canExecute ? "pw-success" : "pw-error"}>
                {p.preflight.canExecute
                  ? "Đầu vào đã đạt kiểm tra. Chưa đăng bài."
                  : "Có điều kiện chưa đạt. Sửa máy hoặc nội dung trước khi chạy."}
              </p>
              <div
                className="pw-check-results"
                tabIndex={0}
                role="region"
                aria-label="Kết quả kiểm tra từng máy"
              >
                {p.preflight.assignments
                  .slice(checkPage * 3, (checkPage + 1) * 3)
                  .map((row) => (
                    <div key={row.udid}>
                      <strong>
                        {machineName(row.udid, p.devices, p.metas)}
                      </strong>
                      <span>
                        {row.issues.length
                          ? row.issues.map((i) => i.message).join(" · ")
                          : "Nội dung, dung lượng và luồng đăng được hỗ trợ"}
                      </span>
                    </div>
                  ))}
                {p.preflight.issues
                  .filter((i) => !i.udid)
                  .map((i, n) => (
                    <p key={n}>{i.message}</p>
                  ))}
              </div>
              <PublishPager
                label="Kết quả kiểm tra"
                page={checkPage}
                size={3}
                total={p.preflight.assignments.length}
                onPage={setCheckPage}
              />
            </>
          ) : (
            <p>
              Thiết lập đã đổi.{" "}
              <button type="button" onClick={() => void p.onPreflight()}>
                Kiểm tra lại
              </button>
            </p>
          )}
          <p className="pw-run-order">
            Mở TikTok → Chọn nội dung → Chọn nhạc → Đăng → Lấy liên kết
            {p.sheet ? " → Sheet" : ""}
          </p>
          <p>
            {p.cleanup
              ? "Xóa bản chuyển sau khi xác nhận đăng thành công."
              : "Giữ nội dung đã chuyển trên điện thoại."}
          </p>
        </PublishDialog>
      )}
    </div>
  );
}

function machineName(
  udid: string,
  devices: DeviceInfo[],
  metas: Map<string, DeviceMeta>,
) {
  const i = devices.findIndex((d) => d.udid === udid),
    meta = metas.get(udid);
  return `Máy ${meta?.number ?? i + 1}${meta?.alias ? ` · ${meta.alias}` : ""}`;
}
function ReviewList({
  bundles,
  assignments,
  devices,
  metas,
}: Pick<PublishWizardProps, "assignments" | "devices" | "metas"> & {
  bundles: PublishBundle[];
}) {
  const [ref, size] = usePublishPageSize(60),
    [page, setPage] = useState(0),
    current = Math.min(page, Math.max(0, Math.ceil(bundles.length / size) - 1));
  return (
    <div className="pw-review-list">
      <h3>
        {bundles.length} bài /{" "}
        {Object.values(assignments).filter(Boolean).length} máy
      </h3>
      <div className="pw-review-head">
        <span>Bài sẽ đăng công khai</span>
        <span>Máy nhận</span>
      </div>
      <div className="pw-page-rows" ref={ref}>
        {bundles.slice(current * size, (current + 1) * size).map((b) => (
          <div key={b.id} className="pw-review-row">
            <PublishMedia bundle={b} />
            <span>
              <strong>{b.name}</strong>
              <small>
                {b.mediaKind === "video" ? "Video" : `${b.images.length} ảnh`}
              </small>
            </span>
            <strong>{machineName(assignments[b.id], devices, metas)}</strong>
          </div>
        ))}
      </div>
      <PublishPager
        label="Xem lại"
        page={current}
        size={size}
        total={bundles.length}
        onPage={setPage}
      />
      <p className="pw-muted">
        Nhạc cụ thể được chọn trong TikTok khi lượt chạy bắt đầu.
      </p>
    </div>
  );
}
