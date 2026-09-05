import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import * as api from "../api";
import { requestConfirm } from "../confirmStore";
import type { AutomationDefinitionRecord } from "../types";
import { AutomationProfileControl } from "./AutomationProfileControl";

vi.mock("../api", () => ({
  automationArchive: vi.fn(),
  automationCreate: vi.fn(),
  automationList: vi.fn(),
  automationRevise: vi.fn(),
  automationScheduleCreate: vi.fn(),
  automationScheduleList: vi.fn(),
  automationScheduleUpdate: vi.fn(),
}));

vi.mock("../confirmStore", () => ({
  requestConfirm: vi.fn(),
}));

const saved: AutomationDefinitionRecord = {
  definition: {
    id: "profile-1",
    name: "Ca sáng",
    kind: "nurture",
    latestRevision: 3,
    archived: false,
    createdAt: "2026-09-03T00:00:00Z",
    updatedAt: "2026-09-03T00:00:00Z",
  },
  revision: {
    definitionId: "profile-1",
    revision: 3,
    targetRef: { type: "group", groupId: "morning" },
    config: { saveEnabled: true, saveProb: 25 },
    createdAt: "2026-09-03T00:00:00Z",
  },
};

describe("AutomationProfileControl", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(api.automationList).mockResolvedValue([]);
    vi.mocked(api.automationScheduleList).mockResolvedValue([]);
    vi.mocked(requestConfirm).mockResolvedValue(true);
    vi.mocked(api.automationCreate).mockResolvedValue(saved);
    vi.mocked(api.automationRevise).mockResolvedValue({
      ...saved,
      definition: { ...saved.definition, latestRevision: 4 },
      revision: { ...saved.revision, revision: 4 },
    });
  });

  afterEach(cleanup);

  it("links disabled saving to an existing issue without repeating it", async () => {
    render(<>
      <p id="settings-issue">Cần sửa thời gian xem</p>
      <AutomationProfileControl kind="nurture" target={{ type: "all" }} config={{}}
        defaultName="Nuôi TikTok" disabled disabledReason="Cần sửa thời gian xem"
        disabledReasonId="settings-issue" />
    </>);
    const save = await screen.findByRole("button", { name: "Tạo hồ sơ" });
    expect(save).toBeDisabled();
    expect(save).toHaveAccessibleDescription("Cần sửa thời gian xem");
    expect(screen.getAllByText("Cần sửa thời gian xem")).toHaveLength(1);
    fireEvent.click(save);
    expect(api.automationCreate).not.toHaveBeenCalled();
  });

  it("creates a profile from the current typed target and config snapshot", async () => {
    render(
      <AutomationProfileControl
        kind="nurture"
        target={{ type: "group", groupId: "morning" }}
        config={{ saveEnabled: true, saveProb: 25 }}
        defaultName="Nuôi TikTok"
      />,
    );

    const nameInput = await screen.findByLabelText("Tên hồ sơ Nuôi TikTok");
    expect(screen.getByRole("button", { name: "Tạo hồ sơ" })).not.toHaveClass("primary");
    fireEvent.change(nameInput, {
      target: { value: "Ca sáng" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Tạo hồ sơ" }));

    await waitFor(() =>
      expect(api.automationCreate).toHaveBeenCalledWith(
        "Ca sáng",
        "nurture",
        { type: "group", groupId: "morning" },
        { saveEnabled: true, saveProb: 25 },
      ),
    );
    expect(await screen.findByRole("status")).toHaveTextContent("Đã tạo Ca sáng · bản 3");
  });

  it("does not persist a consequential profile when its save confirmation is declined", async () => {
    const confirmSave = vi.fn().mockResolvedValue(false);
    render(
      <AutomationProfileControl
        kind="publish"
        target={{ type: "all" }}
        config={{ executionConfirmed: true }}
        defaultName="Đăng bài"
        confirmSave={confirmSave}
      />,
    );

    await waitFor(() => expect(api.automationList).toHaveBeenCalled());
    fireEvent.click(screen.getByRole("button", { name: "Tạo hồ sơ" }));
    await waitFor(() => expect(confirmSave).toHaveBeenCalledTimes(1));
    expect(api.automationCreate).not.toHaveBeenCalled();
  });

  it("writes a new immutable revision of the selected profile", async () => {
    vi.mocked(api.automationList).mockResolvedValue([saved.definition]);
    render(
      <AutomationProfileControl
        kind="nurture"
        target={{ type: "all" }}
        config={{ saveEnabled: false, saveProb: 0 }}
        defaultName="Nuôi TikTok"
      />,
    );

    await screen.findByRole("option", { name: "Ca sáng · bản 3" });
    fireEvent.change(screen.getByLabelText("Hồ sơ Nuôi TikTok"), {
      target: { value: "profile-1" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Lưu bản mới" }));

    await waitFor(() =>
      expect(api.automationRevise).toHaveBeenCalledWith(
        "profile-1",
        3,
        { type: "all" },
        { saveEnabled: false, saveProb: 0 },
      ),
    );
    expect(requestConfirm).toHaveBeenCalledWith(
      expect.objectContaining({
        title: "Lưu thiết lập hiện tại thành bản mới?",
        confirmLabel: "Lưu bản mới",
      }),
    );
    expect(await screen.findByRole("status")).toHaveTextContent("Đã lưu Ca sáng · bản 4");
  });

  it("opens on a new profile and never revises a selected profile without explicit consent", async () => {
    vi.mocked(api.automationList).mockResolvedValue([saved.definition]);
    vi.mocked(requestConfirm).mockResolvedValue(false);
    render(
      <AutomationProfileControl
        kind="nurture"
        target={{ type: "all" }}
        config={{ saveEnabled: false, saveProb: 0 }}
        defaultName="Nuôi TikTok"
      />,
    );

    const selector = await screen.findByLabelText("Hồ sơ Nuôi TikTok");
    expect(selector).toHaveValue("");
    expect(screen.getByRole("button", { name: "Tạo hồ sơ" })).toBeEnabled();

    fireEvent.change(selector, { target: { value: "profile-1" } });
    fireEvent.click(screen.getByRole("button", { name: "Lưu bản mới" }));

    await waitFor(() => expect(requestConfirm).toHaveBeenCalledTimes(1));
    expect(api.automationRevise).not.toHaveBeenCalled();
  });

  it("shows a retry action when profiles cannot be loaded", async () => {
    vi.mocked(api.automationList)
      .mockRejectedValueOnce(new Error("database unavailable"))
      .mockResolvedValueOnce([]);
    render(
      <AutomationProfileControl
        kind="interaction"
        target={{ type: "all" }}
        config={{ actions: { like: true, comment: false, save: true } }}
        defaultName="Tương tác"
      />,
    );

    expect(await screen.findByText("database unavailable")).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "Thử lại hồ sơ" }));
    await waitFor(() => expect(api.automationList).toHaveBeenCalledTimes(2));
  });
});
