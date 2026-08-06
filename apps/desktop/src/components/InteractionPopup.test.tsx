import { fireEvent, render, screen, waitFor } from "@testing-library/react";
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
  vi.clearAllMocks();
});

const devices = [
  {
    udid: "actor-a",
    name: "Phone A",
    model: "iPhone 8",
    iosVersion: "16.7.16",
    connection: "usb",
    status: "ready",
    wdaReady: true,
  },
  {
    udid: "actor-b",
    name: "Phone B",
    model: "iPhone 8",
    iosVersion: "16.7.15",
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
