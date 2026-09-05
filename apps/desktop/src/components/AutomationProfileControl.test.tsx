import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import * as api from "../api";
import { requestConfirm, requestSaveChanges } from "../confirmStore";
import { hasWorkspaceDrafts, requestWorkspaceLeave, useWorkspaceDraft } from "../workspaceDraft";
import { createRef } from "react";
import type { AutomationDefinitionRecord } from "../types";
import { AutomationProfileControl, type AutomationProfileHandle } from "./AutomationProfileControl";

vi.mock("../api", () => ({
  automationArchive: vi.fn(),
  automationCreate: vi.fn(),
  automationGet: vi.fn(),
  automationList: vi.fn(),
  automationRevise: vi.fn(),
  automationScheduleCreate: vi.fn(),
  automationScheduleList: vi.fn(),
  automationScheduleUpdate: vi.fn(),
}));

vi.mock("../confirmStore", () => ({
  requestConfirm: vi.fn(),
  requestSaveChanges: vi.fn().mockResolvedValue("discard"),
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
    vi.mocked(api.automationGet).mockResolvedValue(saved);
    vi.mocked(api.automationRevise).mockResolvedValue({
      ...saved,
      definition: { ...saved.definition, latestRevision: 4 },
      revision: { ...saved.revision, revision: 4 },
    });
  });

  afterEach(cleanup);

  it("guards name-only edits and keeps them when navigation is declined", async () => {
    render(<AutomationProfileControl kind="nurture" target={{ type: "all" }} config={{}} defaultName="Nuôi TikTok" />);
    const name = await screen.findByLabelText("Tên hồ sơ Nuôi TikTok");
    expect(hasWorkspaceDrafts()).toBe(false);
    fireEvent.change(name, { target: { value: "Tên mới" } });
    expect(hasWorkspaceDrafts()).toBe(true);
    vi.mocked(requestSaveChanges).mockResolvedValueOnce("stay");
    await act(async () => expect(await requestWorkspaceLeave()).toBe(false));
    expect(name).toHaveValue("Tên mới");
    await act(async () => expect(await requestWorkspaceLeave()).toBe(true));
    expect(name).toHaveValue("Nuôi TikTok");
    expect(api.automationCreate).not.toHaveBeenCalled();
  });

  it("keeps a newer profile name after an earlier create finishes", async () => {
    let complete!: (record: AutomationDefinitionRecord) => void;
    vi.mocked(api.automationCreate).mockImplementationOnce(() => new Promise((resolve) => { complete = resolve; }));
    render(<AutomationProfileControl kind="nurture" target={{ type: "all" }} config={{}} defaultName="Nuôi TikTok" />);
    const name = await screen.findByLabelText("Tên hồ sơ Nuôi TikTok");
    fireEvent.change(name, { target: { value: "Ca sáng" } });
    fireEvent.click(screen.getByRole("button", { name: "Tạo hồ sơ" }));
    fireEvent.change(name, { target: { value: "Ca tối" } });
    await act(async () => complete(saved));
    expect(name).toHaveValue("Ca tối");
    expect(name).toBeEnabled();
    expect(hasWorkspaceDrafts()).toBe(true);
    expect(screen.getByLabelText("Hồ sơ Nuôi TikTok")).toHaveValue("");
  });

  it("saves one snapshot when both the workspace and profile name are dirty", async () => {
    const ref = createRef<AutomationProfileHandle>();
    function Fixture() {
      useWorkspaceDraft({ id: "fixture-parent", label: "Thiết lập", dirty: true, snapshotKey: "edited", save: async () => ref.current!.save(), discard: () => undefined });
      return <AutomationProfileControl ref={ref} kind="nurture" target={{ type: "all" }} config={{}} defaultName="Nuôi TikTok" dirty draftId="fixture-parent" />;
    }
    render(<Fixture />);
    fireEvent.change(await screen.findByLabelText("Tên hồ sơ Nuôi TikTok"), { target: { value: "Ca sáng" } });
    vi.mocked(requestSaveChanges).mockResolvedValueOnce("save");
    await act(async () => expect(await requestWorkspaceLeave()).toBe(true));
    expect(api.automationCreate).toHaveBeenCalledTimes(1);
    expect(api.automationRevise).not.toHaveBeenCalled();
  });

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
    fireEvent.click(await screen.findByRole("button", { name: "Lưu bản mới" }));

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
    fireEvent.click(await screen.findByRole("button", { name: "Lưu bản mới" }));

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

  it("loads the selected pinned revision before exposing it for use", async () => {
    vi.mocked(api.automationList).mockResolvedValue([saved.definition]);
    const onApply = vi.fn();
    render(<AutomationProfileControl kind="nurture" target={{type:"all"}} config={{}}
      defaultName="Nuôi TikTok" onApply={onApply} />);
    fireEvent.change(await screen.findByLabelText("Hồ sơ Nuôi TikTok"), {target:{value:"profile-1"}});
    await waitFor(() => expect(onApply).toHaveBeenCalledWith(saved));
    expect(api.automationGet).toHaveBeenCalledWith("profile-1", 3);
    expect(api.automationCreate).not.toHaveBeenCalled();
    expect(api.automationRevise).not.toHaveBeenCalled();
  });

  it("does not replace edits made while a profile is loading", async () => {
    vi.mocked(api.automationList).mockResolvedValue([saved.definition]);
    let resolve!: (record: AutomationDefinitionRecord) => void;
    vi.mocked(api.automationGet).mockReturnValue(new Promise((done) => { resolve = done; }));
    const onApply = vi.fn();
    const props = {kind:"nurture" as const,target:{type:"all" as const},defaultName:"Nuôi TikTok",onApply};
    const view = render(<AutomationProfileControl {...props} config={{numVideos:3}} />);
    fireEvent.change(await screen.findByLabelText("Hồ sơ Nuôi TikTok"), {target:{value:"profile-1"}});
    view.rerender(<AutomationProfileControl {...props} config={{numVideos:8}} />);
    resolve(saved);
    await waitFor(() => expect(screen.getByLabelText("Hồ sơ Nuôi TikTok")).toBeEnabled());
    expect(onApply).not.toHaveBeenCalled();
    expect(screen.getByLabelText("Hồ sơ Nuôi TikTok")).toHaveValue("");
  });
});
