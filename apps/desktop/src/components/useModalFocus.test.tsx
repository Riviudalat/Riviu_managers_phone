import { useState } from "react";
import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, expect, it, vi } from "vitest";
import { useModalFocus } from "./useModalFocus";
import { ConfirmHost } from "./ConfirmHost";
import { requestConfirm } from "../confirmStore";
import { DeviceFunctionList } from "./DeviceFunctionList";

afterEach(cleanup);

function Modal({ onClose }: { onClose: () => void }) {
  const ref = useModalFocus<HTMLDivElement>(onClose);
  return <div ref={ref} role="dialog" aria-modal="true" aria-label="Tools" tabIndex={-1}>
    <button onClick={onClose}>Close tools</button>
    <details><summary>More</summary><button>Hidden action</button></details>
    <button onClick={() => void requestConfirm({ title: "Confirm action" })}>Last action</button>
    <div style={{ display: "none" }}><button>Hidden group action</button></div>
  </div>;
}

function Harness({ closed = () => undefined }: { closed?: () => void }) {
  const [open, setOpen] = useState(false);
  return <><button onClick={() => setOpen(true)}>Open tools</button>
    {open && <Modal onClose={() => { closed(); setOpen(false); }} />}
    <ConfirmHost />
  </>;
}

it("cycles within visible controls and restores the opener after Escape", async () => {
  const user = userEvent.setup();
  render(<Harness />);
  const opener = screen.getByRole("button", { name: "Open tools" });
  await user.click(opener);
  expect(screen.getByRole("button", { name: "Close tools" })).toHaveFocus();
  await user.tab({ shift: true });
  expect(screen.getByRole("button", { name: "Last action" })).toHaveFocus();
  await user.tab();
  expect(screen.getByRole("button", { name: "Close tools" })).toHaveFocus();
  await user.keyboard("{Escape}");
  expect(screen.queryByRole("dialog")).toBeNull();
  expect(opener).toHaveFocus();
});

it("lets a nested confirmation handle Escape before the device modal", async () => {
  const user = userEvent.setup();
  const closed = vi.fn();
  render(<Harness closed={closed} />);
  await user.click(screen.getByRole("button", { name: "Open tools" }));
  const action = screen.getByRole("button", { name: "Last action" });
  await user.click(action);
  expect(screen.getByRole("alertdialog")).toBeTruthy();
  await user.keyboard("{Escape}");
  expect(screen.queryByRole("alertdialog")).toBeNull();
  expect(screen.getByRole("dialog", { name: "Tools" })).toBeTruthy();
  expect(closed).not.toHaveBeenCalled();
  expect(action).toHaveFocus();
});

it("captures the current opener each time a persistent drawer is enabled", async () => {
  function PersistentDrawer() {
    const [open, setOpen] = useState(false);
    const ref = useModalFocus<HTMLDivElement>(() => setOpen(false), open);
    return <><button onClick={() => setOpen(true)}>First opener</button><button onClick={() => setOpen(true)}>Second opener</button>
      {open && <div ref={ref} role="dialog" aria-modal="true" tabIndex={-1}><button onClick={() => setOpen(false)}>Close drawer</button></div>}
    </>;
  }
  const user = userEvent.setup();
  render(<PersistentDrawer />);
  const first = screen.getByRole("button", { name: "First opener" });
  const second = screen.getByRole("button", { name: "Second opener" });
  await user.click(first);
  await user.keyboard("{Escape}");
  expect(first).toHaveFocus();
  await user.click(second);
  await user.keyboard("{Escape}");
  expect(second).toHaveFocus();
});

it("uses the fallback when the recording stop button disappears before the modal closes", async () => {
  function RecordingHandoff() {
    const [open, setOpen] = useState(false);
    const ref = useModalFocus<HTMLDivElement>(() => setOpen(false), open, {
      restoreFocus: () => document.getElementById("phone-menu-return"),
    });
    return <>
      <button id="phone-menu-return">Phone menu</button>
      {!open && <button onClick={() => setOpen(true)}>Stop recording</button>}
      {open && <div ref={ref} role="dialog" aria-modal="true" tabIndex={-1}><button>Save recording</button></div>}
    </>;
  }
  render(<RecordingHandoff />);
  await userEvent.click(screen.getByRole("button", { name: "Stop recording" }));
  await userEvent.keyboard("{Escape}");
  expect(screen.getByRole("button", { name: "Phone menu" })).toHaveFocus();
});

it("includes only an owned device flyout in modal tab order and closes it before the modal", async () => {
  const close = vi.fn();
  function PortalModal() {
    const ref = useModalFocus<HTMLDivElement>(close);
    return <><div ref={ref} role="dialog" aria-modal="true" aria-label="Device tools" tabIndex={-1}>
      <button>Before menu</button>
      <DeviceFunctionList platform="android" showSearch={false} nodes={[{ id: "tools", label: "More tools", children: [
        { id: "first", label: "First tool", run: vi.fn() },
        { id: "second", label: "Second tool", run: vi.fn() },
      ] }]} />
      <button>After menu</button>
    </div><button>Unrelated background control</button></>;
  }
  const user = userEvent.setup();
  render(<PortalModal />);
  await user.tab();
  const anchor = screen.getByRole("button", { name: "More tools" });
  expect(anchor).toHaveFocus();
  const first = screen.getByRole("menuitem", { name: "First tool" });
  const second = screen.getByRole("menuitem", { name: "Second tool" });
  expect(screen.getByRole("dialog").contains(first)).toBe(false);
  await user.tab();
  expect(first).toHaveFocus();
  await user.tab();
  expect(second).toHaveFocus();
  await user.tab();
  expect(screen.getByRole("button", { name: "After menu" })).toHaveFocus();
  await user.tab({ shift: true });
  expect(second).toHaveFocus();
  await user.keyboard("{Escape}");
  expect(screen.queryByRole("menu")).toBeNull();
  expect(anchor).toHaveFocus();
  expect(close).not.toHaveBeenCalled();
  await user.keyboard("{Escape}");
  expect(close).toHaveBeenCalledOnce();
});
