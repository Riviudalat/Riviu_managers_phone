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
  report,
  machineName,
  bundleName,
  page,
  onPage,
  onRetry,
  busy,
}: {
  report: PublishPreflightReport;
  machineName: (udid: string) => string;
  bundleName?: (id: string) => string;
  page: number;
  onPage: (page: number) => void;
  onRetry: () => void;
  busy: boolean;
}) {
  const currentPage = Math.min(
    page,
    Math.max(0, Math.ceil(report.assignments.length / 3) - 1),
  );
  return (
    <>
      <p
        className={report.canExecute ? "pw-success" : "pw-error"}
        role="status"
      >
        {report.canExecute
          ? "Đầu vào đã đạt kiểm tra. Chưa đăng bài."
          : "Chưa thể đăng. Xử lý điều kiện chưa đạt ở từng máy rồi kiểm tra lại."}
      </p>
      <div
        className="pw-check-results"
        tabIndex={0}
        role="region"
        aria-label="Kết quả kiểm tra từng máy"
      >
        {report.assignments
          .slice(currentPage * 3, (currentPage + 1) * 3)
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
                {publishTikTokBuildLabel(row)}
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
                    Nội dung: {row.media}; luồng đăng: {row.composer}; nhạc:{" "}
                    {row.soundPicker}; dung lượng: {row.storage}
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
        {report.issues
          .filter((issue) => !issue.udid)
          .map((issue, index) => (
            <article className="pw-preflight-device" key={`global:${index}`}>
              <Problem issue={issue} />
              <details className="pw-preflight-technical">
                <summary>Chi tiết kỹ thuật</summary>
                <p>
                  <code>{issue.code}</code>: {issue.message}
                </p>
              </details>
            </article>
          ))}
      </div>
      <PublishPager
        label="Kết quả kiểm tra"
        page={currentPage}
        size={3}
        total={report.assignments.length}
        onPage={onPage}
      />
      {!report.canExecute && (
        <button type="button" disabled={busy} onClick={onRetry}>
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
