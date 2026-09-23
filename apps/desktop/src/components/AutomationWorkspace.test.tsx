import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { useState } from "react";
import { AutomationWorkspace } from "./AutomationWorkspace";
import type { DeviceInfo, TargetRef } from "../types";

vi.mock("./NurturePopup", () => ({ NurturePopup: ({ scopeControl }: { scopeControl: React.ReactNode }) => <>{scopeControl}</> }));
vi.mock("./InteractionPopup", () => ({ InteractionPopup: ({ scopeControl }: { scopeControl: React.ReactNode }) => <>{scopeControl}</> }));
vi.mock("../pages/PublishPage", () => ({ PublishPage: ({ scopeControl }: { scopeControl: React.ReactNode }) => <>{scopeControl}</> }));
afterEach(cleanup);
const devices = [{ udid: "a", name: "Phone A", status: "ready", platform: "android" }, { udid: "b", name: "Phone B", status: "ready", platform: "android" }] as DeviceInfo[];

it.each(["nurture", "interaction", "publish"] as const)("%s opens the full roster from an empty explicit scope", kind => {
  function Harness() {
    const [target, setTarget] = useState<TargetRef>({ type: "explicit", udids: [] });
    return <AutomationWorkspace kind={kind} devices={devices} groups={[]} selected={[]} targetRef={target}
      targetUdids={target.type === "all" ? ["a", "b"] : target.type === "explicit" ? target.udids : []}
      onTargetRefChange={setTarget} onSelectUdids={() => {}} metas={new Map()} labels={new Map([["a", "Máy A"], ["b", "Máy B"]])} />;
  }
  render(<Harness />);
  fireEvent.click(screen.getByRole("button", { name: "Chọn thiết bị" }));
  const dialog = screen.getByRole("dialog");
  const a = within(dialog).getByRole("checkbox", { name: /Máy A/ });
  expect(within(dialog).getByRole("checkbox", { name: /Máy B/ })).toBeVisible();
  fireEvent.click(a);
  expect(a).toBeChecked();
  expect(screen.getByRole("combobox", { name: "Phạm vi thiết bị" })).toHaveValue("explicit");
});
