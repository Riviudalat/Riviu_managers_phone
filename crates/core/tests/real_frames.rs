//! Screen classification against frames captured from the live iPhone 8.
//!
//! The synthetic scenes in `screen.rs` pin the geometry; these pin the
//! thresholds against real MJPEG output — JPEG artefacts, video content behind
//! the chrome, and the compression the device applies at quality 55.
//!
//! Capture more with `RIVIU_FRAME_DUMP=<dir>` during a live run, or with
//! `cargo run -p riviu-managers-phone --bin capture_frames` when the case you
//! need does not change classification (a dump only writes on a class change,
//! so it can never produce two frames of the same feed card).

use std::path::Path;
use std::time::Instant;

use riviu_core::screen::{self, CLOSE_X_THRESHOLD};
use riviu_core::ScreenKind;

/// WDA logical width of the device the fixtures came from.
const LOGICAL_W: f64 = 375.0;

fn load(name: &str) -> image::RgbImage {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    image::open(&path)
        .unwrap_or_else(|e| panic!("open {}: {e}", path.display()))
        .to_rgb8()
}

fn feed_frames() -> Vec<(&'static str, image::RgbImage)> {
    vec![
        ("feed-iphone8.jpg", load("feed-iphone8.jpg")),
        ("feed-iphone8-b.jpg", load("feed-iphone8-b.jpg")),
    ]
}

#[test]
fn real_feed_frames_are_classified_as_the_feed() {
    for (name, img) in feed_frames() {
        assert_eq!(
            img.width(),
            750,
            "{name} is not the @2x capture we measured"
        );
        let obs = screen::classify(&img, Some(LOGICAL_W));
        assert_eq!(obs.kind, ScreenKind::Feed, "{name}: {}", obs.debug_line());
    }
}

#[test]
fn the_close_button_does_not_match_a_real_feed() {
    for (name, img) in feed_frames() {
        let found = screen::find_close_button(&img, Some(LOGICAL_W));
        let score = found.map(|(_, _, s)| s).unwrap_or(0.0);
        assert!(
            score < CLOSE_X_THRESHOLD,
            "{name}: feed scored {score:.3} against the ✕ template \
             (threshold {CLOSE_X_THRESHOLD}) — margin has eroded"
        );
    }
}

/// An Add-phone style sheet pasted over a *real* feed frame: the sheet hides
/// the compose bar, and the ✕ must be found at the pixel it was pasted at.
#[test]
fn a_sheet_over_a_real_feed_is_found_and_located() {
    let mut img = load("feed-iphone8.jpg");
    let (w, h) = (img.width(), img.height());
    for y in 700..h {
        for x in 0..w {
            img.put_pixel(x, y, image::Rgb([252, 252, 252]));
        }
    }
    let glyph =
        image::open(Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/tiktok_close_x.png"))
            .unwrap()
            .to_rgb8();
    image::imageops::overlay(&mut img, &glyph, 673, 735);

    let obs = screen::classify(&img, Some(LOGICAL_W));
    match obs.kind {
        ScreenKind::ClosableSheet { x, y, score } => {
            // Centre of a 40×40 glyph pasted at (673, 735) is (693, 755).
            assert!((x * w as f64 - 693.0).abs() < 8.0, "x {x} ({score:.3})");
            assert!((y * h as f64 - 755.0).abs() < 8.0, "y {y} ({score:.3})");
        }
        other => panic!(
            "expected a closable sheet, got {other:?} ({})",
            obs.debug_line()
        ),
    }
}

/// Stream coordinates must survive the trip to WDA points. The watcher taps at
/// `fraction × logical size`, so a ✕ at pixel 693 of a 750-wide capture has to
/// land on point 346.5 of a 375-point screen.
#[test]
fn frame_pixels_map_onto_wda_points() {
    let img = load("feed-iphone8.jpg");
    let (w, h) = (img.width() as f64, img.height() as f64);
    for (px, py) in [(693.0, 755.0), (689.0, 620.0), (375.0, 1281.0)] {
        let (fx, fy) = (px / w, py / h);
        let point = (fx * LOGICAL_W, fy * (LOGICAL_W * h / w));
        assert!(
            (point.0 - px / 2.0).abs() < 0.5 && (point.1 - py / 2.0).abs() < 0.5,
            "({px},{py}) mapped to {point:?}, expected the @2x halving"
        );
    }
}

/// The action rail must be located from the frame, not assumed. The fixture
/// shows the follow badge at pixel (689, 525) — measured by eye off the capture
/// — and the like heart at (687, 620), i.e. the +54-logical-point offset.
#[test]
fn the_action_rail_is_located_from_a_real_frame() {
    let img = load("feed-iphone8.jpg");
    let (w, h) = (img.width() as f64, img.height() as f64);
    let rail = screen::find_action_rail(&img).expect("follow badge not found");

    assert!(rail.located);
    assert_eq!(rail.layout(), 2, "this capture is TikTok's layout 2");
    assert!(
        (rail.follow_y * h - 525.0).abs() < 12.0,
        "follow badge at {} px, expected ~525",
        rail.follow_y * h
    );
    assert!(
        (rail.like_y * h - 620.0).abs() < 14.0,
        "like heart at {} px, expected ~620",
        rail.like_y * h
    );
    assert!(
        (rail.x * w - 689.0).abs() < 12.0,
        "rail x at {} px, expected ~689",
        rail.x * w
    );
}

/// The coordinates the old code used fell between icons. Guard against ever
/// shipping those again: the previous like point (0.92, 0.42) must be far
/// enough from the located heart to be recognisably wrong.
#[test]
fn the_previous_like_coordinate_really_did_miss_the_heart() {
    let img = load("feed-iphone8.jpg");
    let h = img.height() as f64;
    let rail = screen::find_action_rail(&img).expect("follow badge");
    let old_like_y = 0.42 * h;
    let real_like_y = rail.like_y * h;
    assert!(
        (old_like_y - real_like_y).abs() > 40.0,
        "old {old_like_y} vs real {real_like_y}: the regression this test guards \
         is that likes tapped dead space between the follow badge and the heart"
    );
}

/// A feed frame is not a comment drawer.
#[test]
fn the_feed_is_not_mistaken_for_the_comment_drawer() {
    for (name, img) in feed_frames() {
        let (state, ev) = screen::comment_drawer_state(&img);
        assert_eq!(
            state,
            screen::CommentDrawer::Closed,
            "{name}: {}",
            ev.debug_line()
        );
    }
}

/// Telling an already-liked video from an unliked one is what stops the engine
/// un-liking someone's post.
///
/// This asserts the decision production actually makes —
/// `like_redness_at(img, located_rail) > LIKE_FILLED_REDNESS` — rather than the
/// hand-picked 60/45 bounds it used to, which were both looser and tighter than
/// the shipped threshold in different directions and so passed for frames the
/// engine would have decided the other way. It reads the rail the same way too:
/// `ActionRail::fallback()` is layout-2 constants, not what the engine taps.
///
/// Measured margins against the 90.0 threshold, worst case in each direction:
///
/// | frame | redness | verdict |
/// |---|---|---|
/// | `feed-heart-liked-sponsored` | 96.4 | liked, +6.4 of headroom |
/// | `feed-heart-liked` | 123.8 | liked |
/// | `feed-same-card-3` | 29.3 | unliked, 60.7 of headroom |
/// | `feed-rail-variant` | 0.6 | unliked |
#[test]
fn a_liked_heart_reads_red_and_an_unliked_one_does_not() {
    for name in ["feed-heart-liked.jpg", "feed-heart-liked-sponsored.jpg"] {
        let img = load(name);
        let rail = screen::locate_action_rail(&img).expect("a liked card still has a rail");
        let redness = screen::like_redness_at(&img, &rail);
        assert!(
            redness > screen::LIKE_FILLED_REDNESS,
            "{name}: liked heart measured {redness:.1}, below the {:.1} the engine \
             requires — it would like the post again and un-like it",
            screen::LIKE_FILLED_REDNESS
        );
    }

    for name in [
        "feed-iphone8.jpg",
        "feed-iphone8-b.jpg",
        "feed-rail-variant.png",
        "feed-same-card-1.jpg",
        "feed-same-card-2.jpg",
        "feed-same-card-3.jpg",
    ] {
        let img = load(name);
        let rail = screen::locate_action_rail(&img).expect("rail");
        let redness = screen::like_redness_at(&img, &rail);
        assert!(
            redness <= screen::LIKE_FILLED_REDNESS,
            "{name}: unliked heart measured {redness:.1}, at or above the {:.1} that \
             means 'already liked' — the like would be skipped",
            screen::LIKE_FILLED_REDNESS
        );
    }
}

/// The watcher classifies several frames a second, in debug builds too. This
/// is a guard rail, not a benchmark: a regression that puts a full pass into
/// the hundreds of milliseconds would starve the popup loop.
///
/// Timed as the *fastest* of several passes, not the mean. The mean measures
/// whatever else the machine was doing — the whole suite runs in parallel, and
/// this test failed at 416 ms against a 400 ms bar while passing at a quarter
/// of that when run alone. The fastest pass is the one that actually reflects
/// the code's cost, and a real regression raises it just the same.
///
/// That change fixed the estimator and left the bound, so the same failure came
/// back. Measured on identical code and identical input, fastest-of-five still
/// moves with machine state by 2.5×:
///
/// | how it was run | fastest of five |
/// |---|---|
/// | this test alone | 166 ms |
/// | the whole `real_frames` binary | 377 ms |
/// | `cargo test --workspace` | 424 ms |
///
/// So taking the fastest pass narrows the noise but does not remove it, and any
/// bound close to the isolated cost is a coin flip on a busy machine. See the
/// bound below for where it now sits and why.
fn fastest_classify(img: &image::RgbImage, runs: u32) -> std::time::Duration {
    (0..runs)
        .map(|_| {
            let started = Instant::now();
            let _ = screen::classify(img, Some(LOGICAL_W));
            started.elapsed()
        })
        .min()
        .expect("at least one run")
}

#[test]
fn classification_stays_fast_enough_for_the_watcher() {
    let img = load("feed-iphone8.jpg");
    // Warm the cached template so the first decode is not counted.
    let _ = screen::classify(&img, Some(LOGICAL_W));

    let per_frame = fastest_classify(&img, 5);
    println!(
        "classify: {:?}/frame (feed, compose bar visible)",
        per_frame
    );

    // The feed path short-circuits on the compose bar, so it must be very fast.
    assert!(
        per_frame < std::time::Duration::from_millis(120),
        "feed classification took {per_frame:?}/frame"
    );

    // The expensive path is a frame with no compose bar: that runs the template
    // match. Measure it explicitly so the pyramid's cost is visible.
    let mut bare = img.clone();
    for y in (0.94 * bare.height() as f64) as u32..bare.height() {
        for x in 0..bare.width() {
            bare.put_pixel(x, y, image::Rgb([10, 10, 10]));
        }
    }
    let per_frame = fastest_classify(&bare, 5);
    println!(
        "classify: {:?}/frame (no compose bar — full template match)",
        per_frame
    );
    // Sits above the machine-state spread on purpose. The worst honest observation of
    // this exact code was 424 ms, under `cargo test --workspace`; a 400 ms bound made a
    // green suite depend on how warm the laptop was. This is a debug build and a
    // regression guard, not a real-time budget — 24 FPS would be 41 ms/frame, so 1200 ms
    // is already ~29× the frame it protects. What it still catches is the kind of change
    // that matters here: losing the pyramid, or scanning at full resolution, both of
    // which cost multiples rather than percent. The printed number above is the thing to
    // watch for slow drift, since an assert can only fire once it is already too late.
    assert!(
        per_frame < std::time::Duration::from_millis(1200),
        "template-match classification took {per_frame:?}/frame"
    );
}

/// The composer's emoji grid must be found per frame: TikTok inserts a
/// "recently used" section above it after the first comment, which shifts every
/// cell. Hard-coded positions worked once and then silently missed.
#[test]
fn the_emoji_grid_is_located_in_a_real_composer_frame() {
    let img = load("composer-emoji-panel.png");
    let grid = screen::find_emoji_grid(&img);
    assert!(
        grid.len() >= 3,
        "expected several full emoji rows, found {}",
        grid.len()
    );
    for (i, row) in grid.iter().enumerate() {
        assert!(
            row.len() >= 5,
            "row {i} has only {} cells — partial rows must be dropped",
            row.len()
        );
        // Cells run left to right across the panel, inside the screen.
        for w in row.windows(2) {
            assert!(w[0].0 < w[1].0, "row {i} is not ordered left to right");
        }
        assert!(row[0].0 > 0.0 && row[row.len() - 1].0 < 1.0);
        assert!(row[0].1 > 0.55, "grid must sit in the lower panel");
    }
}

/// The send arrow is the composer's state in one number: light pink while the
/// field is empty, solid red once something is in it.
#[test]
fn the_send_arrow_tells_empty_from_armed() {
    let empty = screen::composer_send_redness(&load("composer-emoji-panel.png"));
    let armed = screen::composer_send_redness(&load("composer-armed.png"));
    assert!(
        empty < screen::SEND_ARMED_REDNESS,
        "empty composer measured {empty:.1}, threshold {}",
        screen::SEND_ARMED_REDNESS
    );
    assert!(
        armed > screen::SEND_ARMED_REDNESS,
        "armed composer measured {armed:.1}, threshold {}",
        screen::SEND_ARMED_REDNESS
    );
    assert!(
        armed - empty > 50.0,
        "margin between empty ({empty:.1}) and armed ({armed:.1}) is too thin"
    );
}

/// A SIM-less device raises "iPhone chưa được Kích hoạt" over TikTok every few
/// minutes. Until this was recognised a run stalled at zero videos: the frame
/// classified as `Unknown`, so the engine swiped forever at a dimmed backdrop.
#[test]
fn the_activation_alert_is_recognised_and_aimed_at_the_left_button() {
    let img = load("system-alert-activation.jpg");
    let obs = screen::classify(&img, Some(LOGICAL_W));
    let (x, y) = match obs.kind {
        ScreenKind::SystemAlert { x, y } => (x, y),
        other => panic!("classified as {other:?}: {}", obs.debug_line()),
    };
    // "Bỏ qua" measured at (0.320, 0.575); "Thử lại" sits at x 0.679 and must
    // never be the target.
    assert!(
        (x - 0.320).abs() < 0.05,
        "aimed at x={x:.3}, expected the left button near 0.320"
    );
    assert!(
        (y - 0.575).abs() < 0.04,
        "aimed at y={y:.3}, expected the button row near 0.575"
    );
}

/// The dim-backdrop test is what keeps this off TikTok's own white sheets,
/// which reach the screen edges and are never dimmed.
#[test]
fn no_system_alert_on_tiktok_surfaces() {
    for name in [
        "feed-iphone8.jpg",
        "feed-iphone8-b.jpg",
        "feed-rail-variant.png",
        "composer-emoji-panel.png",
        "composer-armed.png",
    ] {
        let img = load(name);
        assert!(
            screen::find_system_alert(&img).is_none(),
            "{name} matched the system-alert signature"
        );
    }
}

/// The like heart is read with an absolute threshold because the relative one
/// broke both ways on real video: a red-heavy clip lifted the baseline so a
/// genuine fill looked like no change, and an outline over red looked filled.
#[test]
fn the_filled_heart_is_separated_from_the_outline_by_an_absolute_threshold() {
    let rail = screen::ActionRail::fallback();
    let liked = screen::like_redness_at(&load("feed-heart-liked-sponsored.jpg"), &rail);
    assert!(
        liked > screen::LIKE_FILLED_REDNESS,
        "liked heart measured {liked:.1}, threshold {}",
        screen::LIKE_FILLED_REDNESS
    );

    // Every frame whose heart is an outline must stay below it, including the
    // ones whose video is red enough to lift the reading.
    for name in [
        "feed-heart-unliked-45x.jpg",
        "feed-iphone8.jpg",
        "feed-iphone8-b.jpg",
    ] {
        let v = screen::like_redness_at(&load(name), &rail);
        assert!(
            v < screen::LIKE_FILLED_REDNESS,
            "{name}: outline measured {v:.1}, threshold {}",
            screen::LIKE_FILLED_REDNESS
        );
    }
}

/// A LIVE preview card and a mid-swipe frame both keep TikTok's compose bar, so
/// both classify as `Feed` — but neither has a rail to tap. A live run tapped
/// 14 of them in a row for 0 likes before this test's rule existed.
#[test]
fn rail_presence_separates_tappable_videos_from_live_cards() {
    assert_eq!(
        screen::feed_card_kind(&load("feed-live-card.jpg")),
        screen::FeedCardKind::LivePreview
    );
    for name in ["feed-live-card.jpg", "feed-mid-swipe.jpg"] {
        let img = load(name);
        assert_eq!(
            screen::classify(&img, Some(LOGICAL_W)).kind,
            ScreenKind::Feed,
            "{name} should still read as the feed — that is what makes it a trap"
        );
        assert!(
            !screen::rail_icons_present(&img),
            "{name} has no rail but was reported as having one"
        );
    }

    for name in [
        "feed-iphone8.jpg",
        "feed-iphone8-b.jpg",
        "feed-rail-variant.png",
        "feed-heart-liked.jpg",
        "feed-heart-liked-sponsored.jpg",
    ] {
        let img = load(name);
        assert!(
            screen::rail_icons_present(&img),
            "{name} has a rail but was reported as having none"
        );
        let rail = screen::locate_action_rail(&img)
            .unwrap_or_else(|| panic!("{name} could not yield a fresh action rail"));
        // On a card whose heart is already filled (liked), the located like
        // point must land on the red heart — not on the comment bubble one
        // pitch below it. The regression this guards: an already-followed +
        // liked card hides the red badge AND excludes the red heart from the
        // white-glyph scan, so the chain used to start at the comment bubble
        // and `like_y` was mislabeled onto it.
        if name.contains("heart-liked") {
            let redness = screen::like_redness_at(&img, &rail);
            assert!(
                redness > screen::LIKE_FILLED_REDNESS,
                "{name}: located like point (y={:.3}) reads redness {redness:.1}, \
                 expected a filled heart (> {:.1}); locate_action_rail put the like \
                 target on the comment bubble instead of the heart",
                rail.like_y,
                screen::LIKE_FILLED_REDNESS
            );
        }
    }
}

/// Three consecutive frames of the *same* sponsored card, captured while its
/// video played. Same author, same caption, same like/comment/share counts —
/// the feed never moved.
fn same_card_frames() -> Vec<(&'static str, image::RgbImage)> {
    vec![
        ("feed-same-card-1.jpg", load("feed-same-card-1.jpg")),
        ("feed-same-card-2.jpg", load("feed-same-card-2.jpg")),
        ("feed-same-card-3.jpg", load("feed-same-card-3.jpg")),
    ]
}

/// The rail is what separates "the feed moved" from "the video played".
///
/// A playing video repaints the whole frame, so a whole-frame digest changes
/// between these three captures even though the card is identical. The rail
/// does not: it stays present and locatable on every frame of one card, and
/// only drops out during the transition to the next one (`feed-mid-swipe.jpg`).
#[test]
fn the_rail_survives_a_playing_video_but_not_a_swipe() {
    for (name, img) in same_card_frames() {
        assert!(
            screen::rail_icons_present(&img),
            "{name}: the card never changed, so its rail must still be present"
        );
        assert!(
            screen::locate_action_rail(&img).is_some(),
            "{name}: rail must stay locatable while the video plays"
        );
    }

    assert!(
        !screen::rail_icons_present(&load("feed-mid-swipe.jpg")),
        "a frame captured mid-swipe must not report a rail"
    );
}

/// The first real LIVE-room capture. Until this fixture existed the LIVE
/// detector had no test at all, and no negative control either.
#[test]
fn a_real_live_room_is_recognised_and_the_feed_is_not() {
    let live = load("live-room-1.jpg");
    assert_eq!(
        screen::classify(&live, Some(LOGICAL_W)).kind,
        ScreenKind::LiveRoom,
        "a real LIVE room must classify as LiveRoom"
    );
    // A LIVE room shows its own chat bar, never TikTok's compose pill.
    assert!(
        !screen::compose_bar_visible(&live).0,
        "a LIVE room has no FYP compose bar"
    );

    // Negative control: ordinary feed frames must never read as a LIVE room.
    for (name, img) in feed_frames().into_iter().chain(same_card_frames()) {
        assert_ne!(
            screen::classify(&img, Some(LOGICAL_W)).kind,
            ScreenKind::LiveRoom,
            "{name} is a feed frame and must not classify as a LIVE room"
        );
    }
}

/// A photo carousel and a video with big caption text are the same thing to a
/// single frame, and pretending otherwise cost more than it paid.
///
/// `feed-photo-carousel.jpg` is a real photo post — "Ảnh" badge, eight page
/// dots, a "1 / 8" counter. `feed-caption-text.jpg` is an ordinary video whose
/// caption is large white text in the same band. The removed detector called
/// the second one a carousel and the first one a video; across 40 captured
/// cards it scored one true positive against nine false ones and missed three
/// photo posts.
///
/// Both must now classify as an ordinary feed card, because that is the honest
/// answer from one frame, and because the horizontal swipe a carousel verdict
/// authorises navigates *away from the feed* when it is wrong. What actually
/// separates them is that the photo post does not change between frames.
#[test]
fn a_photo_post_and_a_caption_heavy_video_are_both_ordinary_feed_cards() {
    for name in ["feed-photo-carousel.jpg", "feed-caption-text.jpg"] {
        let img = load(name);
        assert_eq!(
            screen::feed_card_kind(&img),
            screen::FeedCardKind::Video,
            "{name} must read as an ordinary card; single-frame carousel \
             detection was removed because it was wrong nine times in ten"
        );
        assert!(
            screen::feed_ready(&img, Some(LOGICAL_W)),
            "{name} is still an actionable feed card"
        );
        assert!(
            screen::locate_action_rail(&img).is_some(),
            "{name} still has a locatable rail"
        );
    }
}

/// The saturation guard must not fire on the frame the swipe check depends on.
///
/// `rail_column_saturated` exists so one washed-out video frame cannot be read
/// as "the feed is between cards". But a real mid-swipe is *also* a frame with
/// no icon chain, and on this device the rail is off screen for only 80–120 ms —
/// often a single frame. If the guard swallowed that frame too, no swipe could
/// ever be confirmed. So: the real transition is not saturated, and neither is
/// any settled card.
#[test]
fn the_saturation_guard_does_not_swallow_a_real_mid_swipe() {
    let mid = load("feed-mid-swipe.jpg");
    assert!(
        !screen::rail_icons_present(&mid),
        "the mid-swipe fixture must still read as rail-less"
    );
    assert!(
        !screen::rail_column_saturated(&mid),
        "a real transition must not be mistaken for a washed-out frame, or no \
         swipe could ever be confirmed"
    );

    for (name, img) in feed_frames().into_iter().chain(same_card_frames()) {
        assert!(
            !screen::rail_column_saturated(&img),
            "{name}: an ordinary feed card must not read as saturated"
        );
    }
}

/// And it must fire on the frame it exists for: a rail column washed out by the
/// video behind it, which reads "no icon chain" for a reason that has nothing to
/// do with the feed moving.
#[test]
fn the_saturation_guard_fires_on_a_washed_out_rail_column() {
    let mut blown = load("feed-iphone8.jpg");
    let (w, h) = (blown.width(), blown.height());
    // Paint the rail column near-white across the whole search band, which is
    // what a white product background or a sky does to it.
    let x0 = ((screen::RAIL_X - 0.05) * w as f64) as u32;
    for y in (0.28 * h as f64) as u32..(0.85 * h as f64) as u32 {
        for x in x0..w {
            blown.put_pixel(x, y, image::Rgb([238, 240, 239]));
        }
    }

    assert!(
        !screen::rail_icons_present(&blown),
        "a washed-out column yields one continuous run, so no chain"
    );
    assert!(
        screen::rail_column_saturated(&blown),
        "and that is exactly the frame the guard has to catch"
    );
}

/// How much room the LIVE threshold actually has, now that `live_pill` is
/// measured on feed frames too instead of being left at 0.0 by the compose-bar
/// short circuit.
///
/// Measured: the room reads 134.1; the reddest feed frame in the fixture set
/// (`feed-live-card.jpg`, which carries a LIVE *preview* and so is the closest
/// an ordinary card gets) reads 17.2. The threshold sits at 90/80 between them.
///
/// This bounds the *false positive* direction only. The false negative — a room
/// whose host the account already follows, where the pill is simply not on
/// screen — cannot be bounded without a capture of that screen, and no fixture
/// here is one. `OFF_FEED_LIMIT` is what stops that case costing a session.
#[test]
fn the_live_threshold_has_measurable_headroom_over_every_feed_frame() {
    let room = screen::classify(&load("live-room-1.jpg"), Some(LOGICAL_W))
        .evidence
        .live_pill;
    assert!(
        room > 120.0,
        "the LIVE room reads {room:.1}, barely over the threshold"
    );

    for (name, img) in feed_frames().into_iter().chain(same_card_frames()) {
        let pill = screen::classify(&img, Some(LOGICAL_W)).evidence.live_pill;
        assert!(
            pill < room / 3.0,
            "{name} reads {pill:.1} against the room's {room:.1} — the margin \
             that keeps a feed card off the LIVE path has narrowed"
        );
    }
}

/// The ✕ that leaves a LIVE room sits at `LIVE_EXIT`, and that coordinate used
/// to fall outside the close-button search region entirely — the one ✕ the
/// engine most needs was the one it could never look for.
///
/// The region now contains it. Finding it is a separate problem: the room's ✕
/// is a thin white stroke over translucent grey, and the sheet template scores
/// 0.624 against it, well under `CLOSE_X_THRESHOLD`. So this pins the reachable
/// part, and pins that no fixture produces a false match either — widening a
/// search region is only safe if nothing new matches inside it.
#[test]
fn the_live_exit_is_inside_the_close_button_search_region() {
    let (_, top, _, bottom) = screen::close_x_region();
    assert!(
        screen::LIVE_EXIT.1 > top && screen::LIVE_EXIT.1 < bottom,
        "LIVE_EXIT y={} is outside the search band {top}..{bottom}",
        screen::LIVE_EXIT.1
    );

    for (name, img) in feed_frames()
        .into_iter()
        .chain(same_card_frames())
        .chain([("live-room-1.jpg", load("live-room-1.jpg"))])
    {
        if let Some((x, y, score)) = screen::find_close_button(&img, Some(LOGICAL_W)) {
            assert!(
                score < screen::CLOSE_X_THRESHOLD,
                "{name}: a close button matched at ({x:.3},{y:.3}) score={score:.3} — \
                 the widened region introduced a tap target that was not there"
            );
        }
    }
}

/// Measured at the correctly located heart, video content barely moves the
/// redness: 9.6 / 4.1 / 29.3 across three frames of the same unliked card, all
/// far below `LIKE_FILLED_REDNESS`. The threshold itself is sound — what was
/// not is where the rail said the heart was, which the test below covers.
#[test]
fn the_heart_reading_is_stable_when_the_rail_is_located_correctly() {
    let rail = screen::locate_action_rail(&load("feed-same-card-1.jpg"))
        .expect("the sponsored card has a rail");

    for name in [
        "feed-same-card-1.jpg",
        "feed-same-card-2.jpg",
        "feed-same-card-3.jpg",
    ] {
        let redness = screen::like_redness_at(&load(name), &rail);
        assert!(
            redness < 45.0,
            "{name}: unliked heart measured {redness:.1} at the located heart position"
        );
    }
}

/// The chain only outranks the badge when the two genuinely disagree, so this
/// pins both halves of the rule on the frame that needs it: the badge search on
/// its own is still wrong on `feed-same-card-2.jpg`, and `locate_action_rail`
/// still corrects it. Without the first assertion the test would keep passing
/// if the ribbon simply stopped registering, which would prove nothing about
/// the arbitration.
#[test]
fn the_glyph_chain_overrules_a_badge_the_video_faked() {
    let img = load("feed-same-card-2.jpg");
    let height = img.height() as f64;

    let badge_only = screen::find_action_rail(&img).expect("the ribbon still reads as a badge");
    let arbitrated = screen::locate_action_rail(&img).expect("rail");

    assert!(
        (badge_only.like_y * height - 554.5).abs() < 3.0,
        "the badge search alone still puts like at {:.1} px, not 554.5 — if the \
         input changed, this test no longer exercises the disagreement",
        badge_only.like_y * height
    );
    assert!(
        (arbitrated.like_y * height - 624.5).abs() < 3.0,
        "the chain must win: like landed at {:.1} px",
        arbitrated.like_y * height
    );
}

/// The chain reading is bounded by where the rail can actually be.
///
/// A live run located a rail with the heart at 199 pt on a frame still
/// animating after a back gesture — 115 pt above the real one, and with no
/// follow badge present to contradict it, so the arbitration above had nothing
/// to weigh it against. Every genuine capture in this fixture set puts the
/// heart within a few points of 0.47 of the frame; a chain outside the two
/// known layouts by more than half an icon is some other row of bright things.
#[test]
fn a_located_rail_always_lands_where_the_rail_can_be() {
    let mut checked = 0;
    for (name, img) in feed_frames()
        .into_iter()
        .chain(same_card_frames())
        .chain([("feed-photo-carousel.jpg", load("feed-photo-carousel.jpg"))])
    {
        let Some(rail) = screen::locate_action_rail(&img) else {
            continue;
        };
        checked += 1;
        assert!(
            (0.38..=0.55).contains(&rail.like_y),
            "{name}: like at {:.3} of the frame, outside anywhere the rail sits",
            rail.like_y
        );
    }
    assert!(checked >= 5, "only {checked} frames had a locatable rail");
}

/// A two-glyph chain does not outrank the badge. On `feed-heart-liked.jpg` the
/// heart is filled and so drops out of the white-glyph scan, leaving a stray
/// two-run pair from the video at a plausible pitch — 57 px away from where the
/// badge correctly puts the heart. Letting any chain win would move the like
/// target onto video content and read 123.8 as if the card were unliked.
#[test]
fn a_two_glyph_chain_does_not_outrank_the_badge() {
    let img = load("feed-heart-liked.jpg");
    let rail = screen::locate_action_rail(&img).expect("rail");

    assert!(
        (rail.like_y * img.height() as f64 - 629.5).abs() < 3.0,
        "like landed at {:.1} px, not on the badge-derived heart",
        rail.like_y * img.height() as f64
    );
    assert!(
        screen::like_redness_at(&img, &rail) > screen::LIKE_FILLED_REDNESS,
        "this card is liked and must still read as liked"
    );
}

/// The rail must not move while the card does not.
///
/// The three `feed-same-card-*` captures are the same sponsored post, seconds
/// apart, differing only in the video playing behind the chrome. The card
/// carries a pink "LIVE 8.8 Sale" ribbon above the rail, and the badge search
/// takes the topmost red run in its band — so on frame 2 the ribbon won:
///
/// | frame | layout | like_y before | like_y now |
/// |---|---|---|---|
/// | 1 | 2 | 630 px | 630.5 px |
/// | 2 | **1** | **554 px** | 624.5 px |
/// | 3 | 2 | 630 px | 630.0 px |
///
/// 76 px is more than half an icon pitch: the tap missed the heart and landed
/// on the ribbon, and `like_redness_at` then read the ribbon (79.7) instead of
/// the heart (3.1) — close enough to `LIKE_FILLED_REDNESS` to also fake an
/// "already liked" skip. It was the "tapped 14 in a row for 0 likes" failure
/// class returning through a different door.
///
/// `locate_action_rail` now ranks the badge against the white-glyph chain,
/// which did not move on any of the three frames.
#[test]
fn the_rail_stays_put_across_frames_of_one_card() {
    let heights: Vec<f64> = [
        "feed-same-card-1.jpg",
        "feed-same-card-2.jpg",
        "feed-same-card-3.jpg",
    ]
    .into_iter()
    .map(|name| {
        let img = load(name);
        let rail = screen::locate_action_rail(&img).expect("rail");
        rail.like_y * img.height() as f64
    })
    .collect();

    let spread = heights.iter().cloned().fold(f64::MIN, f64::max)
        - heights.iter().cloned().fold(f64::MAX, f64::min);
    // Half an icon pitch is ~65 px at @2x; anything near that taps the wrong icon.
    assert!(
        spread < 20.0,
        "like target moved {spread:.0} px across frames of the same card: {heights:?}"
    );
}
