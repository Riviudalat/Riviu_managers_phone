import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import * as api from "../api";
import { requestConfirm } from "../confirmStore";
import type { AutomationDefinition, AutomationSchedule } from "../types";
import { AutomationScheduleControl } from "./AutomationScheduleControl";

vi.mock("../api", () => ({
  automationScheduleCreate: vi.fn(),
  automationScheduleList: vi.fn(),
  automationScheduleUpdate: vi.fn(),
}));

vi.mock("../confirmStore", () => ({ requestConfirm: vi.fn() }));

const profile: AutomationDefinition = {
  id: "profile-1",
  name: "Ca sáng",
  kind: "nurture",
  latestRevision: 3,
  archived: false,
  createdAt: "2026-09-04T00:00:00Z",
  updatedAt: "2026-09-04T00:00:00Z",
};

const pinnedSchedule: AutomationSchedule = {
  id: "schedule-1",
  revision: 7,
  name: "Mỗi giờ",
  definitionId: profile.id,
  definitionRevision: 2,
  enabled: true,
  schedule: { schemaVersion: 1, kind: "interval", everyMinutes: 60 },
  nextDueAt: "2026-09-04T01:00:00Z",
  lastErrorCode: null,
  createdAt: "2026-09-04T00:00:00Z",
  updatedAt: "2026-09-04T00:00:00Z",
};

const secondProfile: AutomationDefinition = {
  ...profile,
  id: "profile-2",
  name: "Ca chiều",
};

const secondSchedule: AutomationSchedule = {
  ...pinnedSchedule,
  id: "schedule-2",
  name: "Mỗi hai giờ",
  definitionId: secondProfile.id,
};

function revisedSchedule(patch: Partial<AutomationSchedule> = {}): AutomationSchedule {
  return { ...pinnedSchedule, revision: pinnedSchedule.revision + 1, ...patch };
}

describe("AutomationScheduleControl", () => {
  beforeEach(() => {
    vi.resetAllMocks();
    vi.mocked(api.automationScheduleList).mockResolvedValue([]);
    vi.mocked(api.automationScheduleCreate).mockResolvedValue({
      ...pinnedSchedule,
      id: "schedule-created",
      revision: 1,
      name: "Lịch buổi sáng",
      definitionRevision: profile.latestRevision,
      schedule: { schemaVersion: 1, kind: "interval", everyMinutes: 30 },
    });
    vi.mocked(api.automationScheduleUpdate).mockImplementation(
      async (id, expectedRevision, name, definitionId, definitionRevision, enabled, schedule) => ({
        ...pinnedSchedule,
        id,
        revision: expectedRevision + 1,
        name,
        definitionId,
        definitionRevision,
        enabled,
        schedule,
        updatedAt: "2026-09-04T00:01:00Z",
      }),
    );
    vi.mocked(requestConfirm).mockResolvedValue(true);
  });

  afterEach(cleanup);

  it("shows loading, empty and data states only for the selected saved profile", async () => {
    let finish!: (value: AutomationSchedule[]) => void;
    vi.mocked(api.automationScheduleList).mockReturnValueOnce(
      new Promise((resolve) => {
        finish = resolve;
      }),
    );
    const { rerender } = render(<AutomationScheduleControl profile={null} />);
    expect(screen.queryByLabelText("Lịch tự động")).not.toBeInTheDocument();

    rerender(<AutomationScheduleControl profile={profile} />);
    expect(screen.getByRole("status")).toHaveTextContent("Đang tải lịch");
    finish([]);
    expect(await screen.findByText("Chưa có lịch cho hồ sơ này")).toBeVisible();

    vi.mocked(api.automationScheduleList).mockResolvedValueOnce([
      pinnedSchedule,
      { ...pinnedSchedule, id: "other", definitionId: "profile-2", name: "Không liên quan" },
    ]);
    fireEvent.click(screen.getByRole("button", { name: "Tải lại lịch" }));
    expect(await screen.findByRole("group", { name: "Lịch Mỗi giờ" })).toBeVisible();
    expect(screen.queryByText("Không liên quan")).not.toBeInTheDocument();
  });

  it("ignores a late schedule list after another profile is selected", async () => {
    let resolveFirst!: (value: AutomationSchedule[]) => void;
    const first = new Promise<AutomationSchedule[]>((resolve) => {
      resolveFirst = resolve;
    });
    vi.mocked(api.automationScheduleList)
      .mockReturnValueOnce(first)
      .mockResolvedValueOnce([secondSchedule]);
    const { rerender } = render(<AutomationScheduleControl profile={profile} />);
    await waitFor(() => expect(api.automationScheduleList).toHaveBeenCalledTimes(1));

    rerender(<AutomationScheduleControl profile={secondProfile} />);
    expect(await screen.findByRole("group", { name: "Lịch Mỗi hai giờ" })).toBeVisible();

    await act(async () => {
      resolveFirst([pinnedSchedule]);
      await first;
    });
    expect(screen.getByRole("group", { name: "Lịch Mỗi hai giờ" })).toBeVisible();
    expect(screen.queryByRole("group", { name: "Lịch Mỗi giờ" })).not.toBeInTheDocument();
  });

  it("creates a typed interval schedule pinned to the selected profile revision", async () => {
    render(<AutomationScheduleControl profile={profile} />);
    await screen.findByText("Chưa có lịch cho hồ sơ này");

    fireEvent.change(screen.getByLabelText("Tên lịch mới"), {
      target: { value: "Lịch buổi sáng" },
    });
    fireEvent.change(screen.getByLabelText("Chu kỳ lịch mới (phút)"), {
      target: { value: "30" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Tạo lịch" }));

    await waitFor(() =>
      expect(api.automationScheduleCreate).toHaveBeenCalledWith(
        "Lịch buổi sáng",
        profile.id,
        3,
        true,
        { schemaVersion: 1, kind: "interval", everyMinutes: 30 },
      ),
    );
    expect(await screen.findByText("Đã tạo lịch ở bản hồ sơ 3.")).toBeVisible();
  });

  it("rejects cadence outside 15 through 1440 minutes before calling the API", async () => {
    render(<AutomationScheduleControl profile={profile} />);
    await screen.findByText("Chưa có lịch cho hồ sơ này");
    fireEvent.change(screen.getByLabelText("Tên lịch mới"), { target: { value: "Sai" } });
    fireEvent.change(screen.getByLabelText("Chu kỳ lịch mới (phút)"), {
      target: { value: "14" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Tạo lịch" }));

    expect(await screen.findByRole("alert")).toHaveTextContent("từ 15 đến 1.440 phút");
    expect(api.automationScheduleCreate).not.toHaveBeenCalled();
  });

  it("edits cadence and name with CAS while retaining the pinned old profile revision", async () => {
    vi.mocked(api.automationScheduleList).mockResolvedValue([pinnedSchedule]);
    render(<AutomationScheduleControl profile={profile} />);
    const row = await screen.findByRole("group", { name: "Lịch Mỗi giờ" });

    fireEvent.change(within(row).getByLabelText("Tên lịch Mỗi giờ"), {
      target: { value: "Hai giờ" },
    });
    fireEvent.change(within(row).getByLabelText("Chu kỳ Mỗi giờ (phút)"), {
      target: { value: "120" },
    });
    fireEvent.click(within(row).getByRole("button", { name: "Lưu lịch Mỗi giờ" }));

    await waitFor(() =>
      expect(api.automationScheduleUpdate).toHaveBeenCalledWith(
        pinnedSchedule.id,
        7,
        "Hai giờ",
        profile.id,
        2,
        true,
        { schemaVersion: 1, kind: "interval", everyMinutes: 120 },
      ),
    );
  });

  it("switches a schedule to the current profile revision only after explicit confirmation", async () => {
    vi.mocked(api.automationScheduleList).mockResolvedValue([pinnedSchedule]);
    vi.mocked(api.automationScheduleUpdate).mockResolvedValue(
      revisedSchedule({ definitionRevision: profile.latestRevision }),
    );
    const user = userEvent.setup();
    render(<AutomationScheduleControl profile={profile} />);
    const row = await screen.findByRole("group", { name: "Lịch Mỗi giờ" });

    await user.click(within(row).getByRole("button", { name: "Áp dụng bản hồ sơ 3" }));
    expect(requestConfirm).toHaveBeenCalledWith(
      expect.objectContaining({
        title: "Chuyển lịch sang bản hồ sơ 3?",
        confirmLabel: "Áp dụng bản 3",
      }),
    );
    await waitFor(() =>
      expect(api.automationScheduleUpdate).toHaveBeenCalledWith(
        pinnedSchedule.id,
        7,
        pinnedSchedule.name,
        profile.id,
        3,
        true,
        pinnedSchedule.schedule,
      ),
    );

    vi.mocked(requestConfirm).mockResolvedValueOnce(false);
    vi.mocked(api.automationScheduleList).mockResolvedValueOnce([pinnedSchedule]);
    fireEvent.click(screen.getByRole("button", { name: "Tải lại lịch" }));
    const reloaded = await screen.findByRole("group", { name: "Lịch Mỗi giờ" });
    await user.click(within(reloaded).getByRole("button", { name: "Áp dụng bản hồ sơ 3" }));
    expect(api.automationScheduleUpdate).toHaveBeenCalledTimes(1);
  });

  it("enables and disables with the current schedule CAS revision", async () => {
    vi.mocked(api.automationScheduleList).mockResolvedValue([pinnedSchedule]);
    vi.mocked(api.automationScheduleUpdate).mockResolvedValueOnce(
      revisedSchedule({ enabled: false }),
    );
    render(<AutomationScheduleControl profile={profile} />);
    const row = await screen.findByRole("group", { name: "Lịch Mỗi giờ" });
    fireEvent.click(within(row).getByRole("button", { name: "Tắt lịch Mỗi giờ" }));

    await waitFor(() =>
      expect(api.automationScheduleUpdate).toHaveBeenCalledWith(
        pinnedSchedule.id,
        7,
        pinnedSchedule.name,
        profile.id,
        2,
        false,
        pinnedSchedule.schedule,
      ),
    );
    expect(await screen.findByText("Đang tắt")).toBeVisible();
  });

  it("labels an unknown schedule without interpreting or mutating it", async () => {
    const future = {
      ...pinnedSchedule,
      schedule: { schemaVersion: 2, kind: "calendar", at: "08:00" },
    };
    vi.mocked(api.automationScheduleList).mockResolvedValue([future]);
    render(<AutomationScheduleControl profile={profile} />);

    const row = await screen.findByRole("group", { name: "Lịch Mỗi giờ" });
    expect(within(row).getByRole("alert")).toHaveTextContent(
      "Định dạng lịch này chưa được hỗ trợ",
    );
    expect(within(row).getByLabelText("Chu kỳ Mỗi giờ (phút)")).toBeDisabled();
    expect(within(row).getByRole("button", { name: "Tắt lịch Mỗi giờ" })).toBeDisabled();
    expect(within(row).getByRole("button", { name: "Áp dụng bản hồ sơ 3" })).toBeDisabled();
    expect(api.automationScheduleUpdate).not.toHaveBeenCalled();
  });

  it("recovers from load and CAS failures through accessible retry controls", async () => {
    vi.mocked(api.automationScheduleList)
      .mockRejectedValueOnce(new Error("schedule database unavailable"))
      .mockResolvedValue([pinnedSchedule]);
    vi.mocked(api.automationScheduleUpdate)
      .mockRejectedValueOnce(new Error("schedule revision conflict"))
      .mockResolvedValueOnce(revisedSchedule({ enabled: false }));
    const user = userEvent.setup();
    render(<AutomationScheduleControl profile={profile} />);

    expect(await screen.findByRole("alert")).toHaveTextContent("schedule database unavailable");
    await user.click(screen.getByRole("button", { name: "Thử tải lại lịch" }));
    const row = await screen.findByRole("group", { name: "Lịch Mỗi giờ" });
    await user.click(within(row).getByRole("button", { name: "Tắt lịch Mỗi giờ" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("schedule revision conflict");
    await user.click(screen.getByRole("button", { name: "Tải lại sau xung đột" }));
    await waitFor(() => expect(api.automationScheduleList).toHaveBeenCalledTimes(3));

    const details = screen.getByRole("group", { name: "Chi tiết kỹ thuật lịch Mỗi giờ" });
    expect(details).not.toHaveAttribute("open");
    await user.click(within(details).getByText("Chi tiết"));
    expect(details).toHaveAttribute("open");
    expect(details).toHaveTextContent("schedule-1");
  });
});
