import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { DeviceSyslogPopup } from "./DeviceSyslogPopup";
import type { DeviceInfo } from "../types";

const readSyslog = vi.hoisted(() => vi.fn());

// Every export this component reaches for, named. An omitted one is `undefined`, and calling it
// throws during render — one missing entry reddens the whole file with a React stack.
vi.mock("../api", () => ({ syslog: readSyslog }));

const REDMI: DeviceInfo = {
  udid: "10969614",
  name: "Redmi 12C",
  model: "23021RAAEG",
  platform: "android",
  osVersion: "15",
  connection: "usb",
  status: "ready",
  wdaReady: true,
  tileStreamState: "sampling",
};

beforeEach(() => {
  readSyslog.mockReset();
});
afterEach(cleanup);

describe("reading a phone's own log", () => {
  /**
   * **`syslog` was a registered command with no caller.**
   *
   * The command, `Driver::syslog_tail` and seven test mocks stubbing it all existed; `api.ts`
   * never invoked any of it. So the app that drives twenty phones could not read one phone's
   * log — in exactly the situation this pass came from, a phone that lists and will not drive,
   * where the phone's log is where the reason is.
   */
  it("shows the log the phone returned", async () => {
    readSyslog.mockResolvedValue("08-27 09:14:02.512 E ActivityManager: ANR in com.zhiliaoapp");
    render(<DeviceSyslogPopup device={REDMI} onClose={vi.fn()} />);

    await waitFor(() => expect(screen.getByText(/ANR in com.zhiliaoapp/)).toBeInTheDocument());
    expect(readSyslog).toHaveBeenCalledWith("10969614", expect.any(Number));
  });

  /**
   * **An empty log is not a failed read.**
   *
   * A phone whose buffer was just cleared answers with nothing, and rendering that as a blank
   * panel makes it look like the request failed.
   */
  it("says the log came back empty rather than showing nothing", async () => {
    readSyslog.mockResolvedValue("   \n  \n");
    render(<DeviceSyslogPopup device={REDMI} onClose={vi.fn()} />);

    await waitFor(() => expect(screen.getByText(/log rỗng/)).toBeInTheDocument());
    expect(screen.queryByRole("alert")).toBeNull();
  });

  /** A rejection reads as a sentence, not as `[object Object]`. */
  it("describes a refusal by its message", async () => {
    readSyslog.mockRejectedValue({ code: "DeviceBusy", message: "máy đang chạy nuôi" });
    render(<DeviceSyslogPopup device={REDMI} onClose={vi.fn()} />);

    await waitFor(() => expect(screen.getByRole("alert")).toBeInTheDocument());
    expect(screen.getByRole("alert").textContent).toContain("máy đang chạy nuôi");
    expect(screen.getByRole("alert").textContent).not.toContain("[object Object]");
  });

  /**
   * **The panel warns that the tile goes quiet, because it does.**
   *
   * `syslog` takes the lease with `LeaseStream::Park`, so the live producer stops for the
   * duration. A tile going blank the instant you ask for a log looks like the fault you were
   * investigating, so the panel says it first.
   */
  it("warns that the stream pauses while it reads", async () => {
    readSyslog.mockResolvedValue("x");
    render(<DeviceSyslogPopup device={REDMI} onClose={vi.fn()} />);

    await waitFor(() => expect(screen.getByText(/tạm dừng/)).toBeInTheDocument());
  });

  it("re-reads on demand and closes when asked", async () => {
    readSyslog.mockResolvedValue("first");
    const onClose = vi.fn();
    render(<DeviceSyslogPopup device={REDMI} onClose={onClose} />);
    await waitFor(() => expect(screen.getByText("first")).toBeInTheDocument());

    readSyslog.mockResolvedValue("second");
    await userEvent.click(screen.getByRole("button", { name: "Đọc lại" }));
    await waitFor(() => expect(screen.getByText("second")).toBeInTheDocument());

    await userEvent.click(screen.getByRole("button", { name: "Đóng" }));
    expect(onClose).toHaveBeenCalledTimes(1);
  });
});
