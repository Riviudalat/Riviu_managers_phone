import { cleanup, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import type {
  PublishPreflightAssignmentReport,
  PublishPreflightReport,
} from "../../types";
import { PublishPreflightResult } from "./PublishPreflightResult";

const row: PublishPreflightAssignmentReport = {
  ordinal: 0,
  bundleId: "bundle-1",
  udid: "phone-1",
  packageName: "com.zhiliaoapp.musically",
  version: "45.7.3",
  locale: "en-US",
  media: "pass",
  composer: "fail",
  soundPicker: "fail",
  storage: "pass",
  requiredBytes: 1024,
  availableBytes: 8192,
  issues: [
    {
      code: "composer_unmeasured",
      udid: "phone-1",
      message: "composer chưa đủ locator cho đúng package/build/locale này",
    },
    {
      code: "sound_picker_unmeasured",
      udid: "phone-1",
      message: "sound picker chưa được đo cho đúng package/build/locale này",
    },
  ],
};
const report: PublishPreflightReport = {
  inputDigest: "digest",
  canExecute: false,
  assignments: [row],
  issues: row.issues,
  sheetConfigured: false,
  targetSnapshot: {
    targetRef: { type: "explicit", udids: ["phone-1"] },
    included: [{ udid: "phone-1", number: 1, alias: "Đà Lạt" }],
    excluded: [],
    rosterSha256: "hash",
  },
};
afterEach(cleanup);

describe("publish preflight result", () => {
  it("opens on blocked machines, keeps global blockers visible, and lets the operator inspect passed machines", async () => {
    const passed = (ordinal: number): PublishPreflightAssignmentReport => ({
      ...row,
      ordinal,
      udid: `phone-${ordinal + 1}`,
      bundleId: `bundle-${ordinal + 1}`,
      media: "pass",
      composer: "pass",
      soundPicker: "pass",
      storage: "pass",
      issues: [],
    });
    const blocked = {
      ...row,
      ordinal: 4,
      udid: "phone-5",
      bundleId: "bundle-5",
    };
    const sheetIssue = {
      code: "sheet_connection_unverified",
      message: "Sheet chưa được xác minh",
    };
    render(
      <PublishPreflightResult
        report={{
          ...report,
          assignments: [passed(0), passed(1), passed(2), passed(3), blocked],
          issues: [sheetIssue, ...blocked.issues],
        }}
        machineName={(udid) => `Máy ${udid.split("-")[1]}`}
        page={0}
        onPage={vi.fn()}
        onRetry={vi.fn()}
        busy={false}
      />,
    );
    expect(screen.getByText("Kết nối Sheet chưa được xác minh")).toBeVisible();
    expect(screen.getByText("1 điều kiện chung · 1 máy cần xử lý · 4 máy đạt")).toBeVisible();
    expect(screen.getByRole("article", { name: "Máy 5" })).toBeVisible();
    expect(screen.queryByRole("article", { name: "Máy 1" })).toBeNull();
    expect(screen.getByRole("button", { name: "Đạt (4)" })).toBeVisible();
    await userEvent.click(screen.getByRole("button", { name: "Đạt (4)" }));
    expect(screen.getByRole("article", { name: "Máy 1" })).toBeVisible();
    expect(screen.queryByRole("article", { name: "Máy 5" })).toBeNull();
  });

  it("explains an empty passed-machine filter instead of leaving a blank list", async () => {
    render(<PublishPreflightResult report={report} machineName={() => "Máy 1"} page={0} onPage={vi.fn()} onRetry={vi.fn()} busy={false} />);
    await userEvent.click(screen.getByRole("button", { name: "Đạt (0)" }));
    expect(screen.getByText("Không có máy trong nhóm này.")).toBeVisible();
  });
  it("shows the blocking link check when all four legacy checks pass",()=>{
    const issue={code:"link_verification_unmeasured",udid:"phone-1",message:"locale unsupported"};
    render(<PublishPreflightResult report={{...report,assignments:[{...row,composer:"pass",soundPicker:"pass",issues:[issue],checks:[{id:"link",label:"Nhận diện xác minh liên kết",status:"blocked",reason:issue.message}]}],issues:[issue]}} machineName={()=>"Máy 1"} page={0} onPage={vi.fn()} onRetry={vi.fn()} busy={false}/>);
    expect(screen.getByText("Nhận diện xác minh liên kết")).toBeVisible();
    expect(screen.getByText("Bị chặn")).toBeVisible();
    expect(screen.queryByText("Có điều kiện chưa đạt")).toBeNull();
  });
  it("shows machine, installed version and actionable problems while technical detail stays collapsed", async () => {
    const retry = vi.fn();
    render(
      <PublishPreflightResult
        report={report}
        machineName={() => "Máy 1 · Đà Lạt"}
        page={0}
        onPage={vi.fn()}
        onRetry={retry}
        busy={false}
      />,
    );
    const card = screen.getByRole("article", { name: "Máy 1 · Đà Lạt" });
    expect(
      within(card).getByText(/TikTok quốc tế · Phiên bản 45.7.3/),
    ).toBeVisible();
    expect(
      within(card).getByText("Chưa hỗ trợ luồng đăng trên bản TikTok này"),
    ).toBeVisible();
    expect(
      within(card).getByText("Chưa hỗ trợ chọn nhạc trên bản TikTok này"),
    ).toBeVisible();
    expect(within(card).getByText("composer_unmeasured")).not.toBeVisible();
    await userEvent.click(within(card).getByText("Chi tiết kỹ thuật"));
    expect(within(card).getByText("composer_unmeasured")).toBeVisible();
    await userEvent.click(screen.getByRole("button", { name: "Kiểm tra lại" }));
    expect(retry).toHaveBeenCalledTimes(1);
    expect(report.canExecute).toBe(false);
  });

  it("does not report a failed check as ready when the backend omitted issue text", () => {
    render(
      <PublishPreflightResult
        report={{
          ...report,
          assignments: [{ ...row, issues: [] }],
          issues: [],
        }}
        machineName={() => "Máy 1"}
        page={8}
        onPage={vi.fn()}
        onRetry={vi.fn()}
        busy
      />,
    );
    expect(screen.getByRole("article", { name: "Máy 1" })).toBeVisible();
    expect(screen.getByText("Cần xử lý")).toBeVisible();
    expect(screen.queryByText("Đạt kiểm tra")).toBeNull();
    expect(screen.getByRole("button", { name: "Kiểm tra lại" })).toBeDisabled();
  });
});
