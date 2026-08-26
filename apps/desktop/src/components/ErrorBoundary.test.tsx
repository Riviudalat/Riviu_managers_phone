import { render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ErrorBoundary } from "./ErrorBoundary";

function Throws({ thrown }: { thrown: unknown }): React.ReactNode {
  throw thrown;
}

describe("a render that throws", () => {
  beforeEach(() => {
    // React logs caught render errors through console.error. Silenced so the suite output
    // stays readable; the assertions below are what prove the boundary ran.
    vi.spyOn(console, "error").mockImplementation(() => {});
  });
  afterEach(() => {
    vi.restoreAllMocks();
  });

  /**
   * **The screen must say something, because before this it said nothing.**
   *
   * With no boundary in the tree, React unmounts everything on a render throw and the operator
   * gets a white window — no message, no toast, no log line. That is the whole defect.
   */
  it("shows the message instead of unmounting the app to a blank page", () => {
    render(
      <ErrorBoundary>
        <Throws thrown={new Error("deviceControlBegin is not a function")} />
      </ErrorBoundary>,
    );

    expect(screen.getByRole("alert")).toBeInTheDocument();
    expect(screen.getByText(/deviceControlBegin is not a function/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Tải lại/ })).toBeInTheDocument();
  });

  /**
   * A thrown Tauri error object must not read as `[object Object]`.
   *
   * A render can throw the same `{ code, message }` shape a command rejects with — an awaited
   * value used synchronously, for instance. `describeError` is what makes that readable, and
   * this pins that the boundary uses it rather than `String`.
   */
  it("describes a thrown command error by its message", () => {
    render(
      <ErrorBoundary>
        <Throws thrown={{ code: "DeviceBusy", message: "máy đang chạy nuôi" }} />
      </ErrorBoundary>,
    );

    expect(screen.getByText(/máy đang chạy nuôi/)).toBeInTheDocument();
    expect(screen.queryByText(/\[object Object\]/)).not.toBeInTheDocument();
  });

  /** The failure has to leave the screen and reach the log, with the component named. */
  it("reports the failure and names the component it came from", () => {
    const onError = vi.fn();
    render(
      <ErrorBoundary onError={onError}>
        <Throws thrown={new Error("boom")} />
      </ErrorBoundary>,
    );

    expect(onError).toHaveBeenCalledTimes(1);
    const [message, source] = onError.mock.calls[0];
    expect(message).toBe("boom");
    expect(source).toMatch(/Throws/);
  });

  /** A tree that does not throw is left completely alone. */
  it("renders its children untouched when nothing throws", () => {
    render(
      <ErrorBoundary>
        <p>Riviu Manager</p>
      </ErrorBoundary>,
    );

    expect(screen.getByText("Riviu Manager")).toBeInTheDocument();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });
});
