use super::*;

#[derive(Debug)]
struct NoPublicPost;
impl std::fmt::Display for NoPublicPost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("no public Post")
    }
}
impl std::error::Error for NoPublicPost {}

#[tokio::test(start_paused = true)]
async fn prepared_caption_boundary_never_dispatches_post_when_rehearsal_refuses() {
    let labels = controls_for("com.ss.android.ugc.trill", "en", "38.3.2").unwrap();
    let plan = ComposerPlan::resolve(&labels).unwrap();
    let post = ElementBox {
        x: 891.0,
        y: 79.0,
        width: 147.0,
        height: 84.0,
        enabled: true,
        clickable: true,
        description: Some("Post".into()),
    };
    let xml = trill_focused_caption_xml();
    let session = FakeSession {
        snapshot_overrides: Mutex::new(std::collections::VecDeque::from([xml.clone(), xml])),
        ..FakeSession::with(vec![scene(vec![], None).leaving_by(post)])
    };
    let mut composer = Composer::new(&session, plan, |button: &ElementBox| button.centre());
    let mut boundaries = 0;
    let mut public_intents = 0;
    let result = composer
        .post_with_effect_intent(
            "Caption đầy đủ ✨",
            &AtomicBool::new(false),
            &mut || {
                boundaries += 1;
                // Rehearsal must return before any intent-writing callback or Post tap.
                if boundaries == 1 {
                    return Err(NoPublicPost.into());
                }
                public_intents += 1;
                Ok(())
            },
        )
        .await;
    assert!(result.unwrap_err().is::<NoPublicPost>());
    assert_eq!(boundaries, 1);
    assert_eq!(public_intents, 0);
    assert!(session.taps.lock().is_empty());
}
