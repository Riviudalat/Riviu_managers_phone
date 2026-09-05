import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { useState } from "react";
import { ConfirmHost } from "./components/ConfirmHost";
import { resetConfirms } from "./confirmStore";
import { hasWorkspaceDrafts, requestWorkspaceLeave, useWorkspaceDraft } from "./workspaceDraft";

afterEach(() => { cleanup(); resetConfirms(); });
function Form({ save = async () => true }: {save?: () => Promise<boolean>}) {
  const [value, setValue] = useState("initial");
  const [baseline, setBaseline] = useState("initial");
  useWorkspaceDraft({id:"fixture",label:"Thiết lập",dirty:value !== baseline,snapshotKey:value,
    save:async () => { const result = await save(); if (result) setBaseline(value); return result; },
    discard:() => setValue(baseline)});
  return <><input aria-label="Giá trị" value={value} onChange={(e) => setValue(e.target.value)} /><ConfirmHost /></>;
}
it("does not prompt after loading or restoring the original value", async () => {
  render(<Form />);
  expect(await requestWorkspaceLeave()).toBe(true);
  fireEvent.change(screen.getByLabelText("Giá trị"), {target:{value:"changed"}});
  expect(hasWorkspaceDrafts()).toBe(true);
  fireEvent.change(screen.getByLabelText("Giá trị"), {target:{value:"initial"}});
  expect(await requestWorkspaceLeave()).toBe(true);
  expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
});
it.each(["Ở lại", "Bỏ thay đổi", "Lưu"])("handles %s without silently applying other actions", async (choice) => {
  const save = vi.fn().mockResolvedValue(true);
  render(<Form save={save} />);
  fireEvent.change(screen.getByLabelText("Giá trị"), {target:{value:"changed"}});
  let decision!: Promise<boolean>;
  act(() => { decision = requestWorkspaceLeave(); });
  fireEvent.click(screen.getByRole("button", {name:choice}));
  await act(async () => { expect(await decision).toBe(choice !== "Ở lại"); });
  expect(save).toHaveBeenCalledTimes(choice === "Lưu" ? 1 : 0);
  expect(screen.getByLabelText("Giá trị")).toHaveValue(choice === "Bỏ thay đổi" ? "initial" : "changed");
});
it("stays when saving fails and keeps the draft", async () => {
  render(<Form save={async () => false} />);
  fireEvent.change(screen.getByLabelText("Giá trị"), {target:{value:"changed"}});
  let decision!: Promise<boolean>;
  act(() => { decision = requestWorkspaceLeave(); });
  fireEvent.click(screen.getByRole("button", {name:"Lưu"}));
  await act(async () => { expect(await decision).toBe(false); });
  expect(hasWorkspaceDrafts()).toBe(true);
});
it("does not authorize dropping a newer edit when a save resolves late", async () => {
  let resolve!: (value: boolean) => void;
  render(<Form save={() => new Promise((done) => { resolve = done; })} />);
  fireEvent.change(screen.getByLabelText("Giá trị"), {target:{value:"A"}});
  let decision!: Promise<boolean>;
  act(() => { decision = requestWorkspaceLeave(); });
  fireEvent.click(screen.getByRole("button", {name:"Lưu"}));
  await waitFor(() => expect(resolve).toBeTypeOf("function"));
  fireEvent.change(screen.getByLabelText("Giá trị"), {target:{value:"B"}});
  await act(async () => { resolve(true); expect(await decision).toBe(false); });
  expect(screen.getByLabelText("Giá trị")).toHaveValue("B");
});
it("shares one dialog and Escape keeps the draft", async () => {
  render(<Form />);
  fireEvent.change(screen.getByLabelText("Giá trị"), {target:{value:"changed"}});
  let first!: Promise<boolean>;
  let second!: Promise<boolean>;
  act(() => { first = requestWorkspaceLeave(); second = requestWorkspaceLeave(); });
  expect(first).toBe(second);
  fireEvent.keyDown(window, {key:"Escape"});
  await waitFor(async () => expect(await first).toBe(false));
});

it("a scoped dialog cannot authorize discarding a different dirty workspace", async () => {
  function Multiple() {
    const [first, setFirst] = useState(true);
    const [second, setSecond] = useState(true);
    useWorkspaceDraft({id:"one",label:"Một",dirty:first,snapshotKey:String(first),
      save:async () => false, discard:() => setFirst(false)});
    useWorkspaceDraft({id:"two",label:"Hai",dirty:second,snapshotKey:String(second),
      save:async () => false, discard:() => setSecond(false)});
    return <ConfirmHost />;
  }
  render(<Multiple />);
  let scoped!: Promise<boolean>;
  let global!: Promise<boolean>;
  act(() => { scoped = requestWorkspaceLeave(["one"]); global = requestWorkspaceLeave(); });
  expect(screen.getByRole("alertdialog")).toHaveTextContent("Một");
  fireEvent.click(screen.getByRole("button", {name:"Bỏ thay đổi"}));
  await act(async () => { expect(await scoped).toBe(true); });
  await waitFor(() => expect(screen.getByRole("alertdialog")).toHaveTextContent("Hai"));
  fireEvent.click(screen.getByRole("button", {name:"Ở lại"}));
  await act(async () => { expect(await global).toBe(false); });
  expect(hasWorkspaceDrafts()).toBe(true);
});
