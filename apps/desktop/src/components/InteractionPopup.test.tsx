import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { InteractionPopup } from "./InteractionPopup";
import type { ThreadCampaignRequest } from "../types";

const { parseLinks, startThread } = vi.hoisted(() => ({
  parseLinks: vi.fn(async () => [
    {
      lineNo: 1,
      original: "https://www.tiktok.com/@creator/video/123",
      target: {
        originalUrl: "https://www.tiktok.com/@creator/video/123",
        normalizedUrl: "https://www.tiktok.com/@creator/video/123",
        targetKey: "content:123",
        contentId: "123",
        author: "creator",
        kind: "video",
      },
      error: null,
    },
  ]),
  startThread: vi.fn(async () => ({
    queued: true,
    campaign: {
      id: "campaign-1",
      requestId: "request-1",
      state: "running",
      messageCount: 2,
      targetCount: 1,
      succeededMessages: 0,
      failedMessages: 0,
      updatedAt: "2026-08-04T00:00:00Z",
    },
  })),
}));

vi.mock("../api", () => ({
  interactionCancel: vi.fn(async () => undefined),
  interactionGet: vi.fn(async () => null),
  interactionList: vi.fn(async () => []),
  interactionParseLinks: parseLinks,
  interactionResolveLinks: vi.fn(async () => []),
  interactionStartThread: startThread,
  listenRiviuEvents: vi.fn(async () => () => undefined),
}));

afterEach(() => {
  // Explicit, not relying on the auto-cleanup: several tests in this file render the
  // same popup, and a leaked render turns every `getBy*` into "found multiple elements"
  // in whichever test happens to run next — a failure that points at the wrong test.
  cleanup();
  vi.clearAllMocks();
});

const devices = [
  {
    udid: "actor-a",
    name: "Phone A",
    model: "iPhone 8",
    platform: "ios",
    osVersion: "16.7.16",
    connection: "usb",
    status: "ready",
    wdaReady: true,
  },
  {
    udid: "actor-b",
    name: "Phone B",
    model: "iPhone 8",
    platform: "ios",
    osVersion: "16.7.15",
    connection: "usb",
    status: "ready",
    wdaReady: true,
  },
  {
    udid: "actor-android",
    name: "Phone C",
    model: "Redmi Note 12",
    platform: "android",
    osVersion: "15",
    connection: "usb",
    status: "ready",
    wdaReady: true,
  },
] as never[];

describe("InteractionPopup", () => {
  it("parses multiline links and submits every selected actor", async () => {
    render(<InteractionPopup devices={devices} selected={[]} onClose={() => undefined} />);
    const input = screen.getByPlaceholderText("https://www.tiktok.com/@creator/video/123");
    fireEvent.change(input, { target: { value: "https://www.tiktok.com/@creator/video/123" } });
    await waitFor(() => expect(screen.getByText(/creator\/video\/123/)).toBeVisible());

    fireEvent.click(screen.getByRole("button", { name: "Chạy ngay" }));
    await waitFor(() => expect(startThread).toHaveBeenCalledTimes(1));
    const request = (startThread.mock.calls as unknown as Array<[ThreadCampaignRequest]>)[0][0];
    expect(request.actorUdids).toEqual(["actor-a", "actor-b"]);
    expect(request.messageCount).toBe(2);
    expect(request.targets[0].targetKey).toBe("content:123");
    expect(screen.getByRole("tab", { name: "Monitor" })).toHaveAttribute("aria-selected", "true");
  });

  it("offers Android as an actor, grouped by how it reads the screen", async () => {
    // Android used to be filtered out here because the Interaction send path was
    // pixel-only. It now drives the drawer through the accessibility hierarchy, so
    // excluding it would hide working hardware. What replaces the filter is the grouping
    // plus the mixed-thread refusal below.
    render(<InteractionPopup devices={devices} selected={[]} onClose={() => undefined} />);
    expect(screen.getByLabelText("Phone C")).toBeVisible();
    expect(screen.getByText("iPhone (nhận dạng ảnh)")).toBeVisible();
    expect(screen.getByText("Android (hierarchy)")).toBeVisible();
    // Pre-selection comes from one group only, so the default is a runnable thread. Two
    // iPhones outnumber one Android here, so the iPhones win.
    expect(screen.getByRole("button", { name: "Chạy ngay" })).toBeEnabled();

    fireEvent.change(screen.getByPlaceholderText("https://www.tiktok.com/@creator/video/123"), {
      target: { value: "https://www.tiktok.com/@creator/video/123" },
    });
    await waitFor(() => expect(screen.getByText(/creator\/video\/123/)).toBeVisible());
    fireEvent.click(screen.getByRole("button", { name: "Chạy ngay" }));
    await waitFor(() => expect(startThread).toHaveBeenCalledTimes(1));
    const request = (startThread.mock.calls as unknown as Array<[ThreadCampaignRequest]>)[0][0];
    expect(request.actorUdids).toEqual(["actor-a", "actor-b"]);
  });

  it("refuses a nested thread that mixes a pixel actor with a hierarchy actor", async () => {
    // The chain is linear and each message is sent from a different actor, so message N
    // must find message N-1's comment. The two readers get the author label from
    // different places and need not agree, which breaks the chain halfway with no
    // explanation. The server refuses this as well; this is the round trip saved and the
    // reason stated where the operator is looking.
    render(<InteractionPopup devices={devices} selected={[]} onClose={() => undefined} />);
    fireEvent.change(screen.getByPlaceholderText("https://www.tiktok.com/@creator/video/123"), {
      target: { value: "https://www.tiktok.com/@creator/video/123" },
    });
    await waitFor(() => expect(screen.getByText(/creator\/video\/123/)).toBeVisible());

    // Add the Android device to the two already-selected iPhones.
    fireEvent.click(screen.getByLabelText("Phone C"));
    await waitFor(() =>
      expect(screen.getByText(/không chạy trộn iPhone với Android/)).toBeVisible(),
    );
    expect(screen.getByRole("button", { name: "Chạy ngay" })).toBeDisabled();
    expect(startThread).not.toHaveBeenCalled();
  });

  it("allows the same mixed selection in Standalone, which has no parent to find", async () => {
    render(<InteractionPopup devices={devices} selected={[]} onClose={() => undefined} />);
    fireEvent.change(screen.getByPlaceholderText("https://www.tiktok.com/@creator/video/123"), {
      target: { value: "https://www.tiktok.com/@creator/video/123" },
    });
    await waitFor(() => expect(screen.getByText(/creator\/video\/123/)).toBeVisible());
    // One from each group, so the selection is genuinely mixed and still within the
    // default message budget (2 messages, 2 actors).
    fireEvent.click(screen.getByLabelText("Phone B"));
    fireEvent.click(screen.getByLabelText("Phone C"));
    // The mode is a `<select>`, not a radio — changing its value is what switches modes.
    fireEvent.change(screen.getByLabelText(/Kiểu tương tác/), {
      target: { value: "standalone" },
    });

    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Chạy ngay" })).toBeEnabled(),
    );
    fireEvent.click(screen.getByRole("button", { name: "Chạy ngay" }));
    await waitFor(() => expect(startThread).toHaveBeenCalledTimes(1));
    const request = (startThread.mock.calls as unknown as Array<[ThreadCampaignRequest]>)[0][0];
    expect(request.actorUdids).toEqual(["actor-a", "actor-android"]);
    expect(request.mode).toBe("standalone");
  });

  it("requires at least two actors before dispatch", async () => {
    render(<InteractionPopup devices={devices} selected={[]} onClose={() => undefined} />);
    fireEvent.change(screen.getByPlaceholderText("https://www.tiktok.com/@creator/video/123"), {
      target: { value: "https://www.tiktok.com/@creator/video/123" },
    });
    await waitFor(() => expect(screen.getByText(/creator\/video\/123/)).toBeVisible());
    fireEvent.click(screen.getByLabelText("Phone B"));
    fireEvent.click(screen.getByRole("button", { name: "Chạy ngay" }));
    await waitFor(() => expect(screen.getByText("Chọn từ 2 đến 6 thiết bị làm actor")).toBeVisible());
    expect(startThread).not.toHaveBeenCalled();
  });
});
