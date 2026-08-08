import { renderHook } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import {
  answerConfirm,
  requestConfirm,
  resetConfirms,
  useConfirmRequest,
} from "./confirmStore";

/** The store is module-global, so every test starts with nothing pending. */
afterEach(resetConfirms);

function active() {
  return renderHook(() => useConfirmRequest()).result.current;
}

describe("confirmStore", () => {
  it("resolves with the operator's answer", async () => {
    const accepted = requestConfirm({ title: "Phục hồi thiết bị?" });
    const first = active();
    expect(first?.title).toBe("Phục hồi thiết bị?");

    answerConfirm(first!.id, true);
    await expect(accepted).resolves.toBe(true);
    expect(active()).toBeNull();

    const declined = requestConfirm({ title: "Lưu trữ Flow?" });
    answerConfirm(active()!.id, false);
    await expect(declined).resolves.toBe(false);
  });

  it("queues a second request instead of clobbering the first", async () => {
    const firstAnswer = requestConfirm({ title: "Đầu tiên" });
    const secondAnswer = requestConfirm({ title: "Thứ hai" });

    // Only one dialog is ever on screen; the second waits its turn.
    expect(active()?.title).toBe("Đầu tiên");

    answerConfirm(active()!.id, true);
    await expect(firstAnswer).resolves.toBe(true);

    expect(active()?.title).toBe("Thứ hai");
    answerConfirm(active()!.id, false);
    await expect(secondAnswer).resolves.toBe(false);
    expect(active()).toBeNull();
  });

  it("ignores an answer aimed at a request that is no longer showing", async () => {
    const answer = requestConfirm({ title: "Chỉ một lần" });
    const { id } = active()!;

    answerConfirm(id, true);
    await expect(answer).resolves.toBe(true);

    // A late click from an unmounted dialog must not disturb the queue.
    answerConfirm(id, false);
    expect(active()).toBeNull();
  });

  it("declines everything still pending when the surface is torn down", async () => {
    const first = requestConfirm({ title: "Đầu tiên" });
    const second = requestConfirm({ title: "Thứ hai" });

    resetConfirms();

    await expect(first).resolves.toBe(false);
    await expect(second).resolves.toBe(false);
    expect(active()).toBeNull();
  });
});
