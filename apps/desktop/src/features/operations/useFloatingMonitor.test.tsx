import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { useFloatingMonitor } from "./useFloatingMonitor";

function Fixture({ expanded = true, maximized = false, onClick = vi.fn() }) {
  const floating = useFloatingMonitor(expanded, maximized);
  return <section ref={floating.ref} style={floating.style} aria-label="Monitor">
    <header {...floating.handle}>
      <button type="button" onClick={onClick}>Title</button>
      <button type="button" data-monitor-no-drag onClick={onClick}>Control</button>
    </header>
  </section>;
}

beforeEach(() => {
  HTMLElement.prototype.setPointerCapture = vi.fn();
  Object.defineProperty(window, "innerWidth", { configurable: true, value: 1200 });
  Object.defineProperty(window, "innerHeight", { configurable: true, value: 900 });
});

afterEach(() => { cleanup(); vi.restoreAllMocks(); });

function mockBounds() {
  const panel = screen.getByRole("region", { name: "Monitor" });
  vi.spyOn(panel, "getBoundingClientRect").mockReturnValue({ left: 100, top: 100, width: 500, height: 400 } as DOMRect);
  return panel;
}

function pointerDown(target: HTMLElement, pointerId = 1) {
  fireEvent.pointerDown(target, { pointerId, button: 0, isPrimary: true, clientX: 150, clientY: 120 });
}

function pointerMove(target: HTMLElement, pointerId = 1) {
  fireEvent.pointerMove(target, { pointerId, clientX: 200, clientY: 160 });
}

it.each(["pointerUp", "pointerCancel"] as const)("finishes an uncaptured gesture on outside %s so the next drag works", (event) => {
  render(<Fixture />);
  const panel = mockBounds();
  const title = screen.getByRole("button", { name: "Title" });
  pointerDown(title);
  fireEvent.pointerMove(title, { pointerId: 1, clientX: 152, clientY: 121 });
  fireEvent[event](document.body, { pointerId: 1 });
  pointerDown(title, 2);
  pointerMove(title, 2);
  expect(panel).toHaveStyle({ left: "150px", top: "140px" });
  expect(HTMLElement.prototype.setPointerCapture).toHaveBeenCalledWith(2);
});

it("cancels a held pointer when the window loses focus", () => {
  render(<Fixture />);
  const panel = mockBounds();
  const title = screen.getByRole("button", { name: "Title" });
  pointerDown(title);
  fireEvent.blur(window);
  pointerMove(title);
  expect(panel.style.left).toBe("");
  pointerDown(title, 2);
  pointerMove(title, 2);
  expect(panel).toHaveStyle({ left: "150px", top: "140px" });
});

it("keeps a normal click, blocks drag release, and accepts later keyboard activation", () => {
  const onClick = vi.fn();
  render(<Fixture onClick={onClick} />);
  mockBounds();
  const title = screen.getByRole("button", { name: "Title" });
  pointerDown(title);
  fireEvent.pointerUp(title, { pointerId: 1 });
  fireEvent.click(title, { detail: 1 });
  expect(onClick).toHaveBeenCalledTimes(1);
  pointerDown(title, 2);
  pointerMove(title, 2);
  fireEvent.pointerUp(title, { pointerId: 2 });
  fireEvent.click(title, { detail: 1 });
  expect(onClick).toHaveBeenCalledTimes(1);
  fireEvent.click(title, { detail: 0 });
  expect(onClick).toHaveBeenCalledTimes(2);
});

it("does not move when dragging or pressing arrows on a no-drag control", () => {
  const onClick = vi.fn();
  render(<Fixture onClick={onClick} />);
  const panel = mockBounds();
  const control = screen.getByRole("button", { name: "Control" });
  pointerDown(control);
  pointerMove(control);
  fireEvent.keyDown(control, { key: "ArrowRight" });
  fireEvent.click(control, { detail: 1 });
  expect(onClick).toHaveBeenCalledTimes(1);
  expect(panel.style.left).toBe("");
});

it("ignores other pointers without ending the primary drag", () => {
  render(<Fixture />);
  const panel = mockBounds();
  const title = screen.getByRole("button", { name: "Title" });
  pointerDown(title);
  fireEvent.pointerUp(document.body, { pointerId: 2 });
  pointerMove(title, 2);
  expect(panel.style.left).toBe("");
  pointerMove(title);
  expect(panel).toHaveStyle({ left: "150px", top: "140px" });
});

it("maximizes without position overrides or dragging, retaining title clicks and restore position", () => {
  const onClick = vi.fn();
  const view = render(<Fixture onClick={onClick} />);
  const panel = mockBounds();
  const title = screen.getByRole("button", { name: "Title" });
  fireEvent.keyDown(title, { key: "ArrowRight" });
  expect(panel).toHaveStyle({ left: "124px", top: "100px" });
  view.rerender(<Fixture maximized onClick={onClick} />);
  expect(panel.style.left).toBe("");
  expect(panel.style.right).toBe("");
  pointerDown(title);
  pointerMove(title);
  fireEvent.pointerUp(title, { pointerId: 1 });
  fireEvent.keyDown(title, { key: "ArrowRight" });
  fireEvent.click(title, { detail: 1 });
  expect(onClick).toHaveBeenCalledTimes(1);
  expect(panel.style.left).toBe("");
  expect(HTMLElement.prototype.setPointerCapture).not.toHaveBeenCalled();
  view.rerender(<Fixture onClick={onClick} />);
  expect(panel).toHaveStyle({ left: "124px", top: "100px" });
});

it("abandons a pending drag when maximize changes and permits a fresh drag on restore", () => {
  const view = render(<Fixture />);
  const panel = mockBounds();
  const title = screen.getByRole("button", { name: "Title" });
  pointerDown(title);
  view.rerender(<Fixture maximized />);
  view.rerender(<Fixture />);
  pointerDown(title, 2);
  pointerMove(title, 2);
  expect(panel).toHaveStyle({ left: "150px", top: "140px" });
});

it("reclamps the floating position when expanding and resizing the viewport", () => {
  const view = render(<Fixture expanded={false} />);
  const panel = mockBounds();
  const title = screen.getByRole("button", { name: "Title" });
  pointerDown(title);
  fireEvent.pointerMove(title, { pointerId: 1, clientX: 800, clientY: 600 });
  fireEvent.pointerUp(title, { pointerId: 1 });
  expect(panel).toHaveStyle({ left: "692px", top: "492px" });
  vi.mocked(panel.getBoundingClientRect).mockReturnValue({ left: 692, top: 492, width: 850, height: 650 } as DOMRect);
  view.rerender(<Fixture expanded />);
  expect(panel).toHaveStyle({ left: "342px", top: "242px" });
  Object.defineProperty(window, "innerWidth", { configurable: true, value: 900 });
  Object.defineProperty(window, "innerHeight", { configurable: true, value: 700 });
  fireEvent(window, new Event("resize"));
  expect(panel).toHaveStyle({ left: "42px", top: "42px" });
});
