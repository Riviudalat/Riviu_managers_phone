import "@testing-library/jest-dom/vitest";
import { configure } from "@testing-library/dom";

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
