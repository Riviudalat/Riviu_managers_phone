import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import * as api from "../../api";
import { useDeviceKeyboards } from "./useDeviceKeyboards";

vi.mock("../../api", () => ({ deviceShell: vi.fn() }));
vi.mock("../../toastStore", () => ({ pushToast: vi.fn(), toastError: vi.fn() }));

const deviceShell = api.deviceShell as unknown as ReturnType<typeof vi.fn>;
const runBusy = async (work: () => Promise<void>) => {
  await work();
  return true;
};

const LIST = [
  "com.google.android.inputmethod.latin/.LatinIME",
  "com.riviu.agent/.RiviuIME",
].join("\n");

/**
 * Reading and switching a phone's keyboard, without the focus overlay around it.
 *
 * The rule worth pinning is the second shell call: it answers a different question from the
 * first, and its failure must not take the list down with it — otherwise a phone that will
 * not report its *current* keyboard appears to have none at all.
 */
describe("useDeviceKeyboards", () => {
  beforeEach(() => {
    deviceShell.mockReset();
  });

  it("asks nothing until it is told to", () => {
    const { result } = renderHook(() => useDeviceKeyboards("udid-1", runBusy));
    expect(result.current.keyboards).toBeNull();
    expect(deviceShell).not.toHaveBeenCalled();
  });

  it("lists the installed keyboards and marks the current one", async () => {
    deviceShell
      .mockResolvedValueOnce({ stdout: LIST })
      .mockResolvedValueOnce({ stdout: "com.riviu.agent/.RiviuIME" });

    const { result } = renderHook(() => useDeviceKeyboards("udid-1", runBusy));
    await act(async () => {
      await result.current.load();
    });

    expect(result.current.keyboards?.map((k) => k.id)).toEqual([
      "com.google.android.inputmethod.latin/.LatinIME",
      "com.riviu.agent/.RiviuIME",
    ]);
    expect(result.current.current).toBe("com.riviu.agent/.RiviuIME");
  });

  it("keeps the list when the phone will not say which keyboard is current", async () => {
    // Two shells rather than one round trip exactly so this failure stays contained.
    deviceShell
      .mockResolvedValueOnce({ stdout: LIST })
      .mockRejectedValueOnce(new Error("settings: permission denied"));

    const { result } = renderHook(() => useDeviceKeyboards("udid-1", runBusy));
    await act(async () => {
      await result.current.load();
    });

    expect(result.current.keyboards).toHaveLength(2);
    expect(result.current.current).toBeNull();
  });

  it("an unreadable phone reports no keyboards rather than staying on 'loading'", async () => {
    deviceShell.mockRejectedValue(new Error("device offline"));
    const { result } = renderHook(() => useDeviceKeyboards("udid-1", runBusy));
    await act(async () => {
      await result.current.load();
    });
    await waitFor(() => expect(result.current.keyboards).toEqual([]));
  });

  it("refuses to set an id the phone did not just print", async () => {
    // The value reaches a real shell, so it is looked up in the parsed list rather than
    // taken from the caller.
    deviceShell
      .mockResolvedValueOnce({ stdout: LIST })
      .mockResolvedValueOnce({ stdout: "com.riviu.agent/.RiviuIME" });
    const { result } = renderHook(() => useDeviceKeyboards("udid-1", runBusy));
    await act(async () => {
      await result.current.load();
    });
    deviceShell.mockClear();

    await act(async () => {
      await result.current.choose({ id: "evil/.Injected", label: "evil" });
    });

    expect(deviceShell).not.toHaveBeenCalled();
    expect(result.current.current).toBe("com.riviu.agent/.RiviuIME");
  });

  it("switches to one it did print", async () => {
    deviceShell
      .mockResolvedValueOnce({ stdout: LIST })
      .mockResolvedValueOnce({ stdout: "com.riviu.agent/.RiviuIME" });
    const { result } = renderHook(() => useDeviceKeyboards("udid-1", runBusy));
    await act(async () => {
      await result.current.load();
    });
    deviceShell.mockClear().mockResolvedValue({ stdout: "" });

    const target = result.current.keyboards![0];
    await act(async () => {
      await result.current.choose(target);
    });

    expect(deviceShell).toHaveBeenCalledWith("udid-1", `ime set ${target.id}`);
    expect(result.current.current).toBe(target.id);
  });
});
