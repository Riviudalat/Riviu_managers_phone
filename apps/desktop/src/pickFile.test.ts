import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { open } from "@tauri-apps/plugin-dialog";
import { pickDirectory, pickFile, pickFiles } from "./pickFile";
import { toastError } from "./toastStore";

vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
vi.mock("./toastStore", () => ({ toastError: vi.fn() }));

const openMock = vi.mocked(open);
const toastMock = vi.mocked(toastError);

beforeEach(() => {
  openMock.mockReset();
  toastMock.mockReset();
});
afterEach(() => vi.clearAllMocks());

/**
 * The behaviour these tests exist for: a dialog that cannot open must not vanish.
 *
 * Every call site is `const path = await pickFile(...); if (!path) return;` with the device
 * work in a `try` *after* it — so a rejection here used to escape into an unawaited promise
 * and the operator saw the row click do nothing at all. Three rows were reported as broken on
 * exactly that basis.
 */
describe("the native pickers", () => {
  it("returns the chosen path", async () => {
    openMock.mockResolvedValue("D:\\phones\\app.apk");
    await expect(pickFile()).resolves.toBe("D:\\phones\\app.apk");
    expect(toastMock).not.toHaveBeenCalled();
  });

  it("returns null on cancel, in silence", async () => {
    openMock.mockResolvedValue(null);
    await expect(pickFile()).resolves.toBeNull();
    await expect(pickDirectory()).resolves.toBeNull();
    expect(toastMock).not.toHaveBeenCalled();
  });

  it("reports a dialog that could not open instead of rejecting", async () => {
    openMock.mockRejectedValue(new Error("dialog.open not allowed"));

    await expect(pickFile()).resolves.toBeNull();

    expect(toastMock).toHaveBeenCalledTimes(1);
    expect(toastMock.mock.calls[0][0]).toContain("hộp thoại");
  });

  it("reports the same way for the folder and multi-file pickers", async () => {
    openMock.mockRejectedValue(new Error("nope"));

    await expect(pickDirectory()).resolves.toBeNull();
    await expect(pickFiles()).resolves.toEqual([]);

    expect(toastMock).toHaveBeenCalledTimes(2);
  });

  it("takes the first path when the OS answers with an array", async () => {
    // Measured shape of the plugin's reply: `multiple: false` normally answers a string, but
    // the type allows an array and a caller that indexes blindly would crash on one.
    openMock.mockResolvedValue(["A:\\one.png", "A:\\two.png"] as unknown as string);
    await expect(pickFile()).resolves.toBe("A:\\one.png");
  });
});
