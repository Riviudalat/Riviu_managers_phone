//! The helper APK: forwarding to it, checking it is the build we expect, installing it when
//! it is not, and holding the UiAutomator2 session open.
//!
//! The longest single thing in the old file and the one with the most states — an agent can
//! be absent, stale, present but not listening, or listening on a port something else took.

use super::*;

impl AndroidDriver {
    pub(super) fn host_port(&self, serial: &str) -> u16 {
        let mut ports = self.ports.lock();
        if let Some(port) = ports.get(serial) {
            return *port;
        }
        let next = HOST_PORT_BASE + ports.len() as u16;
        ports.insert(serial.to_string(), next);
        next
    }
    fn agent_base(&self, serial: &str) -> String {
        format!("http://127.0.0.1:{}", self.host_port(serial))
    }
    /// Point a host port at the agent's port on the device.
    async fn forward(&self, serial: &str) -> anyhow::Result<()> {
        let forward_spec = format!("tcp:{}", self.host_port(serial));
        let device_spec = format!("tcp:{AGENT_DEVICE_PORT}");
        self.adb
            .device(
                serial,
                &["forward", &forward_spec, &device_spec],
                adb::DEFAULT_TIMEOUT,
            )
            .await
            .context("open the adb forward to the agent")?;
        self.forwarded.lock().insert(serial.to_string());
        Ok(())
    }
    /// Whether an agent is reachable for this device, answered honestly.
    ///
    /// Devices we have never forwarded report `false` rather than borrowing
    /// somebody else's agent.
    ///
    /// The retry is not defensive padding. The tunnel is not durable: an adb
    /// server restart drops every forward while the on-device agent keeps
    /// running, and at the HTTP layer that is indistinguishable from a dead
    /// agent. Measured — the instrumentation runner exited, `adb forward
    /// --list` came back empty, `ps` still showed the server alive, and one
    /// re-forward brought `/status` straight back. Without this the tile would
    /// flap to not-ready every time the adb server bounces. The extra adb call
    /// only happens on the failing path.
    pub(super) async fn agent_ready(&self, serial: &str) -> bool {
        if !self.forwarded.lock().contains(serial) {
            return false;
        }
        let base = self.agent_base(serial);
        if AgentClient::is_ready(&base).await {
            return true;
        }
        if self.forward(serial).await.is_err() {
            return false;
        }
        AgentClient::is_ready(&base).await
    }
    /// The pid of a package, or `None` when it is not running.
    ///
    /// `pidof` exits non-zero for an absent process, which the adb wrapper
    /// reports as a command failure. Absence is an answer here, not an error —
    /// propagating it made `inspect_app_process` fail precisely when it was
    /// asked about a stopped app, which is the case it exists to describe.
    ///
    /// `bundle_id` must already have passed [`adb::validate_package_name`];
    /// every public caller checks it before reaching here.
    pub(super) async fn pid_of(&self, serial: &str, bundle_id: &str) -> Option<u64> {
        self.adb
            .shell(serial, &format!("pidof {bundle_id}"))
            .await
            .ok()
            .and_then(|stdout| adb::parse_pidof(&stdout))
    }
    async fn screen_size(&self, serial: &str) -> anyhow::Result<(f64, f64)> {
        let stdout = self.adb.shell(serial, "wm size").await?;
        let (width, height) = adb::parse_wm_size(&stdout)
            .ok_or_else(|| anyhow!("could not read the screen size from 'wm size'"))?;
        Ok((f64::from(width), f64::from(height)))
    }
    /// `versionName` and `versionCode` for one installed package.
    ///
    /// A package that is not installed is a distinct outcome from one whose dump could not
    /// be parsed, and both say so: Flow's preflight message is the only thing an operator
    /// gets when a run refuses, so "TikTok is not installed" must not arrive as "could not
    /// read the version".
    pub(super) async fn package_identity(
        &self,
        serial: &str,
        package: &str,
    ) -> anyhow::Result<crate::capability::PackageIdentity> {
        let package = adb::validate_package_name(package)?;
        let dumpsys = self
            .adb
            .shell(serial, &format!("dumpsys package {package}"))
            .await
            .with_context(|| format!("read the installed record for {package} on {serial}"))?;
        let version = riviu_core::tiktok_labels::parse_version_name(&dumpsys);
        let build = riviu_core::tiktok_labels::parse_version_code(&dumpsys);
        match (version, build) {
            (Some(version), Some(build)) => Ok(crate::capability::PackageIdentity {
                package: package.to_string(),
                version: version.to_string(),
                build: build.to_string(),
            }),
            _ if !dumpsys.contains(&format!("Package [{package}]")) => {
                Err(anyhow!("{package} is not installed on {serial}"))
            }
            _ => Err(anyhow!(
                "{package} is installed on {serial} but its version could not be read from \
                 `dumpsys package`"
            )),
        }
    }
    /// SHA-256 of an installed package's APK, computed on the device.
    ///
    /// `pm path` then `sha256sum`, in one shell round trip — measured 225 ms end to end on
    /// an SM-G955F, which is affordable on a path that runs once per device per Flow run.
    ///
    /// The two are chained on the phone rather than here so the path never crosses back
    /// through the host: `pm path` prints `package:/data/app/…/base.apk`, and a serial with
    /// two installed splits would otherwise need the host to decide which line to hash.
    pub(super) async fn installed_apk_sha256(
        &self,
        serial: &str,
        package: &str,
    ) -> anyhow::Result<String> {
        let package = adb::validate_package_name(package)?;
        let stdout = self
            .adb
            .shell(
                serial,
                &format!("sha256sum \"$(pm path {package} | head -n 1 | cut -d: -f2)\""),
            )
            .await
            .with_context(|| format!("hash the installed {package} APK on {serial}"))?;
        let digest = stdout.split_whitespace().next().unwrap_or_default();
        if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(anyhow!(
                "could not hash the installed {package} APK on {serial}: `sha256sum` \
                 answered {stdout:?}"
            ));
        }
        Ok(digest.to_ascii_lowercase())
    }
    /// The screen as it is being rendered right now, rotation included.
    ///
    /// **`dumpsys display`, not `wm size`.** The latter reports the display's base
    /// configuration, which has no orientation in it, so a landscape phone answers with its
    /// portrait dimensions and every coordinate derived from them is wrong (AGENTS.md
    /// §9.59, and the doc on [`adb::parse_display_geometry`]).
    pub(super) async fn display_geometry(
        &self,
        serial: &str,
    ) -> anyhow::Result<adb::DisplayGeometry> {
        let stdout = self.adb.shell(serial, "dumpsys display").await?;
        adb::parse_display_geometry(&stdout).ok_or_else(|| {
            anyhow!(
                "could not read the current display geometry from `dumpsys display` on {serial}"
            )
        })
    }
    /// Instrumentation packages on the phone that are not ours.
    ///
    /// **Android lets exactly one UiAutomator instrumentation hold the accessibility connection**,
    /// so a leftover automation tool from another product does not merely coexist: it makes every
    /// tap and every tree read fail, on that phone, permanently, with no error that names a cause.
    /// GenFarmer's own survey warns about this from the other side -- its test bench carried
    /// `com.genfarmer.uiautomator{,.test}` and the note says plainly that one of the two tools has
    /// to be stopped before the other runs.
    ///
    /// Pure, because the value is the *parsing*: `pm list instrumentation` prints
    /// `instrumentation:<pkg>/<runner> (target=<pkg>)`, and a listing read wrongly would either
    /// accuse an innocent phone or clear a guilty one.
    pub(super) fn foreign_instrumentations(listing: &str, ours: &[&str]) -> Vec<String> {
        let mut found = Vec::new();
        for line in listing.lines() {
            let line = line.trim();
            let Some(rest) = line.strip_prefix("instrumentation:") else {
                continue;
            };
            // `<pkg>/<runner>` up to the first space; the `(target=..)` suffix is not part of it.
            let component = rest.split_whitespace().next().unwrap_or(rest);
            let package = component.split('/').next().unwrap_or(component);
            if package.is_empty() || ours.contains(&package) {
                continue;
            }
            if !found.iter().any(|seen: &String| seen == component) {
                found.push(component.to_string());
            }
        }
        found
    }

    /// Turn "something else may be holding UiAutomation" into the name of that something.
    ///
    /// The two messages below already carried the right hypothesis and never checked it, so an
    /// operator was told to go looking for an unnamed automation tool. One `pm list
    /// instrumentation` settles it.
    ///
    /// Best-effort by design: this runs on a path that is already failing, and a phone that
    /// cannot answer this question must not turn a useful error into a different error. No
    /// answer means the sentence simply keeps its old, weaker form.
    async fn foreign_instrumentation_note(&self, serial: &str) -> String {
        let Ok(listing) = self.adb.shell(serial, "pm list instrumentation").await else {
            return String::new();
        };
        let foreign =
            Self::foreign_instrumentations(&listing, &[AGENT_PACKAGE, AGENT_TEST_PACKAGE]);
        if foreign.is_empty() {
            // Worth saying too: it rules out the most likely cause, which is what stops the
            // next hour being spent on it.
            return " Không có instrumentation lạ nào trên máy này, nên nguyên nhân nằm ở chỗ khác."
                .to_string();
        }
        format!(
            " Máy này còn instrumentation của tool khác: {}. Android chỉ cho MỘT UiAutomator giữ \
             accessibility, nên phải xoá hoặc dừng nó trước.",
            foreign.join(", ")
        )
    }

    /// The instrumentation component this driver starts to bring the agent up.
    pub(super) fn agent_runner() -> String {
        format!("{AGENT_TEST_PACKAGE}/{AGENT_RUNNER}")
    }
    /// `(model, release)` — what a capability snapshot calls product type and OS version.
    ///
    /// Read fresh rather than taken from the cached `DeviceInfo`, because that one carries
    /// a model *hint* from `adb devices -l` which can be the codename (`dream2lte`) rather
    /// than the marketing model (`SM-G955F`), and this value is hashed into a device
    /// profile id that has to mean the same thing every time it is computed.
    pub(super) async fn device_identity(&self, serial: &str) -> anyhow::Result<(String, String)> {
        let stdout = self
            .adb
            .shell(
                serial,
                &format!(
                    "getprop ro.product.model; echo {sep}; getprop ro.build.version.release",
                    sep = FIELD_SEPARATOR
                ),
            )
            .await?;
        let mut sections = stdout.split(FIELD_SEPARATOR);
        let model = sections.next().unwrap_or_default().trim().to_string();
        let release = sections.next().unwrap_or_default().trim().to_string();
        if model.is_empty() || release.is_empty() {
            return Err(anyhow!(
                "could not read the model and Android release from {serial}"
            ));
        }
        Ok((model, release))
    }
    /// Remember what we last proved about a serial's agent, for the synchronous readers.
    ///
    /// `DeviceDriver::cached_agent_status` cannot await, and Flow's preflight reads it to
    /// decide whether the phone has a usable control surface. So every path that learns
    /// something about the agent records it here, exactly as the iOS driver does with
    /// `agent_statuses`.
    pub(super) fn publish_agent_status(&self, status: riviu_core::AgentStatus) {
        self.agent_statuses
            .lock()
            .insert(status.udid.clone(), status);
    }
    pub(super) fn agent_status_for(
        &self,
        serial: &str,
        state: riviu_core::AgentState,
        identity: Option<&crate::capability::PackageIdentity>,
        message: Option<String>,
    ) -> riviu_core::AgentStatus {
        let ready = state == riviu_core::AgentState::Ready;
        riviu_core::AgentStatus {
            udid: serial.to_string(),
            state,
            artifact_id: AGENT_PACKAGE.to_string(),
            artifact_version: identity
                .map(|value| value.version.clone())
                .unwrap_or_default(),
            bundle_id: AGENT_PACKAGE.to_string(),
            protocol_version: crate::capability::PROTOCOL_VERSION,
            // What the agent can do is a property of this driver, not of the install: the
            // uiautomator2 server on any phone this project drives does all four, and the
            // measurements behind that claim are in `agent.rs`. Reporting them only when
            // ready keeps a phone that cannot be driven from advertising capabilities.
            features: if ready {
                ["stream", "tap", "swipe", "text"]
                    .iter()
                    .map(|value| value.to_string())
                    .collect()
            } else {
                Vec::new()
            },
            installed_version: identity.map(|value| value.version.clone()),
            installed_build: identity.map(|value| value.build.clone()),
            // No token to be ready or not: the uiautomator2 server has no auth. What this
            // stands for on Android is the same thing `protected_auth_ready` stands for in
            // the snapshot — the control surface answered and could see.
            auth_ready: ready,
            mjpeg_ready: ready,
            session_ready: ready,
            message,
        }
    }
    pub async fn open_session(&self, udid: &str) -> anyhow::Result<AndroidUiSession> {
        let agent = self.ensure_agent(udid).await?;
        let screen = {
            let cache = self
                .screens
                .lock()
                .entry(udid.to_string())
                .or_default()
                .clone();
            // Seed from `wm size` only when there is nothing cached. It is the right *seed*
            // -- available before the agent is primed -- and the wrong *refresh*, because it
            // does not follow rotation (see `ScreenCache`). Re-opening a session for a phone
            // we already know now costs no adb round trip at all.
            if cache.peek().is_none() {
                cache.store(self.screen_size(udid).await?);
            }
            cache
        };
        let helper = match self.try_attach_helper(udid).await {
            Ok(helper) => helper,
            Err(error) => {
                tracing::warn!(
                    serial = udid,
                    %error,
                    "Riviu helper is not attached; clipboard stays unsupported"
                );
                None
            }
        };
        Ok(
            // `new` still takes a tuple so the public constructor is unchanged; the shared
            // handle replaces the private cache it seeds.
            AndroidUiSession::new(agent, self.adb.clone(), udid.to_string(), (0.0, 0.0))
                .with_screen_cache(screen)
                .with_helper(helper),
        )
    }
    /// Attach the helper when it is already on the phone, or when an APK is
    /// configured so we can install it. Missing both is normal and not an
    /// error — nurture must not die because clipboard is unavailable.
    pub(super) async fn try_attach_helper(
        &self,
        serial: &str,
    ) -> anyhow::Result<Option<crate::riviu_agent::HelperClient>> {
        let cached = self.helpers.lock().get(serial).cloned();
        if let Some(helper) = cached {
            if helper.is_alive().await {
                return Ok(Some(helper));
            }
            self.helpers.lock().remove(serial);
        }
        let installed = self
            .adb
            .shell(serial, &format!("pm path {}", crate::riviu_agent::PACKAGE))
            .await
            .unwrap_or_default()
            .contains("package:");
        if !installed && self.riviu_agent_apk.is_none() {
            return Ok(None);
        }
        let helper = crate::riviu_agent::HelperClient::ensure(
            self.adb.clone(),
            serial,
            self.riviu_agent_apk.as_deref(),
        )
        .await?;
        self.helpers
            .lock()
            .insert(serial.to_string(), helper.clone());
        Ok(Some(helper))
    }
    /// Make sure the agent is installed, running and forwarded.
    pub(super) async fn ensure_agent(&self, serial: &str) -> anyhow::Result<AgentClient> {
        let base = self.agent_base(serial);
        self.forward(serial).await?;

        // Reuse the session we already have. Opening a second one costs the whole
        // fleet: see the note on `Self::agents`.
        let cached = self.agents.lock().get(serial).cloned();
        if let Some(agent) = cached {
            if agent.is_alive().await {
                return Ok(agent);
            }
            // Dead, and still registered on the device. Ask the server to forget it
            // before opening another, or the leak this cache exists to stop happens
            // one session at a time anyway.
            let _ = agent.close().await;
            self.agents.lock().remove(serial);
        }

        // A server that answers `/status` usually just needs a fresh session — that is the
        // rotten-session case `AgentClient::recycle` documents. But `/status` does not prove
        // the accessibility tree is readable, and when it is not, a new session against the
        // same server is just as blind: measured on an SM-N950F on 12/08/2026, where an
        // out-of-band `uiautomator dump` had taken `UiAutomation` away and `open_session`
        // happily returned a 4040 ms session whose every element query then blocked.
        //
        // So the new session has to prove itself, and the fall-through is to restart the
        // instrumentation rather than to hand back something that cannot see.
        if AgentClient::is_ready(&base).await {
            // **Both ways of failing here lead to the same restart, and until 17/08/2026 one
            // of them led nowhere.** Losing `UiAutomation` has two presentations, and this
            // path only ever handled the first:
            //
            //   1. the session opens and every query blocks — caught by `is_alive`;
            //   2. the session does not open at all, `SessionNotCreatedException:
            //      java.lang.IllegalStateException: UiAutomation not connected!`, in 137 ms.
            //
            // Reproduced on this fleet with an out-of-band `adb shell uiautomator dump`: the
            // phone lands in (1), the restart runs, and afterwards it sits in (2) — where the
            // `?` on this line returned the Java exception straight to the operator and the
            // recovery below was unreachable, because proving the server broken required a
            // session and the breakage was that no session could be had. Every tap failed,
            // forever, and nothing ever tried to fix it.
            //
            // A server that answers `/status` and will not give a session is wedged whatever
            // the message says, so the failure is not inspected: it is treated exactly like a
            // blind session.
            let opened = self.open_and_cache_agent(serial, &base).await;
            match opened {
                Ok(agent) if agent.is_alive().await => return Ok(agent),
                Ok(agent) => {
                    let _ = agent.close().await;
                    self.agents.lock().remove(serial);
                }
                Err(error) => {
                    tracing::warn!(
                        serial,
                        %error,
                        "the agent answers /status but will not open a session"
                    );
                    self.agents.lock().remove(serial);
                }
            }
            // A restart we already tried and that did not take. Refuse rather than repeat:
            // the holder of `UiAutomation` is on the phone, and a second restart inside the
            // window races the same holder for another twenty seconds of the operator's
            // time. Failing here is not giving up — it is the difference between one clear
            // message and a minute of silence.
            if let Some(since) = self.since_instrumentation_restart(serial) {
                if since < INSTRUMENTATION_RESTART_COOLDOWN {
                    let quiet_for = INSTRUMENTATION_RESTART_COOLDOWN - since;
                    // Said out loud, not just returned. The error reaches whoever made this
                    // call; the log is what tells the next person why a phone spent ten
                    // minutes refusing every gesture without a single restart in sight.
                    tracing::warn!(
                        serial,
                        since_s = since.as_secs(),
                        quiet_for_s = quiet_for.as_secs(),
                        "refusing to restart the instrumentation again inside its cooldown"
                    );
                    let note = self.foreign_instrumentation_note(serial).await;
                    anyhow::bail!(
                        "the agent on {serial} is listening but cannot read the accessibility \
                         tree, and its instrumentation was already restarted {:.0}s ago \
                         without fixing it.{note} Not restarting again for another {:.0}s.",
                        since.as_secs_f64(),
                        quiet_for.as_secs_f64()
                    );
                }
            }
            let note = self.foreign_instrumentation_note(serial).await;
            tracing::warn!(
                serial,
                "the agent answers /status but cannot read the accessibility tree — \
                 restarting the instrumentation.{note}"
            );
            self.note_instrumentation_restart(serial);
            let started = std::time::Instant::now();
            self.restart_instrumentation(serial).await?;
            let recovered = self.instrument_and_wait(serial, &base).await;
            // Logged either way, because the cost of this path is the whole reason it now
            // has a cooldown and nobody should have to induce the fault to find it again.
            tracing::info!(
                serial,
                ms = started.elapsed().as_millis() as u64,
                ok = recovered.is_ok(),
                "instrumentation restart finished"
            );
            return recovered;
        }

        let installed = self
            .adb
            .shell(serial, &format!("pm list packages {AGENT_PACKAGE}"))
            .await
            .unwrap_or_default();
        if !installed.contains(AGENT_PACKAGE) {
            self.install_agent_apks(serial).await?;
        }

        self.instrument_and_wait(serial, &base).await
    }
    /// How long since this device's instrumentation was last restarted for blindness.
    fn since_instrumentation_restart(&self, serial: &str) -> Option<Duration> {
        self.instrumentation_restarts
            .lock()
            .get(serial)
            .map(|at| at.elapsed())
    }
    fn note_instrumentation_restart(&self, serial: &str) {
        self.instrumentation_restarts
            .lock()
            .insert(serial.to_string(), std::time::Instant::now());
    }
    /// Start the runner and wait for a session that can actually read the screen.
    async fn instrument_and_wait(&self, serial: &str, base: &str) -> anyhow::Result<AgentClient> {
        self.spawn_instrumentation(serial)?;
        // The server binds its port a beat after the runner starts.
        for _ in 0..AGENT_READY_POLLS {
            if AgentClient::is_ready(base).await {
                let agent = self.open_and_cache_agent(serial, base).await?;
                if agent.is_alive().await {
                    return Ok(agent);
                }
                // Bound to the port but blind. Reported rather than retried forever: a
                // second restart would race the same holder of `UiAutomation`, and the
                // operator needs to know something else on the phone has it.
                let _ = agent.close().await;
                self.agents.lock().remove(serial);
                return Err(anyhow!(
                    "the agent on {serial} is listening but cannot read the accessibility \
                     tree even after a restart. Something else holds UiAutomation — an \
                     `adb shell uiautomator dump`, or another automation tool on the phone"
                ));
            }
            tokio::time::sleep(AGENT_READY_POLL_EVERY).await;
        }
        Err(anyhow!(
            "the agent on {serial} did not answer /status within {:.0} seconds",
            AGENT_READY_WAIT.as_secs_f64()
        ))
    }
    /// Stop the running instrumentation so the next start is a clean one.
    ///
    /// Force-stopping both halves is what actually recovered the phone by hand on
    /// 12/08/2026 — `open_session` then re-instrumented and answered in 4040 ms. The
    /// server holds `UiAutomation` for its lifetime, so nothing short of ending the
    /// process gets it back.
    /// Push and install both halves of the uiautomator2 instrumentation.
    ///
    /// Until 16/08/2026 this was a message telling the operator to install two APKs the
    /// app did not ship. Measured on a freshly plugged 20-device Galaxy S8 box: video
    /// worked on 20/20 because `scrcpy-server` is bundled and pushed, and control worked
    /// on **0/20** because nothing pushed these. Telling someone to install a file that is
    /// not in the box is not an error message, it is a missing feature.
    ///
    /// `-r -g -t`: reinstall over a stale copy, grant the runtime permissions the server
    /// needs without a dialog, and allow a test-only APK -- the `androidTest` half is
    /// built with `android:testOnly`, which `pm install` refuses by default.
    async fn install_agent_apks(&self, serial: &str) -> anyhow::Result<()> {
        let Some((server, test)) = self.agent_apks.as_ref() else {
            return Err(anyhow!(
                "the agent is not installed on {serial} and this build has no agent APK \
                 to install. Set RIVIU_AGENT_SERVER_APK and RIVIU_AGENT_TEST_APK, or use \
                 an installer that bundles them"
            ));
        };
        // Server first: the test APK declares an instrumentation targeting the server's
        // package, and installing it against a missing target fails on some builds.
        for (apk, package) in [(server, AGENT_PACKAGE), (test, AGENT_TEST_PACKAGE)] {
            let path = apk.to_string_lossy().to_string();
            tracing::info!(serial, package, apk = %path, "installing the uiautomator2 agent");
            self.adb
                .run(
                    &["-s", serial, "install", "-r", "-g", "-t", &path],
                    INSTALL_TIMEOUT,
                )
                .await
                .with_context(|| format!("install {package} on {serial} from {path}"))?;
        }
        // Prove it rather than trust the exit code: `pm install` has been observed to
        // report success for a package that is not then listed.
        let installed = self
            .adb
            .shell(serial, &format!("pm list packages {AGENT_PACKAGE}"))
            .await
            .unwrap_or_default();
        anyhow::ensure!(
            installed.contains(AGENT_PACKAGE),
            "installed the agent on {serial} but `pm list packages` still does not show              {AGENT_PACKAGE}"
        );
        Ok(())
    }
    async fn restart_instrumentation(&self, serial: &str) -> anyhow::Result<()> {
        for package in [AGENT_PACKAGE, AGENT_TEST_PACKAGE] {
            if let Err(error) = self
                .adb
                .shell(serial, &format!("am force-stop {package}"))
                .await
            {
                tracing::warn!(serial, package, %error, "could not stop the agent half");
            }
        }
        // The port stays bound for a moment after the process goes.
        tokio::time::sleep(Duration::from_millis(600)).await;
        Ok(())
    }
    /// Open one session and remember it for this serial.
    async fn open_and_cache_agent(&self, serial: &str, base: &str) -> anyhow::Result<AgentClient> {
        let agent = AgentClient::connect(serial, base).await?;
        self.agents.lock().insert(serial.to_string(), agent.clone());
        Ok(agent)
    }
    /// Start the instrumentation runner and let it keep running.
    ///
    /// `am instrument -w` blocks for the life of the server, so the child is
    /// detached deliberately rather than awaited.
    fn spawn_instrumentation(&self, serial: &str) -> anyhow::Result<()> {
        let mut command = tokio::process::Command::new(self.adb.path());
        command
            .args([
                "-s",
                serial,
                "shell",
                "am",
                "instrument",
                "-w",
                "-e",
                "disableAnalytics",
                "true",
                &format!("{AGENT_TEST_PACKAGE}/{AGENT_RUNNER}"),
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(windows)]
        command.creation_flags(0x0800_0000);
        command
            .spawn()
            .with_context(|| format!("start the agent on {serial}"))?;
        Ok(())
    }
}

#[cfg(test)]
mod instrumentation_tests {
    use super::super::AndroidDriver;

    /// The real shape of `pm list instrumentation`, and the one line that matters in it.
    ///
    /// **Android lets exactly one UiAutomator instrumentation hold the accessibility
    /// connection.** A leftover from another automation product does not coexist -- it makes
    /// every tap and every tree read fail on that phone, permanently, and the error that surfaced
    /// used to say only that "something else may be holding UiAutomation". This is what turns
    /// that into a package name.
    #[test]
    fn a_foreign_instrumentation_is_named_and_ours_is_not() {
        let listing = "instrumentation:io.appium.uiautomator2.server.test/androidx.test.runner.AndroidJUnitRunner (target=io.appium.uiautomator2.server)\n\
             instrumentation:com.genfarmer.uiautomator.test/androidx.test.runner.AndroidJUnitRunner (target=com.genfarmer.uiautomator)\n";

        let foreign = AndroidDriver::foreign_instrumentations(
            listing,
            &[
                "io.appium.uiautomator2.server",
                "io.appium.uiautomator2.server.test",
            ],
        );
        assert_eq!(
            foreign,
            vec![
                "com.genfarmer.uiautomator.test/androidx.test.runner.AndroidJUnitRunner"
                    .to_string()
            ],
            "ours must not be reported, and theirs must be named with its runner"
        );
    }

    /// A clean phone reports nothing, so the message can rule the cause out rather than repeat it.
    #[test]
    fn a_phone_carrying_only_our_agent_is_clean() {
        let listing = "instrumentation:io.appium.uiautomator2.server.test/androidx.test.runner.AndroidJUnitRunner (target=io.appium.uiautomator2.server)\n";
        assert!(AndroidDriver::foreign_instrumentations(
            listing,
            &["io.appium.uiautomator2.server.test"]
        )
        .is_empty());
    }

    /// **Parsing is the whole value here, so the awkward inputs are pinned.**
    ///
    /// A listing read wrongly either accuses an innocent phone or clears a guilty one, and both
    /// send somebody to the wrong place for an hour.
    #[test]
    fn the_listing_is_parsed_rather_than_pattern_matched_loosely() {
        // No trailing `(target=..)`, which some ROMs omit.
        assert_eq!(
            AndroidDriver::foreign_instrumentations("instrumentation:a.b/c.Runner", &[]),
            vec!["a.b/c.Runner".to_string()]
        );
        // Lines that are not instrumentation entries at all.
        assert!(AndroidDriver::foreign_instrumentations(
            "package:com.something\nSecurity exception\n\n",
            &[]
        )
        .is_empty());
        // The same component twice is one finding.
        assert_eq!(
            AndroidDriver::foreign_instrumentations(
                "instrumentation:a.b/c.R (target=a.b)\ninstrumentation:a.b/c.R (target=a.b)",
                &[]
            )
            .len(),
            1
        );
        // Empty output from a phone that refused the question is not an accusation.
        assert!(AndroidDriver::foreign_instrumentations("", &[]).is_empty());
    }
}
