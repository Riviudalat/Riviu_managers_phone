import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import * as api from "../../api";
import type {
  AutomationDefinition,
  OrchestrationDocumentV1,
  OrchestrationRunDetail,
} from "../../types";
import { OrchestrationWorkspace } from "./OrchestrationWorkspace";

vi.mock("../../api", () => ({
  automationList: vi.fn(),
  orchestrationArchive: vi.fn(),
  orchestrationCancelRun: vi.fn(),
  orchestrationGet: vi.fn(),
  orchestrationGetRun: vi.fn(),
  orchestrationList: vi.fn(),
  orchestrationListRuns: vi.fn(),
  orchestrationReconcile: vi.fn(),
  orchestrationRun: vi.fn(),
  orchestrationSaveRevision: vi.fn(),
  orchestrationValidate: vi.fn(),
}));

vi.mock("../../confirmStore", () => ({
  requestConfirm: vi.fn().mockResolvedValue(true),
}));

const profile: AutomationDefinition = {
  id: "10000000-0000-0000-0000-000000000001",
  name: "Tương tác nhẹ",
  kind: "interaction",
  latestRevision: 4,
  archived: false,
  createdAt: "2026-09-03T00:00:00Z",
  updatedAt: "2026-09-03T00:00:00Z",
};

const document: OrchestrationDocumentV1 = {
  schemaVersion: 1,
  id: "20000000-0000-0000-0000-000000000001",
  revision: 3,
  name: "Ca buổi sáng",
  entryNodeId: "start",
  nodes: [
    { id: "start", kind: "start", position: { x: 0, y: 0 } },
    {
      id: "interaction",
      kind: "runInteraction",
      profile: { definitionId: profile.id, revision: 4 },
      position: { x: 240, y: 0 },
    },
    { id: "end", kind: "end", position: { x: 480, y: 0 } },
  ],
  edges: [
    { sourceNodeId: "start", sourcePort: "done", targetNodeId: "interaction" },
    ...(["done", "partial", "failed", "uncertain"] as const).map((sourcePort) => ({
      sourceNodeId: "interaction",
      sourcePort,
      targetNodeId: "end",
    })),
  ],
};

const detail: OrchestrationRunDetail = {
  run: {
    id: "30000000-0000-0000-0000-000000000001",
    documentId: document.id,
    documentRevision: document.revision,
    documentSha256: "a".repeat(64),
    target: {
      targetRef: { type: "group", groupId: "group-a" },
      included: [{ udid: "phone-1", alias: "Máy 1", number: 1 }],
      excluded: [],
      rosterSha256: "b".repeat(64),
    },
    nodeTargets: {},
    state: "running",
    currentNodeId: "interaction",
    errorCode: null,
    createdAt: "2026-09-03T00:00:00Z",
    updatedAt: "2026-09-03T00:00:01Z",
  },
  attempts: [],
};

describe("OrchestrationWorkspace", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(api.automationList).mockResolvedValue([]);
    vi.mocked(api.orchestrationList).mockResolvedValue([]);
    vi.mocked(api.orchestrationListRuns).mockResolvedValue([]);
    vi.mocked(api.orchestrationGetRun).mockResolvedValue(null);
  });

  it("renders an honest empty state without sample runs", async () => {
    render(<OrchestrationWorkspace onDirtyChange={vi.fn()} />);

    expect(await screen.findByText("Chưa có điều phối nào")).toBeVisible();
    expect(screen.queryByText(/mock|sample|demo/i)).not.toBeInTheDocument();
  });

  it("links both modes to panels and activates them with the horizontal keyboard pattern", async () => {
    render(<OrchestrationWorkspace onDirtyChange={vi.fn()} />);

    const setup = await screen.findByRole("tab", { name: "Thiết lập" });
    const monitor = screen.getByRole("tab", { name: "Theo dõi điều phối" });
    expect(setup).toHaveAttribute("tabindex", "0");
    expect(monitor).toHaveAttribute("tabindex", "-1");
    for (const tab of [setup, monitor]) {
      const panel = globalThis.document.getElementById(tab.getAttribute("aria-controls")!);
      expect(panel).toHaveAttribute("role", "tabpanel");
      expect(panel).toHaveAttribute("aria-labelledby", tab.id);
    }

    setup.focus();
    fireEvent.keyDown(setup, { key: "ArrowRight" });
    expect(monitor).toHaveFocus();
    expect(monitor).toHaveAttribute("aria-selected", "true");
    fireEvent.keyDown(monitor, { key: "Home" });
    expect(setup).toHaveFocus();
    expect(setup).toHaveAttribute("aria-selected", "true");
    fireEvent.keyDown(setup, { key: "End" });
    expect(monitor).toHaveFocus();
    fireEvent.keyDown(monitor, { key: "ArrowLeft" });
    expect(setup).toHaveFocus();
  });

  it("shows a retryable load error", async () => {
    vi.mocked(api.orchestrationList)
      .mockRejectedValueOnce(new Error("database unavailable"))
      .mockResolvedValueOnce([]);
    const user = userEvent.setup();
    render(<OrchestrationWorkspace onDirtyChange={vi.fn()} />);

    expect(await screen.findByRole("alert")).toHaveTextContent("database unavailable");
    await user.click(screen.getByRole("button", { name: "Thử lại" }));
    expect(await screen.findByText("Chưa có điều phối nào")).toBeVisible();
    expect(api.orchestrationList).toHaveBeenCalledTimes(2);
  });

  it("pins the selected profile revision in a new campaign node", async () => {
    vi.mocked(api.automationList).mockResolvedValue([profile]);
    const user = userEvent.setup();
    render(<OrchestrationWorkspace onDirtyChange={vi.fn()} />);

    await user.click(await screen.findByRole("button", { name: "Tạo điều phối" }));
    const add = screen.getByRole("button", { name: "Thêm Tương tác" });
    expect(add).toBeDisabled();
    await user.selectOptions(
      screen.getByRole("combobox", { name: "Chọn hồ sơ Tương tác để thêm" }),
      profile.id,
    );
    await user.click(add);

    expect(screen.getByText("Tương tác nhẹ")).toBeVisible();
    expect(screen.getByText("Bản 4")).toBeVisible();
    expect(screen.getByRole("combobox", { name: "Đích khi Hoàn tất của bước 1" })).not.toHaveValue("");
    expect(screen.getByRole("combobox", { name: "Đích khi Chưa chắc chắn của bước 1" })).not.toHaveValue("");
  });

  it("does not silently select the first profile and persists an explicit outcome route", async () => {
    vi.mocked(api.automationList).mockResolvedValue([profile]);
    vi.mocked(api.orchestrationValidate).mockResolvedValue({
      document,
      executionOrder: ["start", "interaction", "end"],
      canonicalJson: "{}",
      sha256: "a".repeat(64),
      profiles: { interaction: { definitionId: profile.id, revision: 4 } },
    });
    vi.mocked(api.orchestrationSaveRevision).mockImplementation(async (draft) => ({
      compiled: {
        document: { ...draft, revision: 1 },
        executionOrder: draft.nodes.map((node) => node.id),
        canonicalJson: "{}",
        sha256: "a".repeat(64),
        profiles: {},
      },
      createdAt: "2026-09-03T00:00:00Z",
    }));
    const user = userEvent.setup();
    render(<OrchestrationWorkspace onDirtyChange={vi.fn()} />);

    await user.click(await screen.findByRole("button", { name: "Tạo điều phối" }));
    const profileSelect = screen.getByRole("combobox", {
      name: "Chọn hồ sơ Tương tác để thêm",
    });
    expect(profileSelect).toHaveValue("");
    await user.selectOptions(profileSelect, profile.id);
    await user.click(screen.getByRole("button", { name: "Thêm Tương tác" }));
    await user.click(screen.getByRole("button", { name: "Thêm Tương tác" }));

    const interactionNodes = screen.getAllByText("Tương tác nhẹ");
    expect(interactionNodes).toHaveLength(2);
    const partialRoutes = screen.getAllByRole("combobox", { name: /Đích khi Một phần của bước/ });
    const secondInteractionId = partialRoutes[0].querySelectorAll("option")[0]?.value;
    expect(secondInteractionId).toBeTruthy();
    await user.selectOptions(partialRoutes[0], secondInteractionId!);
    await user.click(screen.getByRole("button", { name: "Lưu bản" }));

    const saved = vi.mocked(api.orchestrationSaveRevision).mock.calls[0][0];
    const campaignIds = saved.nodes
      .filter((node) => node.kind === "runInteraction")
      .map((node) => node.id);
    expect(saved.edges).toContainEqual({
      sourceNodeId: campaignIds[0],
      sourcePort: "partial",
      targetNodeId: campaignIds[1],
    });
  });

  it("keeps the newest document when an earlier open resolves last", async () => {
    let resolveFirst!: (value: Awaited<ReturnType<typeof api.orchestrationGet>>) => void;
    const first = new Promise<Awaited<ReturnType<typeof api.orchestrationGet>>>((resolve) => {
      resolveFirst = resolve;
    });
    const secondDocument: OrchestrationDocumentV1 = {
      ...document,
      id: "20000000-0000-0000-0000-000000000002",
      revision: 8,
      name: "Ca buổi tối",
    };
    const summaries = [document, secondDocument].map((item) => ({
      id: item.id,
      name: item.name,
      latestRevision: item.revision,
      archived: false,
      updatedAt: "2026-09-03T00:00:00Z",
    }));
    const record = (item: OrchestrationDocumentV1) => ({
      compiled: {
        document: item,
        executionOrder: ["start", "interaction", "end"],
        canonicalJson: "{}",
        sha256: "a".repeat(64),
        profiles: { interaction: { definitionId: profile.id, revision: 4 } },
      },
      createdAt: "2026-09-03T00:00:00Z",
    });
    vi.mocked(api.orchestrationList).mockResolvedValue(summaries);
    vi.mocked(api.orchestrationGet).mockImplementation((id) =>
      id === document.id ? first : Promise.resolve(record(secondDocument)),
    );
    const user = userEvent.setup();
    render(<OrchestrationWorkspace onDirtyChange={vi.fn()} />);

    await user.click(await screen.findByRole("button", { name: /Ca buổi sáng/ }));
    await user.click(screen.getByRole("button", { name: /Ca buổi tối/ }));
    expect(await screen.findByLabelText("Tên điều phối")).toHaveValue("Ca buổi tối");

    await act(async () => {
      resolveFirst(record(document));
      await first;
    });
    expect(screen.getByLabelText("Tên điều phối")).toHaveValue("Ca buổi tối");
  });

  it("keeps save disabled until a campaign profile exists", async () => {
    const user = userEvent.setup();
    render(<OrchestrationWorkspace onDirtyChange={vi.fn()} />);
    await user.click(await screen.findByRole("button", { name: "Tạo điều phối" }));

    await waitFor(() => {
      expect(screen.getByRole("button", { name: "Lưu bản" })).toBeDisabled();
    });
  });

  it("runs an immutable revision against the semantic target and opens monitoring", async () => {
    vi.mocked(api.automationList).mockResolvedValue([profile]);
    vi.mocked(api.orchestrationList).mockResolvedValue([{
      id: document.id,
      name: document.name,
      latestRevision: document.revision,
      archived: false,
      updatedAt: "2026-09-03T00:00:00Z",
    }]);
    vi.mocked(api.orchestrationGet).mockResolvedValue({
      compiled: {
        document,
        executionOrder: ["start", "interaction", "end"],
        canonicalJson: "{}",
        sha256: "a".repeat(64),
        profiles: { interaction: { definitionId: profile.id, revision: 4 } },
      },
      createdAt: "2026-09-03T00:00:00Z",
    });
    vi.mocked(api.orchestrationRun).mockResolvedValue(detail);
    const user = userEvent.setup();
    render(
      <OrchestrationWorkspace
        onDirtyChange={vi.fn()}
        targetRef={{ type: "group", groupId: "group-a" }}
      />,
    );

    await user.click(await screen.findByRole("button", { name: /Ca buổi sáng/ }));
    await user.click(screen.getByRole("button", { name: "Chạy điều phối" }));

    expect(api.orchestrationRun).toHaveBeenCalledWith(
      document.id,
      document.revision,
      { type: "group", groupId: "group-a" },
    );
    expect(await screen.findByText("Đang chạy")).toBeVisible();
    expect(screen.getByText("1 máy trong phạm vi đã chốt")).toBeVisible();
  });

  it("lists persisted runs and exposes explicit reconcile and cancel controls", async () => {
    vi.mocked(api.orchestrationListRuns).mockResolvedValue([detail.run]);
    vi.mocked(api.orchestrationGetRun).mockResolvedValue(detail);
    vi.mocked(api.orchestrationReconcile).mockResolvedValue(detail);
    vi.mocked(api.orchestrationCancelRun).mockResolvedValue({
      ...detail,
      run: { ...detail.run, state: "cancelled" },
    });
    const user = userEvent.setup();
    render(<OrchestrationWorkspace onDirtyChange={vi.fn()} targetRef={{ type: "all" }} />);

    await user.click(await screen.findByRole("tab", { name: "Theo dõi điều phối" }));
    await user.click(await screen.findByRole("button", { name: /Bản 3.*Đang chạy/ }));
    await user.click(screen.getByRole("button", { name: "Đối soát" }));
    expect(api.orchestrationReconcile).toHaveBeenCalledWith(detail.run.id);
    await user.click(screen.getByRole("button", { name: "Dừng điều phối" }));
    expect(api.orchestrationCancelRun).toHaveBeenCalledWith(detail.run.id);
    expect(await screen.findByText("Đã dừng")).toBeVisible();
  });

  it("polls a running selection to its terminal state without an operator reconcile", async () => {
    const done = { ...detail, run: { ...detail.run, state: "done" as const } };
    vi.mocked(api.orchestrationListRuns).mockResolvedValue([detail.run]);
    vi.mocked(api.orchestrationGetRun)
      .mockResolvedValueOnce(detail)
      .mockResolvedValue(done);
    const user = userEvent.setup();
    render(<OrchestrationWorkspace onDirtyChange={vi.fn()} targetRef={{ type: "all" }} />);

    await user.click(await screen.findByRole("tab", { name: "Theo dõi điều phối" }));
    await user.click(await screen.findByRole("button", { name: /Bản 3.*Đang chạy/ }));

    expect(await screen.findByText("Hoàn tất", {}, { timeout: 2_500 })).toBeVisible();
    expect(api.orchestrationReconcile).not.toHaveBeenCalled();
  });

  it("drops a late run detail after the operator selects another run", async () => {
    let resolveFirst!: (value: OrchestrationRunDetail) => void;
    const first = new Promise<OrchestrationRunDetail>((resolve) => {
      resolveFirst = resolve;
    });
    const second: OrchestrationRunDetail = {
      ...detail,
      run: {
        ...detail.run,
        id: "30000000-0000-0000-0000-000000000002",
        documentRevision: 4,
        target: {
          ...detail.run.target,
          included: [
            ...detail.run.target.included,
            { udid: "phone-2", alias: "Máy 2", number: 2 },
          ],
        },
      },
    };
    vi.mocked(api.orchestrationListRuns).mockResolvedValue([detail.run, second.run]);
    vi.mocked(api.orchestrationGetRun).mockImplementation((runId) =>
      runId === detail.run.id ? first : Promise.resolve(second),
    );
    const user = userEvent.setup();
    render(<OrchestrationWorkspace onDirtyChange={vi.fn()} targetRef={{ type: "all" }} />);

    await user.click(await screen.findByRole("tab", { name: "Theo dõi điều phối" }));
    const runButtons = await screen.findAllByRole("button", { name: /Bản .*Đang chạy/ });
    await user.click(runButtons[0]);
    await user.click(runButtons[1]);
    expect(await screen.findByText("2 máy trong phạm vi đã chốt")).toBeVisible();

    await act(async () => {
      resolveFirst(detail);
      await first;
    });
    expect(screen.getByText("2 máy trong phạm vi đã chốt")).toBeVisible();
  });

  it("drops a late reconcile result after the operator selects another run", async () => {
    let resolveReconcile!: (value: OrchestrationRunDetail) => void;
    const reconcile = new Promise<OrchestrationRunDetail>((resolve) => {
      resolveReconcile = resolve;
    });
    const second: OrchestrationRunDetail = {
      ...detail,
      run: {
        ...detail.run,
        id: "30000000-0000-0000-0000-000000000002",
        documentRevision: 4,
        target: {
          ...detail.run.target,
          included: [
            ...detail.run.target.included,
            { udid: "phone-2", alias: "Máy 2", number: 2 },
          ],
        },
      },
    };
    vi.mocked(api.orchestrationListRuns).mockResolvedValue([detail.run, second.run]);
    vi.mocked(api.orchestrationGetRun).mockImplementation(async (runId) =>
      runId === second.run.id ? second : detail,
    );
    vi.mocked(api.orchestrationReconcile).mockReturnValue(reconcile);
    const user = userEvent.setup();
    render(<OrchestrationWorkspace onDirtyChange={vi.fn()} targetRef={{ type: "all" }} />);

    await user.click(await screen.findByRole("tab", { name: "Theo dõi điều phối" }));
    const runButtons = await screen.findAllByRole("button", { name: /Bản .*Đang chạy/ });
    await user.click(runButtons[0]);
    await screen.findByText("1 máy trong phạm vi đã chốt");
    await user.click(screen.getByRole("button", { name: "Đối soát" }));
    await user.click(runButtons[1]);
    expect(await screen.findByText("2 máy trong phạm vi đã chốt")).toBeVisible();

    await act(async () => {
      resolveReconcile(detail);
      await reconcile;
    });
    expect(screen.getByText("2 máy trong phạm vi đã chốt")).toBeVisible();
  });
});
