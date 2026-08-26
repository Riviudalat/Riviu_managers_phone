import "@testing-library/jest-dom/vitest";
import { configure } from "@testing-library/dom";
import { cleanup } from "@testing-library/react";
import { afterEach } from "vitest";

/**
 * **Unmount what a test rendered, because nothing else does.**
 *
 * `@testing-library/react` registers its own `afterEach(cleanup)` only when `afterEach` exists
 * as a *global*, and this project runs vitest without `globals: true`. So auto-cleanup never
 * armed, and the convention became "every file calls `cleanup()` itself" -- which 13 files do
 * and **14 files with `render()` do not**, several of them with a dozen tests each
 * (`DeviceContextMenu`, `DeviceFilesPopup`, `FlowInspector`, `GroupManagerPopup`,
 * `NurtureWindows`, `SettingsPanel`...).
 *
 * Those 14 pass today by luck: each test happens to query text specific enough that the
 * previous test's leftovers do not collide. The moment two tests in one file look for the same
 * role or label, the second gets `getMultipleElementsFoundError` -- found exactly that way, by
 * a new file whose fourth test asserted `queryByRole("alert")` was absent and was handed three
 * alerts from the tests above it.
 *
 * Registering it here rather than in fourteen files is the difference between a convention and
 * a guarantee: a file added tomorrow inherits it. The files that already call `cleanup()` keep
 * working -- a second call has nothing left to unmount.
 */
afterEach(cleanup);

/**
 * `waitFor`'s default is 1000 ms, and that is a **load** threshold rather than a behaviour
 * one: fifty-four files render in parallel here, and on a busy machine — twenty phones
 * streaming into the app next door, which is the normal state of this workstation — specs
 * that pass alone start failing in the full suite, and a different one each run. Three
 * different files failed on three consecutive runs on 21/08/2026 while every one of them
 * passed in isolation.
 *
 * Raising it costs nothing when the assertion is going to succeed (`waitFor` polls and
 * returns as soon as it does) and only delays a genuine failure. A flaky gate is worse than a
 * slow one: it teaches people to re-run instead of to read.
 */
configure({ asyncUtilTimeout: 5000 });

class ResizeObserverStub implements ResizeObserver {
  observe(): void {}
  unobserve(): void {}
  disconnect(): void {}
}

globalThis.ResizeObserver = ResizeObserverStub;

/**
 * jsdom implements no layout, so it ships no `scrollIntoView` at all — calling it throws
 * `is not a function` rather than doing nothing. That is a gap in the environment, not a
 * fact about the component: a log panel that keeps its newest line in view is right, and
 * guarding every such call with a `typeof` check would push the environment's shortcoming
 * into product code. Stub it once here instead.
 */
Element.prototype.scrollIntoView = function scrollIntoView(): void {};
