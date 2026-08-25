//! Farming a post up to a number, and saying honestly whether the number is reachable.
//!
//! The operator's question is "bài này mới 200 view, tôi muốn 500 — farm thế nào là đủ?".
//! Answering it needs three different things, and they are three different problems:
//!
//! * **like** — one per account, and an account cannot like twice. The ceiling is the number
//!   of accounts that have not liked yet, full stop. A target above that is unreachable no
//!   matter how long it runs, and the operator has to be told that *before* it runs.
//! * **comment** — an account can comment repeatedly, so the ceiling is not the fleet size.
//!   It is taste: fourteen accounts leaving fifty comments reads as what it is.
//! * **view** — measured 24/08/2026 on a real post: ten phones opening it once added **+9**,
//!   opening it again immediately added **+9** again, and a third pass **+8**. So views are
//!   *not* one-per-account; they accumulate with passes, at roughly 0.9 per device per pass.
//!
//! That last measurement is the whole reason a view threshold is a loop rather than a single
//! shot, and the reason the loop is bounded by *observed progress* rather than by arithmetic:
//! the same fleet on a post it had already opened all day added nothing at all, and no
//! formula predicts that. **The plan proposes; the post's own numbers decide.**

use serde::{Deserialize, Serialize};

/// What the operator wants the post to reach. `None` means "leave this one alone".
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PostTargets {
    pub views: Option<u32>,
    pub likes: Option<u32>,
    pub comments: Option<u32>,
}

/// Where the post is now. `None` for a number this build or this screen cannot state.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PostNow {
    pub views: Option<u32>,
    pub likes: Option<u32>,
    pub comments: Option<u32>,
}

/// One metric's verdict, before anything runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetricPlan {
    /// How far short the post is. `0` when it is already there.
    pub shortfall: u32,
    /// The most this fleet could add, or `None` when nothing bounds it but time.
    pub ceiling: Option<u32>,
    /// Passes of the whole fleet needed, when that is a meaningful number.
    pub passes: Option<u32>,
    /// Why it cannot be reached, in the operator's language. `None` means it can.
    pub unreachable: Option<String>,
}

impl MetricPlan {
    /// Nothing to add, so nothing bounds it — including `ceiling`, which would otherwise
    /// differ between a like target that is met and one that is not, for no reason a caller
    /// could act on.
    fn met() -> Self {
        Self {
            shortfall: 0,
            ceiling: None,
            passes: Some(0),
            unreachable: None,
        }
    }
}

/// The whole plan, one entry per metric the operator asked for.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThresholdPlan {
    pub views: Option<MetricPlan>,
    pub likes: Option<MetricPlan>,
    pub comments: Option<MetricPlan>,
}

impl ThresholdPlan {
    /// Everything asked for is already true.
    ///
    /// A metric that could not be read is **not** satisfied, whatever its shortfall says. The
    /// unreadable branches set `shortfall: want` alongside `unreachable: Some(..)`, so a target
    /// of `0` on a metric this build cannot measure gave `shortfall == 0` — and this method
    /// then answered "satisfied" about the same plan `refusals()` was refusing. Which accessor
    /// a caller reached for first decided the behaviour; `threshold_gate` happens to check
    /// refusals first, so it was latent rather than visible. "Satisfied implies no refusals" is
    /// what the two methods jointly promise, and now they keep it.
    pub fn satisfied(&self) -> bool {
        [&self.views, &self.likes, &self.comments]
            .into_iter()
            .flatten()
            .all(|metric| metric.shortfall == 0 && metric.unreachable.is_none())
    }

    /// The reasons, if any, that this plan cannot finish — for showing before it starts.
    pub fn refusals(&self) -> Vec<String> {
        [&self.views, &self.likes, &self.comments]
            .into_iter()
            .flatten()
            .filter_map(|metric| metric.unreachable.clone())
            .collect()
    }
}

/// Views one device is worth in one pass.
///
/// Measured, not assumed: three consecutive passes of ten phones on one post added 9, 9 and
/// 8. Kept as a rate rather than a promise — the loop re-reads the post after every pass and
/// believes the post, not this number.
pub const VIEWS_PER_DEVICE_PER_PASS: f64 = 0.9;

/// What it would take to get this post to those numbers.
///
/// `actors` is how many phones are available; `unliked` how many of them have not already
/// liked this post, which is the only thing that bounds a like target.
pub fn plan_thresholds(
    targets: PostTargets,
    now: PostNow,
    actors: u32,
    unliked: u32,
) -> ThresholdPlan {
    ThresholdPlan {
        views: targets
            .views
            .map(|want| plan_views(want, now.views, actors)),
        // `unliked` is read from like history, `actors` from the phones actually here — so a
        // stale history can name more unliked accounts than there are phones, and the ceiling
        // would then promise likes nothing could deliver. The fleet is the harder bound.
        likes: targets
            .likes
            .map(|want| plan_likes(want, now.likes, unliked.min(actors))),
        comments: targets
            .comments
            .map(|want| plan_comments(want, now.comments, actors)),
    }
}

fn plan_views(want: u32, now: Option<u32>, actors: u32) -> MetricPlan {
    let Some(now) = now else {
        return MetricPlan {
            shortfall: want,
            ceiling: None,
            passes: None,
            unreachable: Some(
                "chưa đọc được số view của bài này (số view chỉ có trên lưới hồ sơ, và phải mở \
                 từng ô để biết ô nào là bài nào) — không đặt được ngưỡng khi chưa đo được"
                    .into(),
            ),
        };
    };
    if now >= want {
        return MetricPlan::met();
    }
    let shortfall = want - now;
    if actors == 0 {
        return MetricPlan {
            shortfall,
            ceiling: Some(0),
            passes: None,
            unreachable: Some("không có máy nào để xem".into()),
        };
    }
    // Views accumulate across passes, so nothing caps them but time — which is why this is
    // the one metric where a big target is a schedule rather than a refusal.
    // The `.max(1.0)` is a floor, not a rounding: without it one device at 0.9 views a pass
    // would divide fine but two-thirds of a device would not, and zero would divide by zero. It
    // does make a one-phone plan optimistic — 100 views reported as 100 passes where the
    // measured rate implies ~112 — and that is left standing because the loop re-reads the post
    // after every pass and believes the post, not this number.
    let per_pass = (f64::from(actors) * VIEWS_PER_DEVICE_PER_PASS).max(1.0);
    let passes = (f64::from(shortfall) / per_pass).ceil() as u32;
    MetricPlan {
        shortfall,
        ceiling: None,
        passes: Some(passes),
        unreachable: None,
    }
}

fn plan_likes(want: u32, now: Option<u32>, unliked: u32) -> MetricPlan {
    let Some(now) = now else {
        return MetricPlan {
            shortfall: want,
            ceiling: None,
            passes: None,
            unreachable: Some(
                "build này chưa đo được nút like có số, nên không biết bài đang bao nhiêu like"
                    .into(),
            ),
        };
    };
    if now >= want {
        return MetricPlan::met();
    }
    let shortfall = want - now;
    // The hard one. An account likes once; there is no second pass to fall back on.
    if shortfall > unliked {
        return MetricPlan {
            shortfall,
            ceiling: Some(unliked),
            // **No pass count on a plan that cannot finish.** It said `Some(1)`, so the same
            // struct claimed "one pass" and "this is impossible" — and a caller reading only
            // `passes` would schedule that pass.
            passes: None,
            unreachable: Some(format!(
                "cần thêm {shortfall} like nhưng chỉ có {unliked} máy chưa like bài này — mỗi acc \
                 chỉ like được một lần, nên chạy bao lâu cũng không quá {unliked}"
            )),
        };
    }
    MetricPlan {
        shortfall,
        ceiling: Some(unliked),
        passes: Some(1),
        unreachable: None,
    }
}

fn plan_comments(want: u32, now: Option<u32>, actors: u32) -> MetricPlan {
    let Some(now) = now else {
        return MetricPlan {
            shortfall: want,
            ceiling: None,
            passes: None,
            unreachable: Some("chưa đọc được số bình luận của bài".into()),
        };
    };
    if now >= want {
        return MetricPlan::met();
    }
    let shortfall = want - now;
    if actors == 0 {
        return MetricPlan {
            shortfall,
            ceiling: Some(0),
            passes: None,
            unreachable: Some("không có máy nào để bình luận".into()),
        };
    }
    // An account may comment more than once, so this is passes, not a ceiling — but each pass
    // puts the same accounts under the post again, which is its own kind of cost.
    MetricPlan {
        shortfall,
        ceiling: None,
        passes: Some(shortfall.div_ceil(actors)),
        unreachable: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_like_target_above_the_fleet_is_refused_before_it_runs() {
        // The measurement that matters: an account cannot like twice, so fourteen phones are
        // fourteen likes and no amount of running changes it. Discovering this after an hour
        // of farming is the failure this refusal exists to prevent.
        let plan = plan_thresholds(
            PostTargets {
                likes: Some(50),
                ..Default::default()
            },
            PostNow {
                likes: Some(22),
                ..Default::default()
            },
            14,
            14,
        );
        assert!(plan.refusals()[0].contains("chỉ like được một lần"));
        let likes = plan.likes.expect("asked for");
        assert_eq!(likes.shortfall, 28);
        assert_eq!(likes.ceiling, Some(14));
        assert!(likes.unreachable.is_some());
        // And it does not also offer a pass count. The same struct used to say "one pass" and
        // "this is impossible", and a caller reading only `passes` would schedule that pass.
        assert_eq!(likes.passes, None);
    }

    /// A like ceiling can never exceed the phones that are actually here.
    #[test]
    fn a_stale_like_history_cannot_promise_more_likes_than_there_are_phones() {
        // `unliked` comes from like history and `actors` from the fleet, so a history that has
        // not caught up can name twenty unliked accounts on a five-phone farm. The ceiling then
        // promises likes nothing could deliver, which is the one thing this metric exists to
        // refuse.
        let plan = plan_thresholds(
            PostTargets {
                likes: Some(30),
                ..Default::default()
            },
            PostNow {
                likes: Some(22),
                ..Default::default()
            },
            5,
            20,
        );
        let likes = plan.likes.expect("asked for");
        assert_eq!(likes.ceiling, Some(5), "the fleet is the harder bound");
        assert!(likes.unreachable.is_some(), "eight more likes, five phones");
    }

    #[test]
    fn a_like_target_within_the_fleet_is_one_pass() {
        let plan = plan_thresholds(
            PostTargets {
                likes: Some(30),
                ..Default::default()
            },
            PostNow {
                likes: Some(22),
                ..Default::default()
            },
            14,
            14,
        );
        let likes = plan.likes.expect("asked for");
        assert_eq!(likes.shortfall, 8);
        assert_eq!(likes.unreachable, None);
        assert_eq!(likes.passes, Some(1));
    }

    #[test]
    fn a_like_target_counts_only_the_accounts_that_have_not_liked() {
        // Ten of the fourteen already liked it on an earlier run, so the ceiling is four.
        let plan = plan_thresholds(
            PostTargets {
                likes: Some(30),
                ..Default::default()
            },
            PostNow {
                likes: Some(22),
                ..Default::default()
            },
            14,
            4,
        );
        assert!(plan.likes.expect("asked for").unreachable.is_some());
    }

    #[test]
    fn a_view_target_is_a_schedule_rather_than_a_refusal() {
        // Measured: ~0.9 views per device per pass, and passes keep counting — 9, 9, 8 over
        // three consecutive passes of ten phones. So +300 is time, not impossibility.
        let plan = plan_thresholds(
            PostTargets {
                views: Some(500),
                ..Default::default()
            },
            PostNow {
                views: Some(200),
                ..Default::default()
            },
            14,
            14,
        );
        let views = plan.views.expect("asked for");
        assert_eq!(views.shortfall, 300);
        assert_eq!(views.unreachable, None);
        // 14 phones ≈ 12.6 views a pass, so 24 passes.
        assert_eq!(views.passes, Some(24));
    }

    #[test]
    fn a_view_target_with_no_reading_refuses_rather_than_guessing() {
        // The number lives on the profile grid and has to be found by opening tiles. When
        // that failed, "we do not know" must not become "start farming anyway".
        let plan = plan_thresholds(
            PostTargets {
                views: Some(500),
                ..Default::default()
            },
            PostNow::default(),
            14,
            14,
        );
        assert!(plan.views.expect("asked for").unreachable.is_some());
    }

    /// `satisfied()` and `refusals()` cannot disagree about the same plan.
    #[test]
    fn a_metric_that_could_not_be_read_is_never_satisfied() {
        // Target zero on an unreadable metric: shortfall is zero because `want` is zero, while
        // the reading itself failed. The two accessors used to answer opposite questions here.
        let plan = plan_thresholds(
            PostTargets {
                views: Some(0),
                ..Default::default()
            },
            PostNow::default(),
            14,
            14,
        );
        assert!(!plan.refusals().is_empty(), "the view could not be read");
        assert!(
            !plan.satisfied(),
            "a plan with a refusal in it is not satisfied"
        );
    }

    #[test]
    fn a_target_already_met_asks_for_nothing() {
        let plan = plan_thresholds(
            PostTargets {
                views: Some(100),
                likes: Some(10),
                comments: Some(5),
            },
            PostNow {
                views: Some(1_285),
                likes: Some(22),
                comments: Some(26),
            },
            14,
            14,
        );
        assert!(plan.satisfied());
        assert!(plan.refusals().is_empty());
    }

    #[test]
    fn comments_are_passes_because_an_account_may_comment_twice() {
        let plan = plan_thresholds(
            PostTargets {
                comments: Some(40),
                ..Default::default()
            },
            PostNow {
                comments: Some(26),
                ..Default::default()
            },
            5,
            5,
        );
        let comments = plan.comments.expect("asked for");
        assert_eq!(comments.shortfall, 14);
        assert_eq!(comments.passes, Some(3));
        assert_eq!(comments.unreachable, None);
    }
}
