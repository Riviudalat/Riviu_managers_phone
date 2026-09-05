import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { operationCancelBatch, operationGetRun, operationQueryRuns } from "../api";
import type { OperationRunDetail } from "../types";
import { LibraryBatchMonitor } from "./LibraryBatchMonitor";
import { useLibraryBatch } from "../useLibraryBatch";

vi.mock("../api", () => ({ operationCancelBatch:vi.fn(),operationGetRun:vi.fn(),operationQueryRuns:vi.fn() }));
const detail: OperationRunDetail = {
  summary:{ id:"appInstall:batch",sourceId:"batch",kind:"appInstall",title:"Fixture.apk",state:"running",targetCount:2,totalItems:2,completedItems:0,issueCount:0,retryableCount:0,retryScope:null,createdAt:null,updatedAt:null },
  items:[
    { id:"a",kind:"device",label:"Máy 2 · Kệ A",state:"running",udid:"a",errorCode:null,detail:null,evidence:null,retryable:false },
    { id:"b",kind:"device",label:"Máy 19 · Kệ B",state:"queued",udid:"b",errorCode:null,detail:null,evidence:null,retryable:false },
  ],
};
function Harness() {
  const batch = useLibraryBatch("appInstall");
  return <><button disabled={batch.active || batch.loading || !!batch.error}>Cài mới</button><LibraryBatchMonitor batch={batch}/></>;
}
beforeEach(() => {
  vi.mocked(operationQueryRuns).mockReset().mockResolvedValue({runs:[detail.summary],total:1,counts:{active:1,succeeded:0,attention:0},hasMore:false});
  vi.mocked(operationGetRun).mockReset().mockResolvedValue(detail);
  vi.mocked(operationCancelBatch).mockReset().mockResolvedValue(undefined);
});
describe("LibraryBatchMonitor durable lifecycle", () => {
  it("never includes uncertain devices in an explicit retry", async () => {
    const retry = vi.fn();
    render(<LibraryBatchMonitor batch={{ detail:{ ...detail,batch:{artifactId:"fixture",target:{targetRef:{type:"all"},included:[],excluded:[],rosterSha256:"a".repeat(64)}},
      items:[{...detail.items[0],state:"uncertain",retryable:false},{...detail.items[1],state:"failed",retryable:true}] },
      active:false,loading:false,error:null,reload:vi.fn(),follow:vi.fn() }} onRetry={retry}/>);
    await userEvent.click(screen.getByRole("button",{name:"Chạy lại 1 máy"}));
    expect(retry).toHaveBeenCalledWith("fixture",["b"]);
  });
  it("reattaches after page unmount and keeps immutable device labels without a roster", async () => {
    const page = render(<Harness/>);
    expect(await screen.findByText("Máy 19 · Kệ B")).toBeVisible();
    expect(screen.getByRole("button",{name:"Cài mới"})).toBeDisabled();
    page.unmount();
    render(<Harness/>);
    expect(await screen.findByText("Máy 2 · Kệ A")).toBeVisible();
    expect(operationGetRun).toHaveBeenCalledTimes(2);
    expect(screen.getByRole("button",{name:"Cài mới"})).toBeDisabled();
  });
  it("cancels only queued work and surfaces cancellation errors", async () => {
    vi.mocked(operationCancelBatch).mockRejectedValueOnce(new Error("database locked"));
    render(<Harness/>);
    await userEvent.click(await screen.findByRole("button",{name:"Dừng máy đang chờ"}));
    expect(await screen.findByRole("alert")).toHaveTextContent("database locked");
    expect(operationCancelBatch).toHaveBeenCalledWith("appInstall:batch");
  });
  it("fails closed until history reload succeeds", async () => {
    vi.mocked(operationQueryRuns).mockRejectedValueOnce(new Error("history unavailable"));
    render(<Harness/>);
    expect(await screen.findByRole("alert")).toHaveTextContent("history unavailable");
    expect(screen.getByRole("button",{name:"Cài mới"})).toBeDisabled();
    await userEvent.click(screen.getByRole("button",{name:"Thử lại tiến độ"}));
    await waitFor(() => expect(screen.queryByRole("alert")).toBeNull());
  });
});
