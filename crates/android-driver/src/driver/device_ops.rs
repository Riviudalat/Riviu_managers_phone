//! One-shot operations against a phone: radios, wallpaper, mock location, root, the
//! filesystem, display metrics, power and the input method.
//!
//! What they have in common is shape rather than subject — each is a short adb round trip
//! that either worked or did not, leaving no process running behind it. That is precisely
//! what the stream half is not, which is why the two were worth separating.

use super::*;

/// The two facts one `dumpsys window` answers about who can be driven.
///
/// Read together because they explain each other: a `System` foreground with `locked:
/// Some(true)` is a lock screen, the same reading with `locked: None` is a build whose
/// keyguard keys are not printed, and an `App` foreground with `locked: Some(true)` is a
/// keyguard occluded by an app that is nonetheless not reachable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScreenGuardState {
    /// `None` means the dump carried none of the three keyguard keys — unknown, which is
    /// not the same as unlocked.
    pub locked: Option<bool>,
    pub foreground: adb::ForegroundWindow,
}

impl ScreenGuardState {
    /// Whether this phone is behind a lock screen, judged from both facts.
    ///
    /// The keyguard keys are the evidence when they are there. When they are not, a system
    /// window in front is the next best thing — measured on this fleet, `StatusBar` owning
    /// focus was true of exactly the two locked phones and of none of the twelve others.
    pub fn behind_lock_screen(&self) -> bool {
        match self.locked {
            Some(locked) => locked,
            None => matches!(self.foreground, adb::ForegroundWindow::System(_)),
        }
    }

    /// The operator-facing name of what is in the way, for a message that names it.
    pub fn blocker(&self) -> Option<&str> {
        match &self.foreground {
            adb::ForegroundWindow::System(name) => Some(name.as_str()),
            adb::ForegroundWindow::App(_) | adb::ForegroundWindow::Unreadable => None,
        }
    }
}

/// The name a pulled path keeps once it is on this host.
///
/// `adb pull` is given the destination **directory**, so the phone's own name for the file is
/// what it lands as -- keeping the phone's name is what stops an export of twenty phones from
/// becoming twenty files whose origin is only in the log. This works out the name the same way,
/// because the caller then has to check that exact path exists: `adb pull` has been seen to
/// report success for a directory it produced nothing from.
///
/// Pure and extracted so the awkward inputs can be pinned. `"/sdcard/"` and `"/"` both trim to
/// nothing, and a `rsplit` on the result yields an empty string rather than `None` -- which is
/// why the filter is there. Without it the landed path would be the destination directory
/// itself, and the existence check would pass for the wrong reason: the directory always exists.
pub(super) fn pulled_name(remote: &str) -> &str {
    remote
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .filter(|part| !part.is_empty())
        .unwrap_or("pulled")
}

/// Where a pushed file lands on the phone.
///
/// Pure and extracted for the same reason as [`pulled_name`]: this string is what `adb push`
/// writes to and what the read-back then looks for, so a wrong one either writes somewhere
/// unexpected or reports a success that did not happen. The trailing-slash case is the one worth
/// pinning -- the file manager's path box and its breadcrumbs disagree about whether a directory
/// ends in `/`, so both forms reach here, and `//` in a device path is not something to hand to
/// a shell.
pub(super) fn pushed_target(remote_dir: &str, file_name: &str) -> String {
    format!("{}/{file_name}", remote_dir.trim_end_matches('/'))
}

impl AndroidDriver {
    /// Put a USB-attached phone into TCP/IP adb mode, discover its Wi-Fi address, and
    /// `adb connect` to it — so it can be driven over the LAN without the cable (feature A4,
    /// xiaowei WIFI mode). Returns the `host:port` now connected. The USB serial keeps
    /// working; the wireless endpoint shows up as an additional device on the next refresh.
    pub async fn enable_wifi_adb(&self, serial: &str) -> anyhow::Result<String> {
        // 5555 is adb's conventional wireless port and what `adb tcpip` restarts adbd on.
        self.adb
            .run(
                &["-s", serial, "tcpip", "5555"],
                std::time::Duration::from_secs(10),
            )
            .await
            .map_err(|e| anyhow::anyhow!("adb tcpip failed: {e}"))?;
        // adbd restarts; give it a beat before asking the phone for its address.
        tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
        let out = self
            .adb
            .shell(serial, adb::WLAN_IP_SHELL)
            .await
            .map_err(|e| anyhow::anyhow!("read wlan0 address failed: {e}"))?;
        let ip = adb::parse_wlan_ipv4(&out)
            .ok_or_else(|| anyhow::anyhow!("no Wi-Fi (wlan0) address — is the phone on Wi-Fi?"))?;
        let host = format!("{ip}:5555");
        self.wifi_connect(&host).await?;
        Ok(host)
    }
    /// `adb connect <host:port>` to a phone already in TCP/IP mode (manual entry, or after
    /// [`Self::enable_wifi_adb`]). adb prints "connected"/"already connected" on success and a
    /// human reason on failure, which is surfaced verbatim.
    pub async fn wifi_connect(&self, host: &str) -> anyhow::Result<()> {
        let out = self
            .adb
            .run(&["connect", host], std::time::Duration::from_secs(10))
            .await
            .map_err(|e| anyhow::anyhow!("adb connect failed: {e}"))?;
        let low = out.to_lowercase();
        if low.contains("connected") {
            Ok(())
        } else {
            Err(anyhow::anyhow!("adb connect {host}: {}", out.trim()))
        }
    }
    /// `adb disconnect <host:port>`, dropping a wireless endpoint (the USB side, if any, is
    /// unaffected).
    ///
    /// Note what this does **not** do: adbd on the phone keeps listening on `0.0.0.0:5555`.
    /// This only drops *this host's* client connection. To close the port, see
    /// [`Self::disable_wifi_adb`].
    pub async fn wifi_disconnect(&self, host: &str) -> anyhow::Result<()> {
        self.adb
            .run(&["disconnect", host], std::time::Duration::from_secs(10))
            .await
            .map_err(|e| anyhow::anyhow!("adb disconnect failed: {e}"))?;
        Ok(())
    }
    /// Put adbd back on USB, closing the TCP/IP port it was listening on.
    ///
    /// The way back that did not exist. `enable_wifi_adb` restarts adbd on `0.0.0.0:5555`, and
    /// on Android 9 that port is gated only by the RSA host-key prompt — on farm phones where
    /// "always allow" was tapped once (which is the point of a farm), anyone on the LAN who
    /// gets a key trusted has full `adb shell`, which on the rooted subset is root. Until now
    /// nothing in this codebase ran `adb usb`, so the only way to close the port again was to
    /// reboot the phone, and nothing in the UI said so.
    ///
    /// Takes the **USB** serial deliberately: `adb usb` has to be addressed to the device, and
    /// asking over the wireless transport would be sawing off the branch — the command that
    /// closes the port would travel through the port it closes.
    pub async fn disable_wifi_adb(&self, serial: &str) -> anyhow::Result<()> {
        let out = self
            .adb
            .run(&["-s", serial, "usb"], std::time::Duration::from_secs(10))
            .await
            .map_err(|e| anyhow::anyhow!("adb usb failed: {e}"))?;
        // adb prints "restarting in USB mode" on success. Some builds print nothing at all and
        // still switch, so an empty answer is accepted; only an explicit error is refused.
        if out.to_lowercase().contains("error") {
            anyhow::bail!("adb usb {serial}: {}", out.trim());
        }
        Ok(())
    }
    /// Set the phone's wallpaper from a local image file (feature A3, "number as wallpaper").
    /// Pushes the file to the device then asks the helper to apply it. Requires the Riviu
    /// helper APK (bundled); errors clearly if it is not attachable.
    pub async fn set_wallpaper(&self, serial: &str, local_path: &str) -> anyhow::Result<()> {
        let helper = self
            .try_attach_helper(serial)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Riviu helper không sẵn sàng trên máy này"))?;
        let device_path = "/data/local/tmp/riviu-wallpaper.png";
        self.adb
            .run(
                &["-s", serial, "push", local_path, device_path],
                std::time::Duration::from_secs(30),
            )
            .await
            .map_err(|e| anyhow::anyhow!("push wallpaper failed: {e}"))?;
        helper.set_wallpaper(device_path).await
    }
    /// Inject a mock GPS location (feature B). Grants the helper the mock-location appop
    /// (best-effort — on some builds it must be set once in Developer Options) then asks the
    /// helper to feed the coordinates into the GPS/network providers.
    pub async fn set_mock_location(&self, serial: &str, lat: f64, lng: f64) -> anyhow::Result<()> {
        let helper = self
            .try_attach_helper(serial)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Riviu helper không sẵn sàng trên máy này"))?;
        // Best-effort: needs WRITE_SECURE_SETTINGS-level appop, which `adb shell` has.
        let _ = self
            .adb
            .shell(
                serial,
                &format!(
                    "appops set {} android:mock_location allow",
                    crate::riviu_agent::PACKAGE
                ),
            )
            .await;
        helper.set_mock_location(lat, lng).await
    }
    /// Stop mock location, returning the phone to its real GPS (feature B).
    ///
    /// Revokes the appop as well as removing the test provider. Without that, one use of "set
    /// location" left `com.riviu.agent` as the phone's selected mock-location app **forever** —
    /// a permission that normally requires a human to pick the app in Developer Options, handed
    /// out permanently. Combined with an unauthenticated helper that is a standing GPS-spoofing
    /// capability for any other app on the device, long after the operator finished with it.
    ///
    /// Revoke is best-effort and runs even when the helper is unreachable: the appop is granted
    /// by adb, not by the helper, so a helper that has died must not strand the grant.
    pub async fn stop_mock_location(&self, serial: &str) -> anyhow::Result<()> {
        let stopped = match self.try_attach_helper(serial).await {
            Ok(Some(helper)) => helper.stop_mock_location().await,
            Ok(None) => Err(anyhow::anyhow!("Riviu helper không sẵn sàng trên máy này")),
            Err(error) => Err(error),
        };
        // **The revoke is the security-relevant half, so its failure has to be said out
        // loud.** This was `let _ = ...`: the helper could remove its test provider, the
        // `appops deny` could then be refused by the ROM or lost to a dropped transport, and
        // the method still returned `Ok(())`. `com.riviu.agent` stays the phone's selected
        // mock-location app -- a permission that normally needs a human in Developer Options
        // -- with the API reporting that it had been revoked. The doc comment above promises
        // the revoke; this makes the promise checkable.
        //
        // Still best-effort in the sense that matters: it runs even when the helper is
        // unreachable, because the appop is granted by adb rather than by the helper.
        let revoked = self
            .adb
            .shell(
                serial,
                &format!(
                    "appops set {} android:mock_location deny",
                    crate::riviu_agent::PACKAGE
                ),
            )
            .await;
        match (stopped, revoked) {
            (Ok(()), Ok(_)) => Ok(()),
            // The provider is gone but the grant is not. That is the dangerous half, so it
            // wins over a clean stop.
            (Ok(()), Err(error)) => Err(anyhow!(
                "đã bỏ vị trí giả nhưng KHÔNG thu hồi được quyền mock_location của \
                 {}: {error}",
                crate::riviu_agent::PACKAGE
            )),
            (Err(stop_error), Ok(_)) => Err(stop_error),
            (Err(stop_error), Err(revoke_error)) => Err(anyhow!(
                "{stop_error}; và cũng không thu hồi được quyền mock_location: {revoke_error}"
            )),
        }
    }

    // --- Root tier (feature C, xiaowei "ROOT 模式 / 一键新机"). These need a rooted phone
    // (Magisk `su`); on a non-rooted phone `is_rooted` returns false and the mutating calls
    // report that rather than half-applying. Only android_id can be set without root (adb
    // carries WRITE_SECURE_SETTINGS). ---
    /// True when `su` grants uid 0 — i.e. the phone is rooted (Magisk) and has authorised adb.
    pub async fn is_rooted(&self, serial: &str) -> bool {
        match self.adb.shell(serial, "su -c id").await {
            Ok(out) => out.contains("uid=0"),
            Err(_) => false,
        }
    }
    /// Run a command as root (`su -c`). Errors if the phone is not rooted, rather than
    /// silently running it unprivileged.
    pub async fn root_shell(&self, serial: &str, command: &str) -> anyhow::Result<String> {
        if !self.is_rooted(serial).await {
            anyhow::bail!("máy chưa root (không có su)");
        }
        // Double quotes so the caller's command keeps its own single quotes; callers here
        // pass fixed commands, not operator free-text.
        self.adb
            .shell(serial, &format!("su -c \"{command}\""))
            .await
    }
    /// One-click new identity (xiaowei 一键新机): overwrite the app-visible device fingerprint.
    /// `android_id` applies without root (adb WRITE_SECURE_SETTINGS); `serialno` and `mac`
    /// need root (`resetprop`, `ip link`). Each field is best-effort and reported; a field the
    /// device or its root state rejects does not fail the others. Note this changes what apps
    /// *read* (Build/Settings/MAC), not the baseband IMEI.
    pub async fn set_device_identity(
        &self,
        serial: &str,
        android_id: Option<&str>,
        serialno: Option<&str>,
        mac: Option<&str>,
    ) -> anyhow::Result<String> {
        // Validated **before** anything touches the phone, and validated together: all three
        // are pasted into `su -c "…"` below, where `$(…)` and a backtick still substitute
        // inside the double quotes, so an unchecked value here is root code execution on the
        // device. Checking up front rather than per-field is deliberate — a partially applied
        // identity is worse than a refused one, and the doc comment two functions up
        // ("callers here pass fixed commands, not operator free-text") was true of
        // `factory_reset` and never true of this function.
        let android_id = android_id
            .map(adb::validate_android_id)
            .transpose()
            .map_err(|error| anyhow::anyhow!("android_id không hợp lệ: {error}"))?;
        let serialno = serialno
            .map(adb::validate_serial_no)
            .transpose()
            .map_err(|error| anyhow::anyhow!("serialno không hợp lệ: {error}"))?;
        let mac = mac
            .map(adb::validate_mac)
            .transpose()
            .map_err(|error| anyhow::anyhow!("địa chỉ MAC không hợp lệ: {error}"))?;

        let rooted = self.is_rooted(serial).await;
        let mut done: Vec<String> = Vec::new();
        let mut failed: Vec<String> = Vec::new();

        if let Some(id) = android_id {
            // Try plain first (adb has WRITE_SECURE_SETTINGS); fall back to root.
            let plain = self
                .adb
                .shell(serial, &format!("settings put secure android_id {id}"))
                .await;
            let ok = plain.is_ok()
                || (rooted
                    && self
                        .adb
                        .shell(
                            serial,
                            &format!("su -c \"settings put secure android_id {id}\""),
                        )
                        .await
                        .is_ok());
            if ok {
                done.push("android_id".into());
            } else {
                failed.push("android_id".into());
            }
        }

        if let Some(sn) = serialno {
            if rooted {
                let a = self
                    .adb
                    .shell(serial, &format!("su -c \"resetprop ro.serialno {sn}\""))
                    .await;
                let b = self
                    .adb
                    .shell(
                        serial,
                        &format!("su -c \"resetprop ro.boot.serialno {sn}\""),
                    )
                    .await;
                if a.is_ok() || b.is_ok() {
                    done.push("serialno".into());
                } else {
                    failed.push("serialno".into());
                }
            } else {
                failed.push("serialno(cần root)".into());
            }
        }

        if let Some(m) = mac {
            if rooted {
                let cmd = format!(
                    "su -c \"ip link set wlan0 down; ip link set wlan0 address {m}; ip link set wlan0 up\""
                );
                if self.adb.shell(serial, &cmd).await.is_ok() {
                    done.push("wifi_mac".into());
                } else {
                    failed.push("wifi_mac".into());
                }
            } else {
                failed.push("wifi_mac(cần root)".into());
            }
        }

        let mut summary = format!(
            "Đã đổi: {}",
            if done.is_empty() {
                "—".into()
            } else {
                done.join(", ")
            }
        );
        if !failed.is_empty() {
            summary.push_str(&format!(" · Không đổi được: {}", failed.join(", ")));
        }
        Ok(summary)
    }

    // --- The per-phone function menu (xiaowei 功能, one row each). Everything here is one
    // or two adb calls and none of it needs the helper APK or root, which is the reason this
    // block exists as its own tier: the operator gets the whole menu on a stock phone. ---
    /// List one directory on the phone (xiaowei "Preview Mobile Files").
    ///
    /// The trailing slash is not cosmetic. Measured on 23021RAAEG: `ls -la /sdcard` prints
    /// *the symlink* — one line, `/sdcard -> /storage/self/primary` — while `ls -la /sdcard/`
    /// prints the contents. Browsing without it shows the phone's main storage as a single
    /// mysterious file.
    ///
    /// A non-zero exit is surfaced as an error with the phone's own sentence, because for a
    /// browser that is the honest outcome: `ls: /sdcard/nope: No such file or directory` on
    /// stderr with an empty stdout is a refusal, and rendering it as an empty folder would
    /// claim the directory exists and is empty.
    pub async fn list_device_dir(
        &self,
        serial: &str,
        path: &str,
    ) -> anyhow::Result<riviu_core::DeviceDirListing> {
        let path = adb::validate_device_path(path)?;
        let listed = if path.ends_with('/') {
            path.to_string()
        } else {
            format!("{path}/")
        };
        let out = self
            .adb
            .shell_output(
                serial,
                &format!("ls -la {}", adb::quote_device_path(&listed)),
                Duration::from_secs(30),
            )
            .await?;
        // One condition used to decide this -- `entries.is_empty() && exit_code != 0` -- and it
        // left three holes, each of which made the browser state something false instead of
        // failing. `classify_ls_output` is where those three are reasoned about and tested.
        match adb::classify_ls_output(&out.stdout, &out.stderr, out.exit_code) {
            adb::LsOutcome::Complete(entries) => Ok(riviu_core::DeviceDirListing {
                entries,
                incomplete: None,
            }),
            adb::LsOutcome::Partial { entries, reason } => Ok(riviu_core::DeviceDirListing {
                entries,
                incomplete: Some(reason),
            }),
            adb::LsOutcome::Refused(reason) => {
                anyhow::bail!("không đọc được {path}: {reason}")
            }
        }
    }
    /// Copy one file or folder off the phone onto this machine (xiaowei "Export File").
    ///
    /// Returns where it landed. `adb pull` is given the destination *directory*, so the
    /// phone's own name for the file is kept — renaming it here would make an export of
    /// twenty phones into twenty files whose origin is only in the log.
    pub async fn pull_device_path(
        &self,
        serial: &str,
        remote: &str,
        dest_dir: &Path,
    ) -> anyhow::Result<PathBuf> {
        let remote = adb::validate_device_path(remote)?;
        std::fs::create_dir_all(dest_dir)
            .with_context(|| format!("tạo thư mục {}", dest_dir.display()))?;
        let name = pulled_name(remote);
        // Not `shell`: `adb pull` is a client subcommand, so the path never reaches a device
        // shell and needs no quoting — it is one argv element.
        self.adb
            .device(
                serial,
                &["pull", remote, &dest_dir.to_string_lossy()],
                Duration::from_secs(300),
            )
            .await
            .map_err(|e| anyhow!("adb pull thất bại: {e}"))?;
        let landed = dest_dir.join(name);
        if !landed.exists() {
            anyhow::bail!(
                "adb pull báo xong nhưng không thấy {} — đường dẫn trên máy có thể là thư mục rỗng",
                landed.display()
            );
        }
        Ok(landed)
    }
    /// Put a local file onto the phone (xiaowei "Import File").
    ///
    /// Deliberately *not* the media-import path: that one stages a campaign and tells
    /// MediaStore about the result so a picture shows up in the gallery. This is the file
    /// manager's own push — whatever the file is, wherever the operator pointed — and it
    /// promises nothing more than "the bytes are at this path now", which it verifies.
    pub async fn push_device_file(
        &self,
        serial: &str,
        local: &Path,
        remote_dir: &str,
    ) -> anyhow::Result<String> {
        let remote_dir = adb::validate_device_path(remote_dir)?;
        if !local.is_file() {
            anyhow::bail!("không thấy file {}", local.display());
        }
        let name = local
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .ok_or_else(|| anyhow!("đường dẫn nguồn không có tên file"))?;
        let target = pushed_target(remote_dir, &name);
        adb::validate_device_path(&target)?;
        self.adb
            .device(
                serial,
                &["push", &local.to_string_lossy(), &target],
                Duration::from_secs(300),
            )
            .await
            .map_err(|e| anyhow!("adb push thất bại: {e}"))?;
        // Proof, not optimism: `adb push` has been seen to report success while the target
        // directory was read-only, and a file manager that then lists nothing new looks
        // broken rather than refused.
        let check = self
            .adb
            .shell_output(
                serial,
                &format!("ls -la {}", adb::quote_device_path(&target)),
                Duration::from_secs(20),
            )
            .await?;
        if adb::parse_ls_listing(&check.stdout).is_empty() {
            anyhow::bail!("đẩy xong nhưng máy không thấy {target}");
        }
        Ok(target)
    }
    /// Delete a file or folder on the phone.
    ///
    /// Two guards, and they answer different fears. [`adb::is_undeletable_root`] refuses a
    /// delete aimed at a storage root — the one gesture no confirm dialog can undo — and the
    /// path validator refuses anything a single quote could break out of. Everything else is
    /// the operator's call, and the UI asks first.
    pub async fn delete_device_path(&self, serial: &str, path: &str) -> anyhow::Result<()> {
        let path = adb::validate_device_path(path)?;
        if adb::is_undeletable_root(path) {
            anyhow::bail!("{path} là gốc lưu trữ — không xoá cả gốc, chỉ xoá thứ bên trong");
        }
        let out = self
            .adb
            .shell_output(
                serial,
                &format!("rm -rf {}", adb::quote_device_path(path)),
                Duration::from_secs(60),
            )
            .await?;
        if out.exit_code != 0 {
            let reason = if out.stderr.trim().is_empty() {
                out.stdout.trim().to_string()
            } else {
                out.stderr.trim().to_string()
            };
            anyhow::bail!("xoá {path} thất bại: {reason}");
        }
        // `rm -rf` is silent about a path it could not remove for a reason it swallows, so
        // absence is read back rather than assumed.
        let check = self
            .adb
            .shell_output(
                serial,
                &format!("ls -la {}", adb::quote_device_path(path)),
                Duration::from_secs(20),
            )
            .await?;
        // **The read-back has to be a reading, not a guess.** This used to be
        // `parse_ls_listing(&check.stdout)`, which sees stdout only. A ROM that answers
        // `ls: <path>: Permission denied` on stderr with exit 0 and nothing on stdout then
        // produces an empty listing, the condition is false, and the delete is reported as
        // confirmed -- when what actually happened is that the *verification* was refused and
        // the file may still be there.
        //
        // `classify_ls_output` exists precisely to tell those three cases apart and was
        // already used by `list_device_dir`; this site was missed when it landed. Found by an
        // independent review on 27/08/2026.
        match adb::classify_ls_output(&check.stdout, &check.stderr, check.exit_code) {
            // Nothing there, and the phone said so cleanly. This is the success case.
            adb::LsOutcome::Complete(entries) if entries.is_empty() => Ok(()),
            adb::LsOutcome::Complete(_) => {
                anyhow::bail!("{path} vẫn còn trên máy sau khi xoá")
            }
            // A row we could read means it survived, whatever else was unreadable.
            adb::LsOutcome::Partial { entries, reason } if !entries.is_empty() => {
                anyhow::bail!("{path} vẫn còn trên máy sau khi xoá ({reason})")
            }
            adb::LsOutcome::Partial { reason, .. } => {
                anyhow::bail!("xoá {path} rồi nhưng không kiểm lại được: {reason}")
            }
            adb::LsOutcome::Refused(reason) => {
                anyhow::bail!("xoá {path} rồi nhưng không kiểm lại được: {reason}")
            }
        }
    }
    /// Turn the phone's own Wi-Fi radio on or off (xiaowei ADB submenu "Turn on/off WIFI"),
    /// and report the state it actually settled at.
    ///
    /// `svc wifi` prints nothing at all — success and refusal look identical — so the answer
    /// comes from reading `settings get global wifi_on` back. Measured end to end on
    /// 23021RAAEG (Android 15) 21/08/2026: `disable` then a 1 s wait reads `0`, `enable` then
    /// a 2 s wait reads `1`, and both `svc` calls exit 0 with empty output either way.
    ///
    /// **A phone reached over wireless adb disconnects itself by obeying this.** The serial
    /// says which — `10969614` is USB, `192.168.1.42:5555` is not — and the UI warns before
    /// asking; the driver still obeys, because an operator switching a phone to cable next is
    /// a legitimate thing to want.
    pub async fn set_wifi_radio(&self, serial: &str, on: bool) -> anyhow::Result<bool> {
        let verb = if on { "enable" } else { "disable" };
        self.adb
            .shell(serial, &format!("svc wifi {verb}"))
            .await
            .map_err(|e| anyhow!("svc wifi {verb} thất bại: {e}"))?;
        // The radio takes a moment to report its new state; asking immediately reads the old
        // one and makes a working toggle look like it did nothing. Two seconds because that
        // is what the slower direction (on) needed when measured, not because it looks safe.
        tokio::time::sleep(Duration::from_millis(2000)).await;
        // **An unread state is not a state.** This was `.unwrap_or_default()`, so a phone
        // that dropped its transport during the two-second settle -- which is exactly what a
        // wireless-adb phone does when told to turn its own Wi-Fi off -- returned `Ok(false)`.
        // For `disable` that is indistinguishable from a confirmed success, and for `enable`
        // it reports the toggle as having done nothing. The function's own contract is "report
        // the state it actually settled at", and a failed read cannot do that.
        let read = self
            .adb
            .shell(serial, "settings get global wifi_on")
            .await
            .with_context(|| {
                format!("đã gửi `svc wifi {verb}` cho {serial} nhưng không đọc lại được trạng thái")
            })?;
        Ok(read.trim() == "1")
    }
    /// Put the display back to the resolution and density the phone shipped with (xiaowei
    /// ADB submenu "Reset DPI" / "Reset resolution"), and say what it reads as afterwards.
    ///
    /// Returns the phone's own two lines so the operator sees the result rather than a
    /// claim. Measured on 23021RAAEG: `Physical size: 1080x2400` and `Physical density: 440`.
    /// `wm` prints only the *override* when one is set, so a reset that worked is a listing
    /// with the physical values and no override line.
    pub async fn reset_display_metrics(
        &self,
        serial: &str,
        density: bool,
        size: bool,
    ) -> anyhow::Result<String> {
        if !density && !size {
            anyhow::bail!("không có gì để đặt lại");
        }
        if size {
            self.adb
                .shell(serial, "wm size reset")
                .await
                .map_err(|e| anyhow!("wm size reset thất bại: {e}"))?;
        }
        if density {
            self.adb
                .shell(serial, "wm density reset")
                .await
                .map_err(|e| anyhow!("wm density reset thất bại: {e}"))?;
        }
        let read = self
            .adb
            .shell(serial, "wm size; wm density")
            .await
            .unwrap_or_default();
        Ok(read.trim().to_string())
    }
    /// Power the phone off (xiaowei "Shutdown").
    ///
    /// `adb reboot -p` and not `adb shell reboot -p`: the client subcommand is the one that
    /// survives the connection dying underneath it, which is guaranteed here — the device
    /// this is aimed at stops answering as a direct result. A transport error *after* the
    /// request went out is therefore not treated as a failure; the fleet list noticing the
    /// phone is gone is the real confirmation, and it arrives on the next refresh.
    pub async fn power_off(&self, serial: &str) -> anyhow::Result<()> {
        match self
            .adb
            .device(serial, &["reboot", "-p"], Duration::from_secs(20))
            .await
        {
            Ok(_) => Ok(()),
            Err(error) => {
                let text = error.to_string().to_lowercase();
                if text.contains("closed")
                    || text.contains("device offline")
                    || text.contains("device not found")
                    || text.contains("no devices")
                {
                    tracing::info!(
                        serial,
                        "tắt máy: adb mất kết nối ngay sau lệnh, coi là đã tắt"
                    );
                    Ok(())
                } else {
                    Err(anyhow!("tắt máy thất bại: {error}"))
                }
            }
        }
    }
    /// Open the phone's own Settings app (xiaowei hidden-function "Phone Settings").
    ///
    /// An action intent rather than a package launch: the Settings *package* differs by ROM
    /// (`com.android.settings` on AOSP, MIUI ships its own on this fleet's Redmi), while
    /// `android.settings.SETTINGS` is resolved by the phone itself and cannot be wrong.
    pub async fn open_system_settings(&self, serial: &str) -> anyhow::Result<()> {
        let out = self
            .adb
            .shell_output(
                serial,
                "am start -a android.settings.SETTINGS",
                Duration::from_secs(20),
            )
            .await?;
        // `am start` exits 0 while printing `Error: Activity not started`, so the exit code
        // alone would let a refusal pass as a success.
        let text = format!("{}{}", out.stdout, out.stderr);
        if text.contains("Error:") {
            anyhow::bail!("mở Cài đặt thất bại: {}", text.trim());
        }
        Ok(())
    }
    /// Wake the screen (xiaowei hidden-function "Turn On Screen").
    ///
    /// KEYCODE_WAKEUP, which is idempotent — unlike KEYCODE_POWER, which *toggles* and so
    /// puts an already-awake phone to sleep. The same constant the capture path uses.
    pub async fn wake_screen(&self, serial: &str) -> anyhow::Result<()> {
        self.adb
            .shell(serial, adb::WAKE_KEYEVENT)
            .await
            .map(|_| ())
            .map_err(|e| anyhow!("bật màn hình thất bại: {e}"))
    }
    /// Whether the lock screen is up **and** what owns the screen, from one `dumpsys window`.
    ///
    /// Two facts, one round trip, deliberately. Both callers need both: a phone whose
    /// foreground reads as a system window is almost always a locked phone, and a phone that
    /// reports locked needs its foreground read again after the unlock to prove anything.
    /// Asking twice would double the cost of the check on a fleet where `dumpsys window` is
    /// the thing being done fourteen times.
    ///
    /// `locked: None` means the dump carried none of the three keys — unknown, which
    /// [`adb::parse_keyguard_locked`] documents callers must not read as "unlocked".
    /// Give a splash screen a bounded moment to get out of the way. Never fails.
    ///
    /// **Bounded and non-fatal, and the first version of this was neither.** TikTok's splash
    /// carries the app's own package, so the readiness proof — which reads the package — is
    /// satisfied by a phone that has drawn nothing yet, and the interaction then reads an
    /// empty screen and refuses with `no_baseline`. Measured 25/08/2026 on a twenty-phone run:
    /// eight phones sat on `…aweme.splash.SplashActivity` and five failed that way.
    ///
    /// The obvious fix — make the proof wait for a non-splash activity — was tried and made it
    /// much worse: **7/20 with 13 failures**, every one of them `did not reach the foreground
    /// within 40s`. On this fleet, twenty simultaneous cold starts hold the splash past the
    /// whole forty-second budget, and phones that used to leave it a moment later and work
    /// were failed outright. So the splash is worth *waiting on* and never worth *failing on*:
    /// a phone that leaves it inside this window arrives with a rendered feed and a readable
    /// baseline, and a phone that does not is exactly as well off as before this existed.
    ///
    /// Eight seconds: long enough to cover the gap between the process drawing and the feed
    /// appearing on a phone that is loading normally, short enough that twenty of them are a
    /// tail on a three-minute run rather than a second budget beside it.
    pub async fn wait_out_splash(&self, serial: &str) {
        const SPLASH_GRACE: std::time::Duration = std::time::Duration::from_secs(8);
        const SPLASH_POLL: std::time::Duration = std::time::Duration::from_millis(500);
        let deadline = std::time::Instant::now() + SPLASH_GRACE;
        loop {
            match self.foreground_activity(serial).await {
                Some(activity) if adb::is_splash_activity(&activity) => {}
                // Off the splash, or the dump named no activity — either way there is nothing
                // left to wait for.
                _ => return,
            }
            if std::time::Instant::now() >= deadline {
                return;
            }
            tokio::time::sleep(SPLASH_POLL).await;
        }
    }

    /// The activity of the focused window, or `None` when the dump does not name one.
    ///
    /// Only used to tell "the app is up" from "the app is *ready*": the package alone cannot,
    /// because a splash screen carries the package too. `None` is not a failure — the caller
    /// falls back to the behaviour it had before this existed.
    pub async fn foreground_activity(&self, serial: &str) -> Option<String> {
        let dump = self
            .adb
            .shell(serial, "dumpsys window | grep mCurrentFocus")
            .await
            .ok()?;
        adb::parse_foreground_activity(&dump)
    }

    pub async fn screen_guard_state(&self, serial: &str) -> anyhow::Result<ScreenGuardState> {
        let dump = self
            .adb
            .shell(serial, "dumpsys window")
            .await
            .map_err(|e| anyhow!("đọc trạng thái màn hình thất bại: {e}"))?;
        Ok(ScreenGuardState {
            locked: adb::parse_keyguard_locked(&dump),
            foreground: adb::parse_foreground_window(&dump),
        })
    }

    /// Try to get past a swipe-only lock screen, and say honestly whether it worked.
    ///
    /// Sends [`adb::KEYGUARD_DISMISS_KEYEVENTS`] and then **re-reads** the keyguard rather
    /// than trusting the keys. That re-read is the whole contract: a phone with a PIN,
    /// pattern or fingerprint stays locked and this returns `false`, so a caller reports
    /// "cần mở khoá bằng tay" instead of going on to tap a lock screen. Measured on both
    /// locked phones on 23/08/2026 — `mDreamingLockscreen` true → false, TikTok focused
    /// immediately after — so on this fleet it returns `true`.
    ///
    /// Idempotent and safe on an already-unlocked phone: `KEYCODE_WAKEUP` only wakes, and
    /// `KEYCODE_MENU` goes to a focused window that ignores it.
    pub async fn dismiss_keyguard(&self, serial: &str) -> anyhow::Result<bool> {
        for keyevent in adb::KEYGUARD_DISMISS_KEYEVENTS {
            // A refused keyevent is not fatal on its own — the re-read below is the only
            // thing that decides — so this does not abandon the sequence half-sent.
            if let Err(error) = self.adb.shell(serial, keyevent).await {
                tracing::warn!(serial, %error, keyevent, "keyevent mở khoá bị từ chối");
            }
        }
        // The keyguard animates out. Without this the re-read races it and reports a phone
        // still locked that is already on its way open.
        tokio::time::sleep(std::time::Duration::from_millis(1_200)).await;
        let state = self.screen_guard_state(serial).await?;
        // `None` — the build printed none of the three keys — is not proof of success.
        // Fall back to the foreground: an app in front is evidence the keyguard is gone.
        Ok(match state.locked {
            Some(locked) => !locked,
            None => matches!(state.foreground, adb::ForegroundWindow::App(_)),
        })
    }

    /// Take a screenshot and leave it *on the phone* (xiaowei "Screenshot to phone").
    ///
    /// The other screenshot command copies the picture to this machine; this one is the row
    /// beside it in xiaowei's menu, and the difference is the point — a phone whose gallery
    /// has pictures in it looks like a phone somebody uses.
    ///
    /// Verified by reading the size back: `screencap` on a phone that refuses to capture
    /// (secure window in front, measured on TikTok's own screens) exits 0 and leaves a
    /// zero-byte file, which is exactly the silent failure this project's rules forbid.
    /// The name is stamped **by the phone**, in one shell call that also lists the result.
    /// Two reasons, and the second is the load-bearing one: the phone's clock is the one an
    /// operator scrolling its gallery will compare against, and a single call cannot end up
    /// listing a *different* file than the one it captured — which a host-side name plus a
    /// separate `ls` can, on a phone whose second ticked over between them.
    pub async fn screenshot_to_device(&self, serial: &str) -> anyhow::Result<String> {
        // Fixed script, no operator input in it, so `$(…)` here is ours and not an injection
        // surface. `Pictures/` exists on every phone here; a fresh flash may not have it.
        const SCRIPT: &str = "p=/sdcard/Pictures/riviu-$(date +%Y%m%d-%H%M%S).png; \
             mkdir -p /sdcard/Pictures && screencap -p \"$p\" && ls -la \"$p\"";
        let out = self
            .adb
            .shell_output(serial, SCRIPT, Duration::from_secs(60))
            .await?;
        if out.exit_code != 0 {
            let reason = if out.stderr.trim().is_empty() {
                out.stdout.trim().to_string()
            } else {
                out.stderr.trim().to_string()
            };
            anyhow::bail!("chụp vào máy thất bại: {reason}");
        }
        // Listing a file by path prints that whole path as the name — measured on
        // 23021RAAEG — so this row carries both the proof and the answer.
        let captured = adb::parse_ls_listing(&out.stdout)
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("chụp xong nhưng máy không liệt kê được ảnh vừa tạo"))?;
        if captured.size == 0 {
            anyhow::bail!(
                "chụp xong nhưng ảnh rỗng — màn hình đang mở nội dung không cho chụp (secure window)"
            );
        }
        Ok(captured.name)
    }
    /// Put the phone's own names and icons onto a listing adb produced.
    ///
    /// adb stays the source of truth for *which* apps exist — it reads both partitions and
    /// includes apps with no launcher activity, which a `queryIntentActivities` sweep would
    /// miss. The helper answers the one question adb cannot: what an app is called, and what
    /// it looks like (AGENTS.md §9.55).
    ///
    /// **The user partition only, and that is a measurement rather than a preference.** On
    /// 23021RAAEG (21/08/2026) describing all 539 packages took 4 559 ms; the 162
    /// user-partition packages with icons took 3 599 ms. The system partition is 377 rows the
    /// UI keeps behind a toggle and a farm operator does not launch, so paying four seconds
    /// to name them on every listing buys nothing. They keep their package names, which is
    /// what every row showed before this existed.
    ///
    /// Best effort throughout: a phone with no helper keeps its package names and says so
    /// once in the log rather than failing the listing.
    pub(super) async fn name_apps_with_helper(
        &self,
        serial: &str,
        apps: &mut [riviu_core::InstalledApp],
    ) {
        let wanted: Vec<String> = apps
            .iter()
            .filter(|app| app.kind == riviu_core::InstalledAppKind::User)
            .map(|app| app.bundle_id.clone())
            .collect();
        if wanted.is_empty() {
            return;
        }
        let fingerprint = package_set_fingerprint(&wanted);
        if let Some(cached) = self.app_descriptions.lock().get(serial) {
            if cached.fingerprint == fingerprint {
                apply_app_descriptions(apps, &cached.rows);
                return;
            }
        }

        let helper = match self.try_attach_helper(serial).await {
            Ok(Some(helper)) => helper,
            Ok(None) => {
                tracing::info!(
                    serial,
                    "không có Riviu helper — danh sách app chỉ có tên gói, không có nhãn/icon"
                );
                return;
            }
            Err(error) => {
                tracing::warn!(serial, %error, "không gắn được helper để đọc nhãn app");
                return;
            }
        };
        let described = match helper.describe_apps(&wanted, true).await {
            Ok(rows) => rows,
            Err(error) => {
                tracing::warn!(serial, %error, "helper không trả nhãn app");
                return;
            }
        };
        apply_app_descriptions(apps, &described);
        self.app_descriptions.lock().insert(
            serial.to_string(),
            AppDescriptionCache {
                fingerprint,
                rows: described,
            },
        );
    }
    /// Switch the phone's keyboard (xiaowei "Switch Input Method").
    ///
    /// The id is validated the same way the picker in the UI validates it, and for the same
    /// reason twice over: it is interpolated into a device shell, and the helper's own IME
    /// must never be left installed as the phone's keyboard (AGENTS.md §9.5x).
    pub async fn set_input_method(&self, serial: &str, ime_id: &str) -> anyhow::Result<()> {
        let ime_id = crate::riviu_agent::validate_ime_id(ime_id)?;
        if ime_id == crate::riviu_agent::IME_ID {
            anyhow::bail!("không đặt bàn phím của Riviu làm bàn phím chính của máy");
        }
        self.adb
            .shell(serial, &format!("ime set {ime_id}"))
            .await
            .map(|_| ())
            .map_err(|e| anyhow!("đổi bàn phím thất bại: {e}"))
    }
    /// Factory-reset the phone (xiaowei 恢复出厂). Needs root; sends the system MASTER_CLEAR
    /// broadcast. Irreversible — the UI gates this behind an explicit confirm.
    pub async fn factory_reset(&self, serial: &str) -> anyhow::Result<()> {
        if !self.is_rooted(serial).await {
            anyhow::bail!("máy chưa root (không có su) — không thể khôi phục gốc từ xa");
        }
        self.adb
            .shell(
                serial,
                "su -c \"am broadcast -a android.intent.action.MASTER_CLEAR\"",
            )
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod path_tests {
    use super::{pulled_name, pushed_target};

    /// The ordinary case, and the two shapes of trailing slash the UI produces.
    #[test]
    fn a_pulled_file_keeps_the_name_the_phone_gave_it() {
        assert_eq!(
            pulled_name("/sdcard/Download/CV prototype.pdf"),
            "CV prototype.pdf"
        );
        assert_eq!(pulled_name("/sdcard/Download"), "Download");
        assert_eq!(pulled_name("/sdcard/Download/"), "Download");
        assert_eq!(
            pulled_name("/sdcard/Download/Giao Trinh - Bai Giang - HDH"),
            "Giao Trinh - Bai Giang - HDH"
        );
        assert_eq!(
            pulled_name("/sdcard/Download/John's photo.jpg"),
            "John's photo.jpg"
        );
    }

    /// **A path that trims to nothing must not name the destination directory itself.**
    ///
    /// `"/"` trims to `""`, and `rsplit(`/`)` on an empty string yields `Some("")` rather than
    /// `None` -- so without the filter the landed path would be the destination *directory*, and
    /// the caller's existence check would pass because a directory it just created always
    /// exists. A pull that produced nothing would report success.
    #[test]
    fn a_root_path_falls_back_to_a_name_rather_than_to_nothing() {
        assert_eq!(pulled_name("/"), "pulled");
        assert_eq!(pulled_name("///"), "pulled");
        assert_eq!(pulled_name(""), "pulled");
    }

    /// One slash between the directory and the name, whichever form the directory arrived in.
    ///
    /// Both reach here: the file manager's breadcrumbs produce `/sdcard/Download` and its typed
    /// path box produces `/sdcard/Download/`. `//` inside a device path is not something to hand
    /// to a shell, and it also makes the read-back look for a path that is not the one written.
    #[test]
    fn a_pushed_file_gets_exactly_one_slash_before_its_name() {
        assert_eq!(
            pushed_target("/sdcard/Download", "photo.jpg"),
            "/sdcard/Download/photo.jpg"
        );
        assert_eq!(
            pushed_target("/sdcard/Download/", "photo.jpg"),
            "/sdcard/Download/photo.jpg"
        );
        assert_eq!(pushed_target("/sdcard", "a b.txt"), "/sdcard/a b.txt");
    }

    /// The phone's storage root, written either way, still yields an absolute path.
    #[test]
    fn pushing_into_the_root_still_produces_an_absolute_path() {
        assert_eq!(pushed_target("/", "photo.jpg"), "/photo.jpg");
        assert!(pushed_target("/", "photo.jpg").starts_with('/'));
    }
}
