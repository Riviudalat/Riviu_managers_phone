import { useState } from "react";
import type {
  PublishExecutionIssue,
  PublishPreflightReport,
} from "../../types";
import { PublishPager } from "./PublishPager";
import {
  publishPreflightProblem,
  publishPreflightRowPassed,
  publishTikTokBuildLabel,
} from "./publishPreflightDisplay";

export function PublishPreflightResult({
  network = "tiktok",
  report,
  machineName,
  bundleName,
  page,
  onPage,
  onRetry,
  busy,
  hideRetry = false,
}: {
  network?: "tiktok" | "threads";
  report: PublishPreflightReport;
  machineName: (udid: string) => string;
  bundleName?: (id: string) => string;
  page: number;
  onPage: (page: number) => void;
  onRetry: () => void;
  busy: boolean;
  hideRetry?: boolean;
}) {
  const [view, setView] = useState<"blocked" | "passed" | "all">("blocked");
  const blocked = report.assignments.filter((row) => !publishPreflightRowPassed(row));
  const passed = report.assignments.filter(publishPreflightRowPassed);
  const globalIssues = report.issues.filter((issue) => !issue.udid);
  const currentView = view === "blocked" && !blocked.length ? "all" : view;
  const visibleRows = currentView === "blocked" ? blocked : currentView === "passed" ? passed : [...blocked, ...passed];
  const pageSize = 8;
  const currentPage = Math.min(
    page,
    Math.max(0, Math.ceil(visibleRows.length / pageSize) - 1),
  );
  const changeView = (next: "blocked" | "passed" | "all") => {
    setView(next);
    onPage(0);
  };
  return (
    <>
      <p
        className={report.canExecute ? "pw-success" : "pw-error"}
        role="status"
      >
        {report.canExecute
          ? "Đầu vào đã đạt kiểm tra. Chưa đăng bài."
          : "Chưa thể đăng. Xử lý điều kiện chưa đạt bên dưới rồi kiểm tra lại."}
      </p>
      <div className="pw-preflight-overview" role="group" aria-label="Lọc kết quả kiểm tra">
        <span>{globalIssues.length ? `${globalIssues.length} điều kiện chung · ` : ""}{blocked.length} máy cần xử lý · {passed.length} máy đạt</span>
        <div className="pw-preflight-views">
          <button type="button" aria-pressed={currentView === "blocked"} onClick={() => changeView("blocked")}>Cần xử lý ({blocked.length})</button>
          <button type="button" aria-pressed={currentView === "passed"} onClick={() => changeView("passed")}>Đạt ({passed.length})</button>
          <button type="button" aria-pressed={currentView === "all"} onClick={() => changeView("all")}>Tất cả ({report.assignments.length})</button>
        </div>
      </div>
      {globalIssues.length > 0 && (
        <div className="pw-preflight-global" role="region" aria-label="Điều kiện chung chưa đạt">
          <strong>Điều kiện chung</strong>
          {globalIssues.map((issue, index) => (
            <div key={`${issue.code}:${index}`}>
              <Problem issue={issue} />
              <details className="pw-preflight-technical">
                <summary>Chi tiết kỹ thuật</summary>
                <p><code>{issue.code}</code>: {issue.message}</p>
              </details>
            </div>
          ))}
        </div>
      )}
      <div
        className="pw-check-results"
        tabIndex={0}
        role="region"
        aria-label="Kết quả kiểm tra từng máy"
      >
        {visibleRows
          .slice(currentPage * pageSize, (currentPage + 1) * pageSize)
          .map((row) => (
            <article
              className="pw-preflight-device"
              key={row.udid}
              aria-label={machineName(row.udid)}
            >
              <header>
                <strong>{machineName(row.udid)}</strong>
                <span
                  className={
                    publishPreflightRowPassed(row) ? "pw-success" : "pw-error"
                  }
                >
                  {publishPreflightRowPassed(row)
                    ? "Đạt kiểm tra"
                    : "Cần xử lý"}
                </span>
              </header>
              <p>{bundleName?.(row.bundleId) ?? `Bài ${row.ordinal + 1}`}</p>
              <p className="pw-preflight-build">
                {network === "threads" ? `Threads · ${row.version || "Chưa đọc được phiên bản"}` : publishTikTokBuildLabel(row)}
              </p>
              {Boolean(row.checks?.length) && (
                <dl className="pw-preflight-checks">
                  {row.checks!.map((check) => (
                    <div key={check.id}>
                      <dt>{check.label}</dt>
                      <dd
                        className={
                          check.status === "blocked" ? "pw-error" : undefined
                        }
                      >
                        {
                          {
                            pass: "Đạt",
                            blocked: "Bị chặn",
                            unknown: "Chưa quan sát",
                            notApplicable: "Không áp dụng",
                          }[check.status]
                        }
                        {check.reason && <span> · {check.reason}</span>}
                      </dd>
                    </div>
                  ))}
                </dl>
              )}
              {row.issues.length > 0 ? (
                <ul className="pw-preflight-problems">
                  {row.issues.map((issue, index) => (
                    <li key={`${issue.code}:${index}`}>
                      <Problem issue={issue} />
                    </li>
                  ))}
                </ul>
              ) : publishPreflightRowPassed(row) ? (
                <p>Nội dung, dung lượng và luồng đăng được hỗ trợ.</p>
              ) : (
                <p>
                  Có kiểm tra chưa đạt nhưng máy chưa trả lý do. Kiểm tra lại để
                  lấy kết quả mới.
                </p>
              )}
              <details className="pw-preflight-technical">
                <summary>Chi tiết kỹ thuật</summary>
                <dl>
                  <dt>Gói ứng dụng</dt>
                  <dd>{row.packageName || "Chưa đọc được"}</dd>
                  <dt>Phiên bản / ngôn ngữ</dt>
                  <dd>
                    {row.version || "Chưa đọc được"} /{" "}
                    {row.locale || "Chưa đọc được"}
                  </dd>
                  <dt>Thiết bị</dt>
                  <dd>{row.udid}</dd>
                  <dt>Kiểm tra</dt>
                  <dd>
                    Nội dung: {row.media}; luồng đăng: {row.composer};{network === "tiktok" && <> nhạc: {row.soundPicker};</>} dung lượng: {row.storage}
                  </dd>
                </dl>
                {row.issues.map((issue, index) => (
                  <p key={`${issue.code}:${index}`}>
                    <code>{issue.code}</code>: {issue.message}
                  </p>
                ))}
              </details>
            </article>
          ))}
        {!visibleRows.length && <p className="pw-preflight-empty">Không có máy trong nhóm này.</p>}
      </div>
      <PublishPager
        label="Kết quả kiểm tra"
        page={currentPage}
        size={pageSize}
        total={visibleRows.length}
        onPage={onPage}
      />
      {!report.canExecute && !hideRetry && (
        <button type="button" disabled={busy} onClick={() => { setView("blocked"); onPage(0); onRetry(); }}>
          Kiểm tra lại
        </button>
      )}
    </>
  );
}

function Problem({ issue }: { issue: PublishExecutionIssue }) {
  const problem = publishPreflightProblem(issue);
  return (
    <div className="pw-preflight-problem">
      <strong>{problem.title}</strong>
      <p>{problem.action}</p>
    </div>
  );
}
