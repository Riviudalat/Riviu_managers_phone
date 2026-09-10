import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { publishSheetPrepare, publishSheetGetConfig } from "../../api";
import type { PublishSheetCheckResult } from "../../types";
import { PublishSheetConnection } from "./PublishSheetConnection";

vi.mock("../../api", () => ({ publishSheetPrepare: vi.fn(), publishSheetGetConfig: vi.fn() }));
const first = "https://docs.google.com/spreadsheets/d/first/edit#gid=0";
const second = "https://docs.google.com/spreadsheets/d/second/edit#gid=0";
const result = (verified = true): PublishSheetCheckResult => ({ sheetUrl: first, spreadsheetId: "first", sheetGid: 0,
  readable: true, connectionVerified: verified, layout: "compact", columns: [], message: verified ? "Verified first" : "Read only" });
function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: Error) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}
beforeEach(() => {
  vi.resetAllMocks();
  vi.mocked(publishSheetPrepare).mockImplementation(() => new Promise(() => {}));
  vi.mocked(publishSheetGetConfig).mockResolvedValue({ webhookUrl: "https://example.com/hook", hasToken: true, internalReporting: false, sheetUrl: "" });
});
afterEach(cleanup);

describe("PublishSheetConnection", () => {
  it("does not authorize an unknown saved destination while config is loading", async () => {
    const config = deferred<Awaited<ReturnType<typeof publishSheetGetConfig>>>();
    vi.mocked(publishSheetGetConfig).mockReturnValue(config.promise);
    const ready = vi.fn();
    render(<PublishSheetConnection onReadyChange={ready} />);
    expect(ready).toHaveBeenLastCalledWith(false);
    await act(async () => config.resolve({ webhookUrl: "https://example.com/hook", hasToken: true, sheetUrl: first }));
    expect(ready).toHaveBeenLastCalledWith(false);
  });
  it("keeps the existing configured writer ready until a link is entered", async () => {
    const ready = vi.fn();
    render(<PublishSheetConnection onReadyChange={ready} />);
    await waitFor(() => expect(ready).toHaveBeenLastCalledWith(true));
    fireEvent.change(screen.getByRole("textbox", { name: "Link Google Sheet" }), { target: { value: first } });
    expect(ready).toHaveBeenLastCalledWith(false);
    fireEvent.change(screen.getByRole("textbox", { name: "Link Google Sheet" }), { target: { value: "" } });
    expect(ready).toHaveBeenLastCalledWith(false);
  });

  it("requires verification again for a restored URL", async () => {
    vi.mocked(publishSheetGetConfig).mockResolvedValue({ webhookUrl: "https://example.com/hook", hasToken: true, internalReporting: false, sheetUrl: first });
    const pending = deferred<PublishSheetCheckResult>();
    vi.mocked(publishSheetPrepare).mockReturnValue(pending.promise);
    const ready = vi.fn();
    render(<PublishSheetConnection onReadyChange={ready} />);
    await waitFor(() => expect(screen.getByRole("textbox", { name: "Link Google Sheet" })).toHaveValue(first));
    expect(ready).toHaveBeenLastCalledWith(false);
    expect(publishSheetPrepare).toHaveBeenCalledWith(first);
    await act(async () => pending.resolve(result()));
    await waitFor(() => expect(ready).toHaveBeenLastCalledWith(true));
    fireEvent.change(screen.getByRole("textbox", { name: "Link Google Sheet" }), { target: { value: second } });
    expect(ready).toHaveBeenLastCalledWith(false);
  });

  it("locks the URL until completion and never starts overlapping backend checks", async () => {
    const pending = deferred<PublishSheetCheckResult>();
    vi.mocked(publishSheetPrepare).mockReturnValue(pending.promise);
    const ready = vi.fn();
    render(<PublishSheetConnection onReadyChange={ready} />);
    const input = screen.getByRole("textbox", { name: "Link Google Sheet" });
    fireEvent.change(input, { target: { value: first } });
    fireEvent.click(screen.getByRole("button", { name: "Kết nối Sheet" }));
    expect(input).toBeDisabled();
    expect(screen.getByRole("button", { name: "Đang chuẩn bị…" })).toBeDisabled();
    fireEvent.change(input, { target: { value: second } });
    fireEvent.keyDown(input, { key: "Enter" });
    expect(publishSheetPrepare).toHaveBeenCalledTimes(1);
    expect(publishSheetPrepare).toHaveBeenCalledWith(first);
    expect(ready).toHaveBeenLastCalledWith(false);
    await act(async () => pending.resolve(result()));
    expect(input).toHaveValue(first);
    expect(input).toBeEnabled();
    expect(ready).toHaveBeenLastCalledWith(true);
  });

  it("keeps read-only and failed checks unready and releases busy on failure", async () => {
    vi.mocked(publishSheetPrepare).mockResolvedValueOnce(result(false)).mockRejectedValueOnce(new Error("Network failed"));
    const ready = vi.fn();
    render(<PublishSheetConnection onReadyChange={ready} />);
    const input = screen.getByRole("textbox", { name: "Link Google Sheet" });
    fireEvent.change(input, { target: { value: first } });
    fireEvent.click(screen.getByRole("button", { name: "Kết nối Sheet" }));
    await screen.findByText("Read only");
    expect(ready).toHaveBeenLastCalledWith(false);
    fireEvent.click(screen.getByRole("button", { name: "Kết nối Sheet" }));
    await screen.findByRole("alert");
    expect(input).toBeEnabled();
    expect(ready).toHaveBeenLastCalledWith(false);
  });

  it("ignores a late config URL after editing and a check reply after unmount", async () => {
    const config = deferred<Awaited<ReturnType<typeof publishSheetGetConfig>>>();
    const pending = deferred<PublishSheetCheckResult>();
    vi.mocked(publishSheetGetConfig).mockReturnValue(config.promise);
    vi.mocked(publishSheetPrepare).mockReturnValue(pending.promise);
    const ready = vi.fn();
    const view = render(<PublishSheetConnection onReadyChange={ready} />);
    const input = screen.getByRole("textbox", { name: "Link Google Sheet" });
    fireEvent.change(input, { target: { value: first } });
    await act(async () => config.resolve({ webhookUrl: "", hasToken: false, internalReporting: false, sheetUrl: second }));
    expect(input).toHaveValue(first);
    fireEvent.click(screen.getByRole("button", { name: "Kết nối Sheet" }));
    view.unmount();
    const count = ready.mock.calls.length;
    await act(async () => pending.resolve(result()));
    expect(ready).toHaveBeenCalledTimes(count);
  });
});

it("unlocks after successful manual prepare when initial config loading failed", async () => {
  vi.mocked(publishSheetGetConfig).mockRejectedValueOnce(new Error("Temporary config error"));
  vi.mocked(publishSheetPrepare).mockResolvedValue(result());
  const ready = vi.fn();
  render(<PublishSheetConnection onReadyChange={ready}/>);
  await screen.findByRole("alert");
  fireEvent.change(screen.getByRole("textbox", { name: "Link Google Sheet" }), { target: { value: first } });
  fireEvent.click(screen.getByRole("button", { name: "Kết nối Sheet" }));
  await waitFor(() => expect(ready).toHaveBeenLastCalledWith(true));
});
