import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { NurturePopup } from "./NurturePopup";
import type { NurtureSettings } from "../types";

/**
 * Render tests for the redesigned panel, and they exist because a screenshot could not
 * prove it. The panel's body scrolls, so the lower half of the "Hành vi" tab — the human
 * rhythm switches and the bundle field — is off-screen in any capture, and the driver has
 * no scroll. A render assertion sees the whole tree, and unlike a screenshot it keeps
 * holding.
 */

/**
 * A note on the label queries below.
 *
 * Several captions are followed by the `!` explanation glyph, which is `aria-hidden` — a
 * browser's accessible-name computation skips `aria-hidden` subtrees, so the field's real
 * name is unchanged. Testing Library's `getByLabelText` is simpler than that: for a
 * wrapping `<label>` it matches raw `textContent`, glyph included. Hence the anchored
 * regexes. The property that matters in a browser is asserted directly instead, in
 * "explains every control behind a `!`".
 */
const saved = vi.hoisted(() => ({ saveSettings: vi.fn() }));

const settings: NurtureSettings = {
  baseUrl: "https://api.deepseek.com",
  model: "deepseek-v4-flash",
  apiKey: "",
  inputPricePer1m: 1.25,
  outputPricePer1m: 10,
  bundleId: "com.ss.iphone.ugc.Ame",
  numVideos: 120,
  numRounds: 1,
  likeProb: 35,
  commentProb: 0,
  followProb: 3,
  frenzyProb: 6,
  watchMin: 3,
  watchMax: 18,
  persona: "casual",
  fatigue: true,
  timeOfDay: true,
  pauseSwipe: true,
  nightStart: 0,
  nightEnd: 0,
  recoverDelayMin: 2,
  recoverDelayMax: 4,
  staggerDelayMin: 5,
  staggerDelayMax: 15,
  commentLang: "vi",
  aiDirections: "Tự nhiên",
  maxCommentWords: 12,
  scheduleEnabled: false,
  scheduleEveryMinutes: 240,
  scheduleDurationMinutes: 150,
  scheduleUdids: [],
  likeEnabled: true,
  commentEnabled: true,
  followEnabled: true,
  frenzyEnabled: true,
  carouselEnabled: true,
  carouselMaxSlides: 12,
  carouselPortionPercent: 100,
};

vi.mock("../api", () => ({
  nurtureGetSettings: vi.fn(async () => settings),
  nurtureSaveSettings: saved.saveSettings,
  nurtureSessionStatus: vi.fn(async () => []),
  nurtureStart: vi.fn(async () => undefined),
  nurtureStop: vi.fn(async () => undefined),
  nurtureTestApi: vi.fn(async () => null),
  listenRiviuEvents: vi.fn(async () => () => undefined),
}));

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

const devices = [
  {
    udid: "mock-1",
    name: "iPhone Mock 01",
    model: "iPhone10,1",
    platform: "ios",
    osVersion: "16.7.15",
    connection: "mock",
    status: "ready",
    wdaReady: true,
  },
] as never[];

function open() {
  render(<NurturePopup devices={devices} selected={[]} onClose={() => undefined} />);
  return waitFor(() => expect(screen.getByRole("tab", { name: "Hành vi" })).toBeVisible());
}

describe("NurturePopup", () => {
  it("groups the settings into tabs and shows one group at a time", async () => {
    await open();
    // Three tabs, "Hành vi" selected first because it is what an operator tunes.
    expect(screen.getByRole("tab", { name: "Hành vi" })).toHaveAttribute("aria-selected", "true");
    expect(screen.getByRole("tab", { name: "AI" })).toHaveAttribute("aria-selected", "false");
    expect(screen.getByRole("tab", { name: "Lịch" })).toHaveAttribute("aria-selected", "false");
    // The AI group is not merely collapsed, it is not rendered — which is the point of
    // tabs over the three stacked collapsibles this replaced.
    expect(screen.queryByLabelText(/^Base URL/)).toBeNull();

    fireEvent.click(screen.getByRole("tab", { name: "AI" }));
    expect(screen.getByLabelText(/^Base URL/)).toBeVisible();
    expect(screen.getByLabelText(/^Model/)).toBeVisible();
    expect(screen.getByLabelText(/^API key/)).toBeVisible();
    expect(screen.getByLabelText(/^Tối đa từ/)).toBeVisible();
    expect(screen.getByLabelText(/^Định hướng giọng điệu/)).toBeVisible();
    expect(screen.getByRole("button", { name: /Test API/ })).toBeVisible();

    fireEvent.click(screen.getByRole("tab", { name: "Lịch" }));
    expect(screen.getByLabelText(/Lịch tự chạy/)).toBeVisible();
    expect(screen.getByLabelText(/^Mỗi \(phút\)/)).toBeVisible();
    expect(screen.getByLabelText(/^Thời lượng \(phút\)/)).toBeVisible();
  });

  it("gives every feature its own switch, separate from its percentage", async () => {
    await open();
    // The switch and the number are two controls, not one: turning a feature off must not
    // destroy the tuned percentage.
    for (const name of ["Thích", "Bình luận", "Follow", "Vuốt nhanh"]) {
      expect(screen.getByLabelText(`Bật ${name}`)).toBeChecked();
    }
    const like = screen.getByLabelText("Bật Thích");
    fireEvent.click(like);
    expect(like).not.toBeChecked();
    // …and the 35 is still there, still editable.
    expect(screen.getByLabelText("Thích phần trăm")).toHaveValue(35);
    expect(screen.getByLabelText("Thích phần trăm")).toBeEnabled();
  });

  it("folds the switches into the summary line", async () => {
    // The line under the four features has to agree with them. Reading the raw
    // percentages made it say "Thích 35% · Bỏ qua 65%" with Thích switched off — the
    // number is deliberately kept when a feature is off, so this is the one place that
    // has to apply the switch. Mirrors `NurtureSettings::into_effective`.
    await open();
    expect(screen.getByText(/^Thích 35% · Bình luận 0% · Bỏ qua 65%/)).toBeVisible();

    fireEvent.click(screen.getByLabelText("Bật Thích"));
    expect(screen.getByText(/^Thích 0% · Bình luận 0% · Bỏ qua 100%/)).toBeVisible();
    // …and the tuned 35 is still in its box, ready to come back.
    expect(screen.getByLabelText("Thích phần trăm")).toHaveValue(35);

    fireEvent.click(screen.getByLabelText("Bật Follow"));
    expect(screen.getByText(/Follow độc lập 0%/)).toBeVisible();
  });

  it("renders the carousel as one switched row with its portion", async () => {
    await open();
    expect(screen.getByLabelText("Bật vuốt ngang bài ảnh")).toBeChecked();
    // The ceiling is no longer a field: it is a safety bound in the engine, not a number an
    // operator has a reason to pick. 100% means "to the end of the post", and the traversal
    // learns where that is by watching a swipe stop changing the screen.
    expect(screen.getByLabelText("Xem bao nhiêu phần trăm bài ảnh")).toHaveValue(100);
    expect(screen.queryByLabelText("Trần số ảnh")).toBeNull();
  });

  it("exposes the human-rhythm features that the old panel never reached", async () => {
    // `fatigue`, `timeOfDay` and `pauseSwipe` have been in `NurtureSettings` from the
    // start and no version of this UI showed any of them, so an operator could not turn a
    // single one off. This is the assertion that they are reachable — and it covers the
    // part of the tab that scrolls out of any screenshot.
    await open();
    expect(screen.getByLabelText(/^Mỏi dần/)).toBeChecked();
    expect(screen.getByLabelText(/^Theo giờ trong ngày/)).toBeChecked();
    expect(screen.getByLabelText(/^Ngập ngừng khi vuốt/)).toBeChecked();
    expect(screen.getByLabelText(/^Nghỉ đêm từ/)).toHaveValue(0);
    expect(screen.getByLabelText("đến")).toHaveValue(0);
    expect(screen.getByLabelText(/Bundle TikTok/)).toHaveValue("com.ss.iphone.ugc.Ame");
  });

  it("explains every control behind a `!` instead of a wall of hint text", async () => {
    // The explanations used to be permanent paragraphs under the fields and were removed
    // for making a settings form read as documentation. They came back as one glyph per
    // control, so the assertion is that each control has one and that it says something
    // specific — a `!` with an empty or generic tooltip would be worse than none.
    await open();
    const info = (of: string) => {
      const el = document.querySelector<HTMLElement>(`[data-info="${of}"]`);
      expect(el, `no ! for ${of}`).not.toBeNull();
      return el!;
    };
    for (const name of [
      "Giới hạn video",
      "Vòng",
      "Thích",
      "Bình luận",
      "Follow",
      "Vuốt nhanh",
      "Xem min",
      "Xem max",
      "Mỏi dần",
      "Theo giờ trong ngày",
      "Ngập ngừng khi vuốt",
      "Nghỉ đêm",
      "Vuốt ngang",
      "Bundle TikTok",
    ]) {
      expect(info(name)).toHaveTextContent("!");
      // A `!` with a generic tooltip would be worse than none.
      expect(info(name).getAttribute("title")!.length).toBeGreaterThan(30);
    }
    // Decorative, and this is the assertion that keeps it so. A label's accessible name is
    // its text content, so a glyph visible to the accessibility tree renames every field it
    // sits beside — "Base URL" became "Base URL !" before this attribute was added.
    expect(info("Mỏi dần")).toHaveAttribute("aria-hidden", "true");
    for (const name of ["Thích", "Bình luận", "Follow", "Vuốt nhanh"]) {
      // The four feature rows name their switch explicitly, so their names are provably
      // untouched by the glyph rather than merely matched loosely.
      expect(screen.getByLabelText(`Bật ${name}`)).toBeInTheDocument();
      expect(screen.getByLabelText(`${name} phần trăm`)).toBeInTheDocument();
    }

    fireEvent.click(screen.getByRole("tab", { name: "AI" }));
    for (const name of ["Base URL", "Model", "API key", "Ngôn ngữ", "Tối đa từ", "Định hướng giọng điệu"]) {
      expect(info(name)).toBeVisible();
    }
    expect(screen.getByLabelText(/^Base URL/)).toBeVisible();

    fireEvent.click(screen.getByRole("tab", { name: "Lịch" }));
    for (const name of ["Lịch tự chạy", "Mỗi (phút)", "Thời lượng (phút)"]) {
      expect(info(name)).toBeVisible();
    }
    // The manual-run horizon is the one thing this field does *not* control, and that was
    // measured the hard way — so the tooltip has to say so.
    expect(info("Thời lượng (phút)").getAttribute("title")).toContain("bấm tay");
  });

  it("shows the pacing override as one switch, off by default", async () => {
    // The operator's numbers are the real numbers unless this is on. `settings` in this
    // file has no `humanLimits` key at all, which is the stored-row case: absent reads as
    // off, the same as the Rust `#[serde(default)]`.
    await open();
    const pacing = screen.getByLabelText(/^Giới hạn nhịp người/);
    expect(pacing).not.toBeChecked();
    // The tooltip has to name what it would take back, in numbers — an operator who turns
    // this on is choosing a much slower run than the percentages above suggest.
    const why = document.querySelector('[data-info="Giới hạn nhịp người"]')!.getAttribute("title")!;
    for (const fragment of ["8–16", "2 trong 5", "12–35"]) {
      expect(why).toContain(fragment);
    }

    fireEvent.click(pacing);
    expect(pacing).toBeChecked();
  });

  it("marks the fields a running session cannot pick up", async () => {
    // The badge is the UI half of `NurtureSettings::absorb_live_changes`: everything else
    // in this tab applies on the next post, and these three do not.
    await open();
    const badges = screen.getAllByText("cần chạy lại");
    expect(badges).toHaveLength(3);
    for (const badge of badges) {
      expect(badge).toHaveAttribute("title", expect.stringContaining("cần dừng và chạy lại"));
    }
  });
});
