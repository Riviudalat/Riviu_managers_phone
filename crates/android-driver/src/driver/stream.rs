//! Everything that gets pixels off a phone: the scrcpy view pipeline, the minicap
//! producer, and the process bookkeeping around both.
//!
//! Split out of `driver.rs`, which was 4,734 lines at 13% test — the least-tested production
//! surface in the repo, and the two thirds of it that is not stream work made the stream work
//! hard to find. Nothing moved but the text: these are the same inherent methods on the same
//! struct, in a child module so they can still reach its private fields.

use super::*;

impl AndroidDriver {
    /// Claim the exclusive right to start a producer for `serial`.
    pub(super) fn claim_start(&self, serial: &str) -> anyhow::Result<StartClaim<'_>> {
        if !self.starting.lock().insert(serial.to_string()) {
            anyhow::bail!("a minicap start for {serial} is already in flight");
        }
        Ok(StartClaim {
            starting: &self.starting,
            serial: serial.to_string(),
        })
    }
    /// Refuse unless the driver owns no producer for `serial` and none is being
    /// born.
    ///
    /// Both halves matter: a producer *starting* would publish into the generation
    /// a handoff is about to hand out.
    pub(super) async fn producer_absent(&self, serial: &str) -> anyhow::Result<()> {
        if self.starting.lock().contains(serial) {
            anyhow::bail!("a minicap start for {serial} is already in flight");
        }
        if self.streams.lock().await.contains_key(serial) {
            anyhow::bail!(
                "{serial} still owns a minicap producer; stop_owned_stream must run first"
            );
        }
        Ok(())
    }
    /// What size the producer's frames come out at.
    ///
    /// **Native, not half, and that is a correctness choice rather than a quality one.**
    ///
    /// Flow measures in device pixels. A compiled coordinate records the size of the image
    /// it was picked against, `flow::executor::validate_geometry` refuses to dispatch
    /// unless the runtime frame matches the device's qualified geometry, and
    /// `FrameRegionChanged` evidence names a rectangle in frame pixels. This producer ran
    /// at `Projection::half` from the start, so on a 1080x2220 phone every frame was
    /// 540x1110 and that check could never pass: image-coordinate taps and the Flow
    /// inspector's coordinate picker were both unreachable on Android no matter what else
    /// was fixed.
    ///
    /// Nothing pays for this that was not already paying. The Android tile grid does not use
    /// minicap at all -- it is on the H.264 view path -- and `background_sample_candidate`
    /// returns false for Android, so the only consumers of these frames are the ones that
    /// measure them. The AI comment path is unaffected in either direction:
    /// `openai_client::make_contact_sheet` resizes every frame to 375x667 before a provider
    /// sees it, so the token bill does not depend on what the phone captured.
    ///
    /// If Android tiles ever move back onto minicap, this is the line to revisit: half the
    /// edge is a quarter of the bytes, and twenty tiles is where that mattered.
    pub(super) fn producer_projection(screen: (u32, u32)) -> crate::frames::Projection {
        crate::frames::Projection::native(screen.0, screen.1)
    }
    /// Spawn minicap for `serial`, publishing into exactly `generation`.
    ///
    /// Never advances a generation and never holds a lock across the adb work. The
    /// step order is the port-hygiene contract: the APK push happens before any
    /// port is taken, the forward happens exactly once, and the producer is only
    /// registered at the very end so a failed start leaves nothing to clean up.
    ///
    /// Returns whether a decoded frame was observed — always `false` for
    /// [`StreamReadiness::BestEffort`], which does not wait for one.
    pub(super) async fn spawn_producer(
        &self,
        serial: &str,
        generation: u64,
        readiness: StreamReadiness,
    ) -> anyhow::Result<bool> {
        let sink = self.sink()?;
        let apk = self.minicap_apk.clone().ok_or_else(|| {
            anyhow!(
                "no minicap apk configured: set RIVIU_MINICAP_APK to DeviceFarmer's \
                 noarch/minicap.apk (AGENTS.md 9)"
            )
        })?;

        let screen = crate::frames::device_screen(&self.adb, serial).await?;
        let options =
            crate::frames::MinicapOptions::for_device(serial, Self::producer_projection(screen));
        // Push before taking a port, so a push failure strands nothing.
        crate::frames::ensure_apk(&self.adb, serial, &apk).await?;

        if readiness == StreamReadiness::DecodedFrame {
            self.refuse_undrivable_screen(serial).await?;
        }

        let mut child = tokio::process::Command::new(self.adb.path())
            .args([
                "-s",
                serial,
                "shell",
                &crate::frames::launch_command(&options),
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .context("spawn minicap")?;

        // Forward exactly once. `adb forward tcp:0` allocates a *new* host port on
        // every call, so retrying the forward alongside the connect leaks one port
        // per attempt — measured: four stranded forwards to the same socket after
        // a single launch. Only the connect is retried, because minicap binds its
        // socket a beat after `app_process` starts.
        let host_port = crate::frames::forward(&self.adb, serial, &options.socket).await?;
        let mut connected = None;
        let mut last_error = None;
        for _ in 0..40 {
            tokio::time::sleep(Duration::from_millis(250)).await;
            if child.try_wait().ok().flatten().is_some() {
                crate::frames::remove_forward(&self.adb, serial, host_port)
                    .await
                    .ok();
                anyhow::bail!("minicap exited before it accepted a connection");
            }
            match crate::frames::MinicapStream::connect(host_port).await {
                Ok(stream) => {
                    connected = Some(stream);
                    break;
                }
                Err(error) => last_error = Some(error),
            }
        }
        let Some(mut stream) = connected else {
            // Give the port back before surfacing the failure.
            crate::frames::remove_forward(&self.adb, serial, host_port)
                .await
                .ok();
            let _ = child.kill().await;
            return Err(
                last_error.unwrap_or_else(|| anyhow!("minicap never accepted a connection"))
            );
        };
        let banner = stream.banner().clone();
        tracing::info!(
            serial,
            host_port,
            generation,
            ?readiness,
            banner = ?banner,
            "minicap frame source started"
        );

        // The interaction path needs to know a real frame landed. A oneshot rather
        // than polling the hub: a *parked* frame from before this producer is still
        // in the hub's cache, so watching the cache would accept a pre-session frame
        // as proof of a stream started after it.
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel::<()>();
        let mut ready_tx = (readiness == StreamReadiness::DecodedFrame).then_some(ready_tx);
        let udid = serial.to_string();
        let publisher = Arc::clone(&sink);
        let reader = tokio::spawn(async move {
            let sink = publisher;
            loop {
                match stream.next_frame().await {
                    Ok(frame) => {
                        // A frame that does not decode is skipped, not published,
                        // while we are still waiting for the first one.
                        let qualifies =
                            ready_tx.is_some() && riviu_core::frame_source::decodes_as_jpeg(&frame);
                        if ready_tx.is_some() && !qualifies {
                            tracing::debug!(
                                udid,
                                generation,
                                bytes = frame.len(),
                                "skipping an undecodable candidate first frame"
                            );
                            continue;
                        }
                        // A stale generation is the signal to stop, not an error:
                        // a newer stream owns this device now.
                        if !sink.publish_if_current(&udid, generation, frame) {
                            tracing::info!(udid, generation, "minicap reader superseded; stopping");
                            return;
                        }
                        if qualifies {
                            if let Some(sender) = ready_tx.take() {
                                let _ = sender.send(());
                            }
                        }
                    }
                    Err(error) => {
                        tracing::warn!(udid, generation, %error, "minicap reader stopped");
                        return;
                    }
                }
            }
        });

        let mut first_frame_observed = false;
        if readiness == StreamReadiness::DecodedFrame {
            let started = std::time::Instant::now();
            match tokio::time::timeout(INTERACTION_FIRST_FRAME_TIMEOUT, ready_rx).await {
                Ok(Ok(())) => {
                    first_frame_observed = true;
                    tracing::info!(
                        serial,
                        generation,
                        ms = started.elapsed().as_millis(),
                        "minicap first decoded frame accepted"
                    );
                }
                // Tear down rather than reporting `Ok(first_frame_observed: false)`:
                // every consumer in core treats `false` as fatal, so returning it
                // would buy nothing except a live orphan producer.
                _ => {
                    reader.abort();
                    let _ = child.kill().await;
                    crate::frames::remove_forward(&self.adb, serial, host_port)
                        .await
                        .ok();
                    anyhow::bail!(
                        "minicap produced no decodable frame in {}s for {serial}: the display may \
                         not have changed (minicap publishes on change), or the projection is \
                         wrong. banner {}x{} virtual {}x{} orient {}, host port {host_port}, \
                         sink generation {}",
                        INTERACTION_FIRST_FRAME_TIMEOUT.as_secs(),
                        banner.real_width,
                        banner.real_height,
                        banner.virtual_width,
                        banner.virtual_height,
                        banner.orientation,
                        sink.generation(serial)
                    );
                }
            }
        }

        // Registered last, so nothing above needs undoing on failure.
        self.streams.lock().await.insert(
            serial.to_string(),
            StreamProducer {
                generation,
                host_port,
                child,
                reader,
                device_pid: banner.pid,
            },
        );
        Ok(first_frame_observed)
    }
    /// What the phone says about its own display, for a caller that needs to explain
    /// itself rather than act.
    ///
    /// `None` is unknown, never "asleep": this fleet spans Android 9 to 15 and they do
    /// not print the same `dumpsys` bodies. Exposed because the desktop's view watchdog
    /// logs through `log` while this crate emits `tracing`, which currently reaches no
    /// sink — so the only way an operator sees *why* a view went silent is for the app
    /// layer to ask and say it.
    pub async fn display_is_awake(&self, serial: &str) -> Option<bool> {
        let power = self.adb.shell(serial, "dumpsys power").await.ok()?;
        adb::parse_display_awake(&power)
    }
    /// Wake the screen before capturing it, because a sleeping one encodes nothing.
    ///
    /// [`Self::refuse_undrivable_screen`] already knew this for minicap and *refuses*;
    /// the view path must not, and the difference is the caller. Nurture is asking to
    /// drive a phone and a refusal sends the operator to unlock it. The tile grid is
    /// asking to watch every phone at once: refusing there gives a black tile and a
    /// watchdog that restarts the encoder every five seconds forever, which is exactly
    /// what a sleeping Redmi did on 14/08/2026 until one keyevent fixed it.
    ///
    /// Best effort on purpose. A phone that will not wake may still have a screen worth
    /// capturing, and trading a working tile for none because a keyevent failed is a
    /// worse outcome than a dim one. Logged at info when the display really was asleep,
    /// so the watchdog's "published nothing" line has a cause next to it instead of
    /// repeating anonymously.
    async fn wake_display_for_capture(&self, serial: &str) {
        let awake = match self.adb.shell(serial, "dumpsys power").await {
            Ok(power) => adb::parse_display_awake(&power),
            Err(_) => None,
        };
        if !adb::should_wake_before_capture(awake) {
            return;
        }
        match self.adb.shell(serial, adb::WAKE_KEYEVENT).await {
            Ok(_) => {
                if awake == Some(false) {
                    tracing::info!(%serial, "display was asleep; woke it before capturing");
                }
            }
            Err(error) => {
                tracing::warn!(%serial, %error, "could not wake before capturing");
            }
        }
    }
    /// Refuse a screen minicap cannot compose from, before anything is spawned.
    ///
    /// Two separate conditions, and the second is the one that bites: measured on a
    /// locked Redmi Note 12 (11/08/2026), `dumpsys power` reported
    /// `mWakefulness=Awake` and `mScreenOnFully=true` while the phone sat on its
    /// lock screen and nothing could be foregrounded. Wakefulness alone passes a
    /// phone no driver can drive.
    ///
    /// An unreadable `dumpsys` is **unknown**, never a refusal — the fleet spans
    /// Android 9 to 15 and they do not print the same bodies.
    async fn refuse_undrivable_screen(&self, serial: &str) -> anyhow::Result<()> {
        if let Ok(power) = self.adb.shell(serial, "dumpsys power").await {
            if adb::parse_display_awake(&power) == Some(false) {
                anyhow::bail!(
                    "{serial} has its display asleep; minicap composes nothing while the screen \
                     is off. Wake the phone and retry"
                );
            }
        }
        if let Ok(window) = self.adb.shell(serial, "dumpsys window").await {
            if adb::parse_keyguard_locked(&window) == Some(true) {
                anyhow::bail!(
                    "{serial} is on the lock screen. The screen may be on, but no app can be \
                     brought to the foreground until it is unlocked"
                );
            }
        }
        Ok(())
    }
    /// Kill a feed and drop its forward. Best effort by design: the caller has
    /// already removed it from the registry, so failing here must not strand the
    /// device with a producer nobody owns.
    async fn stop_producer(&self, serial: &str, mut producer: StreamProducer) -> bool {
        producer.reader.abort();
        // Ignore the kill error: the child may already have been reaped by an
        // earlier `try_wait`, and that is a stopped child, not a failure.
        let _ = producer.child.start_kill();
        let confirmed = matches!(
            tokio::time::timeout(CHILD_EXIT_TIMEOUT, producer.child.wait()).await,
            Ok(Ok(_))
        );
        if !confirmed {
            tracing::warn!(
                serial,
                device_pid = producer.device_pid,
                "could not confirm the minicap child exited"
            );
        }
        if let Err(error) =
            crate::frames::remove_forward(&self.adb, serial, producer.host_port).await
        {
            tracing::warn!(serial, port = producer.host_port, %error, "could not remove the minicap forward");
        }
        confirmed
    }
    /// Remove whatever producer we own for `serial` and kill it.
    ///
    /// `true` means the driver is confirmed to own no live producer afterwards —
    /// **including when it owned none to begin with**. That is not laxity: the
    /// control plane's `StreamStopProof::confirms_stop` requires
    /// `child_stopped && new > old`, and reporting `false` for "there was nothing to
    /// stop" would quarantine the lease on every teardown that follows a failed
    /// stream start. iOS answers the same way.
    async fn take_and_stop_producer(&self, serial: &str) -> bool {
        let producer = self.streams.lock().await.remove(serial);
        match producer {
            Some(producer) => self.stop_producer(serial, producer).await,
            None => true,
        }
    }
    /// The one place a teardown advances a generation.
    ///
    /// `retain_last_frame` distinguishes park from stop: both must make every frame
    /// the dead producer still holds unpublishable, but park keeps the tile's last
    /// image instead of blanking it.
    pub(super) async fn teardown_stream(
        &self,
        serial: &str,
        retain_last_frame: bool,
    ) -> anyhow::Result<riviu_core::stream_budget::StreamStopProof> {
        let sink = self.sink()?;
        let child_stopped = self.take_and_stop_producer(serial).await;
        // Read the old generation separately: `FrameSink` returns only the new one,
        // deliberately. Safe because every advance for this serial happens either in
        // the producer-map critical section or under a start claim, and the control
        // plane holds a per-UDID operation lock across the whole sequence.
        let old_generation = sink.generation(serial);
        let new_generation = if retain_last_frame {
            sink.park_and_advance(serial)
        } else {
            sink.clear_and_advance(serial)
        };
        if child_stopped {
            // Recording the stop lets the plane's recovery path start a session
            // straight after a stop without confirming the handoff again.
            self.interaction.record_stopped(serial, new_generation);
        } else {
            self.interaction.clear(serial);
        }
        Ok(riviu_core::stream_budget::StreamStopProof {
            old_generation,
            new_generation,
            child_stopped,
        })
    }
    /// Start or reuse the tile feed for one device.
    ///
    /// Reuses a live producer whose generation is still current, which is what keeps
    /// a repeated `ensure_stream` from restarting a working stream — the same rule
    /// the iOS path follows.
    pub(super) async fn ensure_minicap_locked(&self, serial: &str) -> anyhow::Result<()> {
        let sink = self.sink()?;
        let claim = self.claim_start(serial)?;

        let reusable = {
            let mut streams = self.streams.lock().await;
            match streams.get_mut(serial) {
                Some(existing) => {
                    let alive = existing
                        .child
                        .try_wait()
                        .map(|status| status.is_none())
                        .unwrap_or(false);
                    alive
                        && existing.generation == sink.generation(serial)
                        && !existing.reader.is_finished()
                }
                None => false,
            }
        };
        if reusable {
            return Ok(());
        }
        // Whatever is there is stale; killing it happens outside the map lock.
        self.take_and_stop_producer(serial).await;

        let generation = sink.clear_and_advance(serial);
        let started = self
            .spawn_producer(serial, generation, StreamReadiness::BestEffort)
            .await;
        drop(claim);
        started.map(|_| ())
    }
    /// Stop the feed for one device, if we own one.
    pub async fn stop_minicap(&self, serial: &str) {
        self.take_and_stop_producer(serial).await;
    }
    fn view_sink(&self) -> anyhow::Result<Arc<dyn crate::view::ViewSink>> {
        self.view_sink.lock().clone().ok_or_else(|| {
            anyhow!(
                "no view sink is wired to the Android driver; call set_view_sink before \
                 starting a view stream"
            )
        })
    }
    fn claim_view_start(&self, serial: &str) -> anyhow::Result<StartClaim<'_>> {
        if !self.view_starting.lock().insert(serial.to_string()) {
            anyhow::bail!("a scrcpy view start for {serial} is already in flight");
        }
        Ok(StartClaim {
            starting: &self.view_starting,
            serial: serial.to_string(),
        })
    }
    /// A `start_view_stream` that still holds the claim. The keeper must not
    /// treat this as a silent producer — there has been no packet yet.
    pub fn view_start_in_flight(&self, serial: &str) -> bool {
        self.view_starting.lock().contains(serial)
    }
    /// Live producer at either preset, or a start that still holds the claim.
    /// The desktop keeper must not spawn a tile while overlay retune is mid-flight.
    pub async fn view_is_active(&self, serial: &str) -> bool {
        if self.view_start_in_flight(serial) {
            return true;
        }
        self.view_is_running(serial, crate::scrcpy::ViewPreset::Tile)
            .await
            || self
                .view_is_running(serial, crate::scrcpy::ViewPreset::Overlay)
                .await
    }
    /// Whether this serial already has a live view at `preset`.
    pub async fn view_is_running(&self, serial: &str, preset: crate::scrcpy::ViewPreset) -> bool {
        let mut views = self.views.lock().await;
        match views.get_mut(serial) {
            Some(existing) => {
                let alive = existing
                    .child
                    .try_wait()
                    .map(|status| status.is_none())
                    .unwrap_or(false);
                alive
                    && existing.preset == preset
                    && !existing.reader.is_finished()
                    && existing.generation > 0
            }
            None => false,
        }
    }
    /// Start or retune the scrcpy view. Same process, new options.
    ///
    /// A producer that is already painting is **kept until the replacement has a keyframe**
    /// (see [`ViewStart`]) rather than stopped up front. That is what the operator feels when
    /// they open a phone: the picture keeps moving through the switch instead of freezing for
    /// the length of a spawn. Does not touch minicap or `StreamBudgetManager`.
    pub async fn start_view_stream(
        &self,
        serial: &str,
        preset: crate::scrcpy::ViewPreset,
    ) -> anyhow::Result<u64> {
        let sink = self.view_sink()?;
        let claim = self.claim_view_start(serial)?;
        self.desired_presets
            .lock()
            .insert(serial.to_string(), preset);
        if self.view_is_running(serial, preset).await {
            return Ok(sink.generation(serial));
        }
        // "Is something alive on this serial" rather than "is it at the preset we want":
        // anything still running is a picture worth keeping until the new one is proven.
        let replacing = self.views.lock().await.contains_key(serial);
        let start = if replacing {
            ViewStart::Replace
        } else {
            ViewStart::Fresh {
                generation: sink.advance(serial),
            }
        };
        self.spawn_view(serial, start, preset).await?;
        drop(claim);
        Ok(sink.generation(serial))
    }
    /// Stop the view for one serial. `true` when nothing is left running,
    /// including when there was nothing to stop.
    pub async fn stop_view_stream(&self, serial: &str) -> bool {
        // Deliberately does NOT forget the desired preset. Measured: it used to, and the
        // watchdog's restart path is stop-then-start (state.rs), so every restart read back
        // the default and an open overlay silently dropped to the tile encode -- observed
        // live as `gen=5 tile 216x480` while the overlay was still on screen.
        //
        // The desire belongs to the operator having an overlay open, not to a producer's
        // lifetime. It is overwritten, never cleared: closing the overlay asks for `tile`,
        // which is the same insert.
        self.take_and_stop_view(serial).await
    }
    /// What this serial should be restarted at. `Tile` for anything never asked for, which
    /// is the pre-existing behaviour for every device the operator has not opened.
    pub fn desired_view_preset(&self, serial: &str) -> crate::scrcpy::ViewPreset {
        self.desired_presets
            .lock()
            .get(serial)
            .copied()
            .unwrap_or(crate::scrcpy::ViewPreset::Tile)
    }
    /// Set the quality and frame rate new views will start with.
    ///
    /// Does **not** touch running producers. Restarting sixteen encoders because a
    /// slider moved is a fleet-wide stall the operator did not ask for, so the caller
    /// decides which views to restart and when — see `set_view_preset`.
    pub fn set_view_tuning(
        &self,
        grid: riviu_core::StreamQuality,
        focus: riviu_core::StreamQuality,
        fps: u32,
    ) {
        *self.view_tuning.lock() = ViewTuningChoice { grid, focus, fps };
    }
    /// Retune by restarting the same producer. Not a second `app_process`.
    pub async fn set_view_preset(
        &self,
        serial: &str,
        preset: crate::scrcpy::ViewPreset,
    ) -> anyhow::Result<u64> {
        self.start_view_stream(serial, preset).await
    }
    pub async fn stop_all_views(&self) {
        let serials: Vec<String> = self.views.lock().await.keys().cloned().collect();
        for serial in serials {
            self.take_and_stop_view(&serial).await;
        }
    }
    async fn take_and_stop_view(&self, serial: &str) -> bool {
        let producer = self.views.lock().await.remove(serial);
        match producer {
            Some(producer) => self.stop_view_producer(serial, producer).await,
            None => true,
        }
    }
    async fn stop_view_producer(&self, serial: &str, mut producer: ViewProducer) -> bool {
        producer.reader.abort();
        // The control socket goes first, and shut down rather than merely dropped.
        // `DesktopConnection.shutdown` on the device closes all three sockets; giving its
        // reader a clean EOF is what stops a teardown that races a `write_all` from leaving
        // a half-written message behind — and a half-written message on this stream is not a
        // lost byte, it is `ControlProtocolException` on a server we are about to kill
        // anyway, but which would log a fatal error and confuse the next reader of the log.
        producer.control_drain.abort();
        if let Ok(mut socket) = producer.control.try_lock() {
            let _ = socket.shutdown().await;
        }
        let _ = producer.child.start_kill();
        let confirmed = matches!(
            tokio::time::timeout(CHILD_EXIT_TIMEOUT, producer.child.wait()).await,
            Ok(Ok(_))
        );
        if !confirmed {
            tracing::warn!(serial, "could not confirm the scrcpy child exited");
        }
        if let Err(error) =
            crate::frames::remove_forward(&self.adb, serial, producer.host_port).await
        {
            tracing::warn!(
                serial,
                port = producer.host_port,
                %error,
                "could not remove the scrcpy forward"
            );
        }
        confirmed
    }
    /// Kill leftover *our* 3.3.4 rows. The encoder argv has `Server 3.3.4`
    /// and not the JAR path (`CLASSPATH` is environ). A grep for the JAR
    /// only hits the `sh -c` wrapper and leaves OMX held — Note 8 then
    /// hellos without an IDR. Never match GenFarmer (`Server 2.4`).
    async fn stop_our_scrcpy_leftovers(&self, serial: &str) {
        let listing = self
            .adb
            .shell(serial, crate::scrcpy::LEFTOVER_LIST_SCRIPT)
            .await
            .unwrap_or_default();
        let mut unique = Vec::new();
        for pid in listing
            .split_whitespace()
            .filter_map(|token| token.parse::<u32>().ok())
            .filter(|pid| *pid > 0)
        {
            if !unique.contains(&pid) {
                unique.push(pid);
            }
        }
        for pid in &unique {
            let _ = self.adb.shell(serial, &format!("kill {pid}")).await;
        }
        if unique.is_empty() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(300)).await;

        // Confirm, then escalate. `kill` is SIGTERM, and a server blocked inside
        // MediaCodec does not have to honour it -- measured on the Redmi, two
        // `app_process` were still holding the encoder after this function had already
        // run and reported nothing, because it never looked again. A survivor is not
        // harmless: it keeps the hardware encoder, so the fresh server we are about to
        // start fails `MediaCodec.configure` and the tile stays black.
        //
        // One escalation, not a loop: if SIGKILL does not take, the process is unkillable
        // by us and retrying cannot change that, so say so and let the spawn attempt
        // produce the real error.
        let survivors = self
            .adb
            .shell(serial, crate::scrcpy::LEFTOVER_LIST_SCRIPT)
            .await
            .unwrap_or_default();
        let survivors: Vec<u32> = survivors
            .split_whitespace()
            .filter_map(|token| token.parse::<u32>().ok())
            .filter(|pid| *pid > 0 && unique.contains(pid))
            .collect();
        if survivors.is_empty() {
            return;
        }
        tracing::warn!(
            serial,
            ?survivors,
            "scrcpy server ignored SIGTERM; sending SIGKILL"
        );
        for pid in &survivors {
            let _ = self.adb.shell(serial, &format!("kill -9 {pid}")).await;
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    async fn spawn_view(
        &self,
        serial: &str,
        start: ViewStart,
        preset: crate::scrcpy::ViewPreset,
    ) -> anyhow::Result<()> {
        let sink = self.view_sink()?;
        let server = self.scrcpy_server.clone().ok_or_else(|| {
            anyhow!(
                "no scrcpy server configured: set RIVIU_SCRCPY_SERVER or ship \
                 sidecars/android/noarch/scrcpy-server (AGENTS.md 9.50)"
            )
        })?;

        // Read once per spawn: a producer keeps whatever tuning it started with, so a
        // settings change takes effect on the next restart rather than half-way through
        // an encode.
        let tuning = {
            let guard = self.view_tuning.lock();
            // The overlay is one phone filling a window; a tile is one of twenty. They are
            // different pictures at different sizes, so they get the operator's two separate
            // quality choices rather than sharing one.
            let quality = match preset {
                crate::scrcpy::ViewPreset::Tile => guard.grid.clone(),
                crate::scrcpy::ViewPreset::Overlay => guard.focus.clone(),
            };
            preset.tuned(quality, guard.fps)
        };

        // Timed step by step, because "a start takes about eleven seconds" is not something
        // anyone can act on. Measured on this fleet a preset switch left the operator with
        // **17.8 s of no frames at all** after double-clicking a phone, and the only way to
        // know which of these five adb round trips to attack is to charge each of them.
        let spawn_started = std::time::Instant::now();
        self.wake_display_for_capture(serial).await;
        let woke = spawn_started.elapsed();

        crate::scrcpy::ensure_server(&self.adb, serial, &server).await?;
        let served = spawn_started.elapsed();
        // NOT on the replace path, and this is load-bearing rather than an optimisation: the
        // sweep matches every 3.3.4 server of ours on the device, and on that path one of
        // them is the producer still painting the operator's screen. Sweeping here would kill
        // the picture we are going through all this to preserve.
        if matches!(start, ViewStart::Fresh { .. }) {
            self.stop_our_scrcpy_leftovers(serial).await;
        }
        let swept = spawn_started.elapsed();

        // Drop forwards left over from a run that never cleaned up. Every failure path
        // below removes its own forward, so this is not for the current process -- it is
        // for the previous one. `adb forward` lives in the adb server, so a crash, a
        // force-quit, or a kill that skips `stop_view_producer` leaves the forward behind
        // with nothing to remove it, and `prune_forwards` cannot find it because it
        // matches the socket name exactly while scrcpy randomises the `scid`. Measured
        // after several development restarts: five stranded forwards across two phones,
        // each to a dead socket, plus two orphaned `app_process` on one of them.
        //
        // `keep` is every port a live producer holds, which is what makes this safe to
        // run on a device that is already streaming into another window.
        let live_ports: Vec<u16> = self
            .views
            .lock()
            .await
            .values()
            .map(|producer| producer.host_port)
            .collect();
        crate::frames::prune_scrcpy_forwards(
            &self.adb,
            serial,
            crate::scrcpy::FORWARD_PREFIX,
            &live_ports,
        )
        .await;
        let pruned = spawn_started.elapsed();

        let scid = (rand::random::<u32>() & 0x7fff_ffff).max(1);
        // Device listens (`tunnel_forward`). Spawn first. This Windows adb
        // refuses the abstract socket if nothing is bound yet, so a TCP
        // opened before listen EOFs and never becomes the video socket.
        // Retry TCP only while dummy has not arrived (`NotListening`).
        let mut child = tokio::process::Command::new(self.adb.path());
        child
            .args([
                "-s",
                serial,
                "shell",
                &crate::scrcpy::launch_command(scid, tuning),
            ])
            .stdin(Stdio::null())
            // Piped, not null. `Ln.i` goes to FD 1, so discarding stdout threw away the
            // server's account of itself -- which encoder it chose, the `Device: [...]` line,
            // and `Video capture reset`. A handshake that hangs instead of exiting then left
            // no host-side evidence whatsoever; the one measured instance ran six minutes
            // with nothing logged (AGENTS.md 9.71).
            //
            // Safe against the obvious hazard: the pipe is only ever read by
            // `scrcpy_exit_detail`, which runs on failure paths and then the child is killed.
            // A healthy server logs a handful of lines at startup and then nothing, so the
            // pipe cannot fill in normal operation -- and if it ever did, the writer blocking
            // is the server, not this process.
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(windows)]
        child.creation_flags(0x0800_0000);
        let mut child = child.spawn().context("spawn scrcpy-server")?;

        let spawned = spawn_started.elapsed();
        let host_port = match crate::scrcpy::forward(&self.adb, serial, scid).await {
            Ok(port) => port,
            Err(error) => {
                let _ = child.kill().await;
                return Err(error);
            }
        };
        let forwarded = spawn_started.elapsed();
        let mut stream = None;
        let mut control = None;
        let mut last_error = None;
        for attempt in 0..40 {
            if child.try_wait().ok().flatten().is_some() {
                crate::frames::remove_forward(&self.adb, serial, host_port)
                    .await
                    .ok();
                anyhow::bail!(
                    "scrcpy-server exited before it accepted a connection{}",
                    scrcpy_exit_detail(&mut child).await
                );
            }
            match crate::scrcpy::ScrcpyStream::try_accept(host_port).await {
                Ok((accepted, accepted_control)) => {
                    stream = Some(accepted);
                    control = Some(accepted_control);
                    break;
                }
                Err(crate::scrcpy::AcceptError::NotListening(error)) => {
                    last_error = Some(error);
                    if attempt + 1 < 40 {
                        tokio::time::sleep(Duration::from_millis(50)).await;
                    }
                }
                Err(crate::scrcpy::AcceptError::Protocol(error)) => {
                    crate::frames::remove_forward(&self.adb, serial, host_port)
                        .await
                        .ok();
                    // The server's own words BEFORE the kill, on this path too. A protocol
                    // failure is exactly the case where it usually has something to say and
                    // is usually still alive to say it.
                    let said = scrcpy_exit_detail(&mut child).await;
                    let _ = child.kill().await;
                    return Err(error.context(format!("scrcpy handshake failed{said}")));
                }
            }
        }
        let Some(mut stream) = stream else {
            crate::frames::remove_forward(&self.adb, serial, host_port)
                .await
                .ok();
            let said = scrcpy_exit_detail(&mut child).await;
            let _ = child.kill().await;
            let error = last_error.unwrap_or_else(|| anyhow!("scrcpy never accepted a connection"));
            return Err(error.context(format!(
                "scrcpy never accepted a connection after 40 attempts{said}"
            )));
        };
        // Set in the same arm as `stream`, so this cannot be reached without it.
        let control = control.expect("try_accept returns both sockets or neither");
        let first =
            match tokio::time::timeout(Duration::from_secs(8), stream.next_sync_sample()).await {
                Ok(Ok(sample)) => sample,
                Ok(Err(error)) => {
                    crate::frames::remove_forward(&self.adb, serial, host_port)
                        .await
                        .ok();
                    let said = scrcpy_exit_detail(&mut child).await;
                    let _ = child.kill().await;
                    return Err(error.context(format!("scrcpy stream failed{said}")));
                }
                Err(_) => {
                    crate::frames::remove_forward(&self.adb, serial, host_port)
                        .await
                        .ok();
                    let said = scrcpy_exit_detail(&mut child).await;
                    let _ = child.kill().await;
                    anyhow::bail!("scrcpy produced no keyframe after the hello{said}");
                }
            };

        // The swap point, and it is deliberately *here* rather than before the spawn.
        //
        // Everything above can fail, and until this line the producer the operator is
        // watching is untouched: a failed replacement costs them nothing, where the old
        // order left the device dark. From here on the new stream is proven -- it has a
        // keyframe in hand -- so the handover is a hand-off rather than a gamble.
        let generation = match start {
            ViewStart::Fresh { generation } => generation,
            ViewStart::Replace => {
                self.take_and_stop_view(serial).await;
                sink.advance(serial)
            }
        };
        let swapped = spawn_started.elapsed();

        tracing::info!(
            serial,
            host_port,
            generation,
            preset = preset.as_str(),
            codec = stream.hello.codec,
            device = %stream.hello.device_name,
            width = first.width,
            height = first.height,
            key = first.key,
            bytes = first.bytes.len(),
            idr = crate::scrcpy::annexb_has_idr(&first.bytes),
            sps = crate::scrcpy::annexb_has_sps(&first.bytes),
            // Cumulative, so each is "by the time this step finished". Differences are the
            // per-step cost; the total is what the operator waits when a preset switch takes
            // their picture away.
            wake_ms = woke.as_millis() as u64,
            jar_ms = served.as_millis() as u64,
            sweep_ms = swept.as_millis() as u64,
            prune_ms = pruned.as_millis() as u64,
            spawn_ms = spawned.as_millis() as u64,
            forward_ms = forwarded.as_millis() as u64,
            // How long the old producer kept painting before it was handed over. On a
            // replace this is the whole spawn, and it is time the operator spent looking at
            // a *live* picture rather than a frozen one.
            swap_ms = swapped.as_millis() as u64,
            replaced = matches!(start, ViewStart::Replace),
            total_ms = spawn_started.elapsed().as_millis() as u64,
            "scrcpy view started"
        );

        let udid = serial.to_string();
        let publisher = Arc::clone(&sink);
        let frame_size = Arc::new(AtomicU32::new(pack_frame_size(first.width, first.height)));
        let reader_frame_size = Arc::clone(&frame_size);
        let first_packet = crate::view::ViewPacket {
            udid: udid.clone(),
            generation,
            kind: crate::view::ViewKind::H264,
            width: first.width,
            height: first.height,
            key: first.key,
            bytes: first.bytes,
        };
        if !publisher.publish(first_packet) {
            crate::frames::remove_forward(&self.adb, serial, host_port)
                .await
                .ok();
            let _ = child.kill().await;
            anyhow::bail!("view sink refused the first scrcpy sample");
        }
        let reader = tokio::spawn(async move {
            loop {
                match stream.next_sample().await {
                    Ok(sample) => {
                        // Before publishing, not after: a touch that races the publish should
                        // see the newer size, because the *server* has already moved to it.
                        reader_frame_size.store(
                            pack_frame_size(sample.width, sample.height),
                            Ordering::Release,
                        );
                        let packet = crate::view::ViewPacket {
                            udid: udid.clone(),
                            generation,
                            kind: crate::view::ViewKind::H264,
                            width: sample.width,
                            height: sample.height,
                            key: sample.key,
                            bytes: sample.bytes,
                        };
                        if !publisher.publish(packet) {
                            tracing::info!(udid, generation, "scrcpy view superseded; stopping");
                            return;
                        }
                    }
                    Err(error) => {
                        tracing::warn!(udid, generation, %error, "scrcpy view reader stopped");
                        return;
                    }
                }
            }
        });

        // Split so the write half can live behind its own lock while a task reads the other
        // end. `into_split` rather than `split` because the two halves outlive this function
        // in different places.
        let (mut control_read, control_write) = control.into_split();
        let control_write = Arc::new(tokio::sync::Mutex::new(control_write));
        let drain_serial = serial.to_string();
        let control_drain = tokio::spawn(async move {
            let mut scratch = [0u8; 1024];
            // Read and discard, never parse. The only thing that arrives is a clipboard
            // notification we did not ask for; the one thing that would be fatal is
            // objecting to a message type we do not know, and a reader that never
            // interprets cannot object.
            while let Ok(read) = control_read.read(&mut scratch).await {
                if read == 0 {
                    break;
                }
            }
            tracing::debug!(serial = %drain_serial, "scrcpy control socket closed");
        });

        self.views.lock().await.insert(
            serial.to_string(),
            ViewProducer {
                generation,
                preset,
                host_port,
                child,
                reader,
                frame_size,
                control: control_write,
                control_drain,
            },
        );
        Ok(())
    }
    /// Put one touch event on the phone, in the coordinate space of the picture on screen.
    ///
    /// `image_w`/`image_h` are the dimensions the *caller* was looking at when the operator
    /// moved their finger. They are not passed on: the message declares this host's latest
    /// observed frame size and the coordinates are rescaled into it. The device compares the
    /// declared size against what it is encoding and drops the event outright when they
    /// differ, so a caller one generation behind would otherwise lose the touch entirely
    /// rather than land it a few pixels off.
    ///
    /// `Ok(false)` means no producer — the overlay is not streaming this phone, so there is
    /// nothing to touch and nothing has gone wrong.
    pub async fn inject_touch(
        &self,
        serial: &str,
        action: crate::scrcpy::TouchAction,
        x: f64,
        y: f64,
        image_w: f64,
        image_h: f64,
    ) -> anyhow::Result<bool> {
        if !(image_w > 0.0 && image_h > 0.0) {
            anyhow::bail!("touch needs the size of the picture it came from");
        }
        let (control, packed) = {
            let views = self.views.lock().await;
            match views.get(serial) {
                Some(producer) => (
                    Arc::clone(&producer.control),
                    producer.frame_size.load(Ordering::Acquire),
                ),
                None => return Ok(false),
            }
        };
        let (frame_w, frame_h) = unpack_frame_size(packed);
        if frame_w == 0 || frame_h == 0 {
            anyhow::bail!("no frame seen from {serial} yet");
        }
        // Clamped, because a pointer can leave the element between two samples and a
        // coordinate outside the picture is a coordinate outside the phone.
        let scaled_x = (x / image_w * f64::from(frame_w)).round();
        let scaled_y = (y / image_h * f64::from(frame_h)).round();
        let clamped_x = scaled_x.clamp(0.0, f64::from(frame_w - 1)) as i32;
        let clamped_y = scaled_y.clamp(0.0, f64::from(frame_h - 1)) as i32;
        let message = crate::scrcpy::inject_touch(action, clamped_x, clamped_y, frame_w, frame_h);
        let mut socket = control.lock().await;
        // ONE `write_all`, under the lock, for the same reason as RESET_VIDEO: the reader on
        // the device has no framing, so an interleaved write desynchronises it permanently
        // and takes the video down with it.
        socket
            .write_all(&message)
            .await
            .with_context(|| format!("send touch to {serial}"))?;
        socket
            .flush()
            .await
            .with_context(|| format!("flush touch to {serial}"))?;
        Ok(true)
    }
    /// Ask the phone for a fresh keyframe, without restarting anything.
    ///
    /// This is what the control socket is for. The alternative cure for a decoder that has
    /// stopped producing frames is a full producer restart, measured at ~11.5 s of black
    /// tile on this fleet; a keyframe request is one byte and the server answers by logging
    /// `Video capture reset` and emitting a fresh IDR. Measured over a 75 s soak: twelve
    /// requests, twelve resets, video flowing throughout.
    ///
    /// `Ok(false)` means there is no producer to ask — not a failure, just nothing to do.
    ///
    /// The `views` lock is released before the write. Holding it across a socket send would
    /// let one unresponsive phone stall the keeper's reconciliation of the whole fleet.
    pub async fn request_keyframe(&self, serial: &str) -> anyhow::Result<bool> {
        let control = {
            let views = self.views.lock().await;
            match views.get(serial) {
                Some(producer) => Arc::clone(&producer.control),
                None => return Ok(false),
            }
        };
        let message = crate::scrcpy::reset_video();
        let mut socket = control.lock().await;
        // ONE `write_all`, under the lock. The device's reader has no framing, so a partial
        // or interleaved write desynchronises it permanently — and that is not a lost
        // message, it is the whole server going down, video included.
        socket
            .write_all(&message)
            .await
            .with_context(|| format!("send RESET_VIDEO to {serial}"))?;
        socket
            .flush()
            .await
            .with_context(|| format!("flush RESET_VIDEO to {serial}"))?;
        Ok(true)
    }
}
