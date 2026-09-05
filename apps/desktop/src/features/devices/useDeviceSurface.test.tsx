import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useDeviceSurface } from "./useDeviceSurface";
import type { DeviceInfo } from "../../types";

const notify = vi.hoisted(() => vi.fn());
vi.mock("../../toastStore", () => ({ pushToast: notify }));
const phone = { udid:"phone-a", name:"Máy A" } as DeviceInfo;
const other = { udid:"phone-b", name:"Máy B" } as DeviceInfo;
beforeEach(() => notify.mockReset());

describe("useDeviceSurface", () => {
  it("preserves an open phone through transient empty scans", () => {
    const hook = renderHook(({ devices }) => useDeviceSurface(devices, "tệp"), { initialProps:{ devices:[phone] } });
    act(() => hook.result.current[1](phone.udid));
    hook.rerender({ devices:[] });
    expect(hook.result.current[0]).toBe(phone.udid);
    hook.rerender({ devices:[phone] });
    expect(hook.result.current[0]).toBe(phone.udid);
    expect(notify).not.toHaveBeenCalled();
  });
  it("announces one departure using the captured name and reopens after reconnect", () => {
    const hook = renderHook(({ devices }) => useDeviceSurface(devices, "trình quản lý tệp"), { initialProps:{ devices:[phone,other] } });
    act(() => hook.result.current[1](phone.udid));
    hook.rerender({ devices:[other] });
    expect(hook.result.current[0]).toBeNull();
    expect(notify).toHaveBeenCalledTimes(1);
    expect(notify).toHaveBeenCalledWith("warn", "Máy đã rời khỏi danh sách", "Máy A không còn kết nối — đã đóng trình quản lý tệp.");
    hook.rerender({ devices:[other] });
    expect(notify).toHaveBeenCalledTimes(1);
    hook.rerender({ devices:[phone,other] });
    act(() => hook.result.current[1](phone.udid));
    expect(hook.result.current[0]).toBe(phone.udid);
  });
  it("keeps the open callback stable while reading the newest roster name", () => {
    const hook = renderHook(({ devices }) => useDeviceSurface(devices, "log"), { initialProps:{ devices:[phone,other] } });
    const open = hook.result.current[1];
    hook.rerender({ devices:[{...phone,name:"Tên mới"},other] });
    expect(hook.result.current[1]).toBe(open);
    act(() => open(phone.udid));
    hook.rerender({ devices:[other] });
    expect(notify).toHaveBeenCalledWith("warn", "Máy đã rời khỏi danh sách", "Tên mới không còn kết nối — đã đóng log.");
  });
  it("closes explicitly without reporting a departure", () => {
    const hook = renderHook(() => useDeviceSurface([phone], "tệp"));
    act(() => hook.result.current[1](phone.udid));
    act(() => hook.result.current[1](null));
    expect(hook.result.current[0]).toBeNull();
    expect(notify).not.toHaveBeenCalled();
  });
});
