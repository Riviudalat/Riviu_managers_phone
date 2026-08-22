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
  baseUrl: "https://openrouter.ai/api/v1",
  model: "openai/gpt-5.6-luna",
  apiKey: "",
  inputPricePer1m: 0.1,
  outputPricePer1m: 0.6,
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

// The whole point of the Test API fix: the frames come from the WebView's decoder, not
// from the host's JPEG hub. `burst` is what the popup is expected to call, and the frames
// it returns are what must reach `nurtureTestApi`.
const burst = vi.fn(async () => [new Uint8Array([0xff, 0xd8, 0xff, 0x01])]);
vi.mock("../viewStore", () => ({
  exportViewJpegBurst: (...args: unknown[]) => burst(...(args as [])),
}));

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

/** Opens the panel with the four interaction rates set to these values, rest unchanged. */
async function openWithRates(like: number, comment: number, follow: number, frenzy: number) {
  const api = await import("../api");
  vi.mocked(api.nurtureGetSettings).mockResolvedValueOnce({
    ...settings,
    likeProb: like,
    commentProb: comment,
    followProb: follow,
    frenzyProb: frenzy,
  });
  await open();
}

const slider = (name: string) => screen.getByLabelText(`${name} thanh kéo phần trăm`);
const box = (name: string) => screen.getByLabelText(`${name} phần trăm`);

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
      expect(info(name).getAttribute("data-tip")!.length).toBeGreaterThan(30);
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
    expect(info("Thời lượng (phút)").getAttribute("data-tip")).toContain("bấm tay");
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
    const why = document.querySelector('[data-info="Giới hạn nhịp người"]')!.getAttribute("data-tip")!;
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
      expect(badge).toHaveAttribute("data-tip", expect.stringContaining("Bắt đầu lại"));
    }
  });

  it("tests the API against the frames the WebView already decoded", async () => {
    // Test API read only the host's JPEG hub. Android phones stopped publishing there when
    // the H.264 view path landed, so pressing this while watching a phone's live picture
    // answered "Chưa có frame stream cho thiết bị …" -- true about the hub and false about
    // the phone in front of the operator.
    const api = await import("../api");
    await open();
    fireEvent.click(screen.getByRole("tab", { name: "AI" }));
    fireEvent.click(screen.getByRole("button", { name: /Test API/ }));

    await waitFor(() => expect(api.nurtureTestApi).toHaveBeenCalled());
    expect(burst).toHaveBeenCalledWith("mock-1");
    expect(vi.mocked(api.nurtureTestApi).mock.calls[0]).toEqual([
      "mock-1",
      [new Uint8Array([0xff, 0xd8, 0xff, 0x01])],
    ]);
  });
});

/**
 * The four interaction rates share one 100% budget, dragged rather than typed.
 *
 * The arithmetic itself is proved in `nurtureBudget.test.ts` against the pure module. What
 * needs a render is the wiring: that each row got a slider, that a slider's `max` is that
 * row's ceiling and not a flat 100, that dragging clamps instead of stealing from a
 * neighbour, and that a config already over the budget can still be brought back.
 */
describe("the shared 100% budget", () => {
  it("tells every rate what the other three leave free, without rescaling it", async () => {
    // 35 + 0 + 3 + 6 = 44 spent, so Thích may reach 100 - 9 = 91 and no further.
    await open();
    expect(slider("Thích")).toHaveAttribute("data-ceiling", "91");
    expect(slider("Bình luận")).toHaveAttribute("data-ceiling", "56");
    expect(slider("Follow")).toHaveAttribute("data-ceiling", "59");
    expect(slider("Vuốt nhanh")).toHaveAttribute("data-ceiling", "62");
    expect(screen.getByText("Còn 56% / 100%")).toBeVisible();

    // The ceiling is drawn on the track and enforced on change; it is NOT the slider's `max`.
    // With `max` set to the ceiling, dragging one row rescaled the others and their thumbs
    // slid across the track while their numbers never moved — see "one row's thumb" below.
    for (const name of ["Thích", "Bình luận", "Follow", "Vuốt nhanh"]) {
      expect(slider(name)).toHaveAttribute("max", "100");
    }
  });

  it("lets the other three share what one leaves, to the last point", async () => {
    // The operator's own example: one at 90 leaves 10, three at 3 leaves one able to reach 4.
    await openWithRates(90, 3, 3, 0);
    expect(slider("Vuốt nhanh")).toHaveAttribute("data-ceiling", "4");

    fireEvent.change(slider("Vuốt nhanh"), { target: { value: "4" } });
    expect(box("Vuốt nhanh")).toHaveValue(4);
    expect(screen.getByText("Còn 0% / 100%")).toBeVisible();
    // One point further is refused, which is the clamp and not the input's own range.
    fireEvent.change(slider("Vuốt nhanh"), { target: { value: "5" } });
    expect(box("Vuốt nhanh")).toHaveValue(4);
  });

  it("moves one row's thumb only when that row's own number moves", async () => {
    // The bug the operator reported: "kéo thanh thứ 2 thì thanh 1 cũng bị kéo theo". Follow
    // sat at 3 on a scale of 0..3, so its thumb was hard right; freeing 49 points rescaled it
    // to 0..52 and the thumb slid left on its own. The number was right the whole time, which
    // is what made it a lie rather than an error — the control moved without being edited.
    await openWithRates(97, 0, 3, 0);
    const followThumb = () => slider("Follow").style.getPropertyValue("--fill");
    const before = followThumb();
    expect(before).toBe("0.03");

    fireEvent.change(slider("Thích"), { target: { value: "48" } });
    expect(box("Thích")).toHaveValue(48);
    // Follow's ceiling grew — that is real and it is drawn — but its own position did not.
    expect(slider("Follow")).toHaveAttribute("data-ceiling", "52");
    expect(followThumb()).toBe(before);
    expect(box("Follow")).toHaveValue(3);
  });

  it("clamps a drag past the free amount instead of taking it off a neighbour", async () => {
    await open();
    fireEvent.change(slider("Thích"), { target: { value: "95" } });
    // Stopped at its ceiling…
    expect(box("Thích")).toHaveValue(91);
    // …and the three the operator did not touch are exactly as they were. A budget that
    // rebalances neighbours behind the operator's back destroys numbers they tuned.
    expect(box("Bình luận")).toHaveValue(0);
    expect(box("Follow")).toHaveValue(3);
    expect(box("Vuốt nhanh")).toHaveValue(6);
  });

  it("keeps the slider and the number box showing the same value", async () => {
    // Two controls over one number: either may move it, and neither may lie about it.
    await open();
    fireEvent.change(slider("Follow"), { target: { value: "20" } });
    expect(box("Follow")).toHaveValue(20);
    fireEvent.change(box("Follow"), { target: { value: "12" } });
    expect(slider("Follow")).toHaveValue("12");
  });

  it("offers a way back for a config saved before the budget existed", async () => {
    // The measured shape of the operator's own settings: 100 + 28 + 3 + 0 = 131. Every
    // ceiling is 0 in that state, so without the button below no slider moves and the panel
    // is a dead end.
    await openWithRates(100, 28, 3, 0);
    expect(screen.getByRole("alert")).toHaveTextContent("cộng lại đang là 131%");
    expect(screen.getByText("Đang dùng 131% / 100%")).toBeVisible();

    fireEvent.click(screen.getByRole("button", { name: "đưa về 100%" }));
    // Taken off the largest, so the two small tuned rates survive.
    expect(box("Thích")).toHaveValue(69);
    expect(box("Bình luận")).toHaveValue(28);
    expect(box("Follow")).toHaveValue(3);
    expect(screen.queryByRole("alert")).toBeNull();
    expect(screen.getByText("Còn 0% / 100%")).toBeVisible();
  });

  it("refuses to save an over-budget config, and says by how much", async () => {
    const api = await import("../api");
    await openWithRates(100, 28, 3, 0);
    fireEvent.click(screen.getByRole("button", { name: "Lưu" }));

    await waitFor(() =>
      expect(screen.getByText(/dùng chung 100%, đang là 131%/)).toBeVisible(),
    );
    expect(api.nurtureSaveSettings).not.toHaveBeenCalled();
  });
});

/**
 * A feature the operator switched off.
 *
 * The engine folds the switch into the probability — `into_effective` zeroes `like_prob` when
 * `like_enabled` is false — so a switched-off rate produces nothing and must therefore cost
 * nothing. Charging it would reserve budget for posts that provably never happen.
 */
describe("a rate the operator switched off", () => {
  it("gives its percent back to the other three the moment it is switched off", async () => {
    await open();
    // 35 + 0 + 3 + 6 = 44.
    expect(screen.getByText("Còn 56% / 100%")).toBeVisible();

    fireEvent.click(screen.getByLabelText("Bật Vuốt nhanh"));
    expect(screen.getByText("Còn 62% / 100%")).toBeVisible();
    // Its 6 is kept, not zeroed — that is the whole point of a switch beside a number.
    expect(box("Vuốt nhanh")).toHaveValue(6);
    // And the freed 6 is now reachable by a rate that is on.
    expect(slider("Thích")).toHaveAttribute("data-ceiling", "97");
  });

  it("stays draggable while off, past what is left free", async () => {
    // 100 spent by Thích alone, so switching Follow off leaves nothing for it. Held to the
    // budget it would be frozen at 0, and an operator who switched a feature off to come back
    // to it later would find the number they were protecting taken away.
    await openWithRates(100, 0, 3, 0);
    fireEvent.click(screen.getByLabelText("Bật Follow"));
    expect(screen.getByText("Còn 0% / 100%")).toBeVisible();
    expect(slider("Follow")).toHaveAttribute("data-ceiling", "0");

    fireEvent.change(slider("Follow"), { target: { value: "40" } });
    expect(box("Follow")).toHaveValue(40);
    // It is parked, not spent: the readout does not move and nothing is over budget.
    expect(screen.getByText("Còn 0% / 100%")).toBeVisible();
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("says so when switching it back on no longer fits, instead of trimming it", async () => {
    await openWithRates(97, 0, 3, 0);
    const followSwitch = screen.getByLabelText("Bật Follow");
    fireEvent.click(followSwitch);
    fireEvent.change(slider("Follow"), { target: { value: "40" } });
    fireEvent.click(followSwitch);

    expect(screen.getByRole("alert")).toHaveTextContent("cộng lại đang là 137%");
    // The number the operator just parked is still 40. Trimming the row they had only asked
    // to *enable* would be the panel editing a tuned number behind them.
    expect(box("Follow")).toHaveValue(40);
    expect(box("Thích")).toHaveValue(97);
  });

  it("does not demand an API key for comments it will never post", async () => {
    // `settings.apiKey` is "" in this fixture. With the switch off the run cannot comment, so
    // refusing the save over a missing key was refusing it over a feature that will not run.
    const api = await import("../api");
    await openWithRates(30, 20, 0, 0);
    fireEvent.click(screen.getByRole("button", { name: "Lưu" }));
    await waitFor(() =>
      expect(screen.getByText(/điền API key trong Cấu hình AI/)).toBeVisible(),
    );
    expect(api.nurtureSaveSettings).not.toHaveBeenCalled();

    fireEvent.click(screen.getByLabelText("Bật Bình luận"));
    fireEvent.click(screen.getByRole("button", { name: "Lưu" }));
    await waitFor(() => expect(api.nurtureSaveSettings).toHaveBeenCalled());
    // The 20 is still on the wire; only the switch says not to use it.
    expect(vi.mocked(api.nurtureSaveSettings).mock.calls[0][0]).toMatchObject({
      commentProb: 20,
      commentEnabled: false,
    });
  });
});
