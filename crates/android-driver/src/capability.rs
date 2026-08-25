//! The Android half of the Flow qualification snapshot.
//!
//! Flow's preflight asks one driver-shaped question — *what exactly is this device, and is
//! its control surface live right now?* — and refuses to run until it has a complete
//! answer. Until 17/08/2026 `AndroidDriver` did not implement
//! `DeviceDriver::inspect_device_for_target` at all, so the trait default returned a typed
//! `unsupported` and **every** Flow run on **every** Android phone failed at preflight. The
//! UI listed those phones as valid targets, which is how a 20-device fleet could be told
//! "all devices are eligible" and then fail 20 out of 20.
//!
//! The assembly here is deliberately split into a pure part and an adb part. Everything in
//! this file is pure: give it the facts and it returns the snapshot. The reading of those
//! facts off a phone lives in [`crate::driver`]. That split is what lets the awkward
//! decisions below be tested rather than argued about.

use riviu_core::{
    ActiveTransport, AgentInstallProof, DeviceCapabilitySnapshot, InstalledAgentIdentity,
    InstalledTargetIdentity, QualifiedGeometry, ScreenOrientation,
};

use crate::adb::DisplayGeometry;

/// This adapter's own contract version with the uiautomator2 server.
///
/// Not something the server reports — Appium's server has no protocol handshake to read.
/// It names *our* side of the arrangement, and it is hash material for
/// `qualified_geometry_profile_id`, so it moves only when the way this driver talks to the
/// agent changes in a way that could invalidate a coordinate picked earlier.
pub(crate) const ADAPTER_VERSION: &str = "android-uiautomator2-v1";

/// Companion to [`ADAPTER_VERSION`], in the field Flow reads as `protocol_version`.
pub(crate) const PROTOCOL_VERSION: u32 = 1;

/// Android's density baseline: 160 dpi is scale 1.0, by definition.
const BASELINE_DENSITY: f64 = 160.0;

/// One installed package, as `dumpsys package` describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PackageIdentity {
    pub(crate) package: String,
    /// `versionName`, e.g. `10.6.2`.
    pub(crate) version: String,
    /// `versionCode`, e.g. `274`.
    pub(crate) build: String,
}

/// Everything a phone had to be asked before a snapshot could be built.
#[derive(Debug, Clone)]
pub(crate) struct AndroidCapabilityFacts {
    pub(crate) agent: PackageIdentity,
    pub(crate) target: PackageIdentity,
    /// SHA-256 of the installed agent APK, read on the device.
    pub(crate) agent_apk_sha256: String,
    pub(crate) display: DisplayGeometry,
    /// `ro.product.model`, e.g. `SM-G955F`.
    pub(crate) product_type: String,
    /// `ro.build.version.release`, e.g. `9`.
    pub(crate) os_version: String,
    /// Whether the agent answered *and* could read the accessibility tree, just now.
    pub(crate) control_surface_live: bool,
    /// The instrumentation component this driver starts to bring the agent up.
    pub(crate) runner: String,
}

/// The rendered screen as Flow's coordinate model wants it.
///
/// Logical size is derived, not read: Android reports pixels and a density, and the
/// density-independent size is `pixels / (dpi / 160)`. Deriving it this way keeps
/// `logical * scale == pixel` exact to the bit, which is the coherence rule
/// `QualifiedGeometry::validate` states.
pub(crate) fn qualified_geometry(display: DisplayGeometry) -> QualifiedGeometry {
    let scale = f64::from(display.density) / BASELINE_DENSITY;
    QualifiedGeometry {
        logical_width: f64::from(display.width) / scale,
        logical_height: f64::from(display.height) / scale,
        pixel_width: display.width,
        pixel_height: display.height,
        scale_x: scale,
        scale_y: scale,
        orientation: orientation(display.rotation),
    }
}

/// `Surface.ROTATION_*` as one of the four orientations the snapshot can carry.
///
/// **A stable labelling, not a claim of equivalence.** `ScreenOrientation`'s names come
/// from iOS, where left/right are defined relative to the home button; Android's rotation
/// index counts quarter-turns from the panel's natural orientation, which on a phone is
/// portrait and on a tablet may not be. What this mapping has to be is *injective and
/// stable* — four rotations to four distinct values, the same way every time — because its
/// only consumers are the device profile id (which needs to tell orientations apart) and
/// the pixel dimensions beside it (which already carry the shape of the screen). Nothing
/// reads the name and infers which way up the phone is.
fn orientation(rotation: u8) -> ScreenOrientation {
    match rotation {
        1 => ScreenOrientation::LandscapeLeft,
        2 => ScreenOrientation::PortraitUpsideDown,
        3 => ScreenOrientation::LandscapeRight,
        _ => ScreenOrientation::Portrait,
    }
}

/// Assemble the snapshot Flow's preflight will hash, store and enforce.
///
/// Three fields need their Android meaning stated, because they were named on iOS:
///
/// * `installed_agent.executable_name` — iOS reads the Mach-O executable inside the agent
///   bundle. Android has no such thing; the honest counterpart is the instrumentation
///   component this driver actually starts (`…server.test/…AndroidJUnitRunner`), since that
///   *is* what runs, and naming the server package again would say nothing new.
/// * `installed_agent.signer_identity_sha256` — iOS hashes the signing identity string from
///   the provisioning profile. adb exposes no comparable string: `dumpsys package` prints a
///   32-bit `hashCode` of the signature, not a digest. So this carries the SHA-256 of the
///   installed APK, read on the phone. It answers the same question the iOS value answers —
///   *is this exactly the agent we think it is?* — with the evidence Android has.
/// * `protected_auth_ready` — iOS proves a token-authenticated route answered. The
///   uiautomator2 server has no auth. The equivalent proof, and the one that decides whether
///   Flow can drive this phone at all, is that the agent answered **and could read the
///   accessibility tree**; a server that binds its port but has lost `UiAutomation` fails
///   every query while looking healthy (see `AndroidDriver::ensure_agent`). That live
///   liveness is what this flag carries.
pub(crate) fn build_snapshot(facts: AndroidCapabilityFacts) -> DeviceCapabilitySnapshot {
    DeviceCapabilitySnapshot {
        installed_agent: InstalledAgentIdentity {
            bundle_id: facts.agent.package.clone(),
            version: facts.agent.version.clone(),
            build: facts.agent.build,
            executable_name: facts.runner,
            signer_identity_sha256: facts.agent_apk_sha256.clone(),
        },
        // The artifact this build selected *is* the one on the phone: unlike iOS, where a
        // signed bundle is chosen from a manifest and then installed, the Android agent is
        // whatever `install_agent_apks` last put there. Carrying the same digest twice is
        // not redundancy — one field says which APK is installed, the other says which APK
        // this run qualified against, and on Android they are the same file by construction.
        selected_artifact_sha256: facts.agent_apk_sha256,
        agent_version: facts.agent.version,
        protocol_version: PROTOCOL_VERSION,
        driver_adapter_version: ADAPTER_VERSION.to_string(),
        transport: ActiveTransport::AdbTransport,
        product_type: facts.product_type,
        os_version: facts.os_version,
        target_app: InstalledTargetIdentity {
            bundle_id: facts.target.package,
            version: facts.target.version,
            build: facts.target.build,
        },
        protected_auth_ready: facts.control_surface_live,
        geometry: Some(qualified_geometry(facts.display)),
    }
}

/// The proof that the agent is installed and answering, with nothing else started.
///
/// **Why this exists at all.** `DeviceControlPlane::preflight_agent` and `repair_agent`
/// both go through `DeviceDriver::repair_agent_install_only`, and that is deliberate rather
/// than a slip: an existing test pins it — preflight must not open a session, must not
/// start a stream, and must not call the driver's own `preflight_agent`, because checking
/// on a phone should not disturb the phone. Only the iOS drivers implemented it. On Android
/// every caller therefore reached the trait's `unsupported(...)` default, and each of these
/// failed for that one reason: Settings' Check and Repair on all twenty rows, the toolbar's
/// bulk repair, every legacy script job (`job_queue` calls it as the first line of
/// `run_on_device`), and the nurture comment preflight — whose refusal told the operator to
/// run the Agent Repair that had just failed for the same reason.
///
/// Android satisfies the install-only contract naturally. `ensure_agent` installs the two
/// APK halves and starts the instrumentation; the instrumentation is the agent's own
/// process, not a UI session and not a producer, so nothing here creates either.
///
/// Split out as a function because it is the part that can be tested without a phone: the
/// contract is a shape, and `validate_install_only` refuses a proof that claims a session
/// or a stream, or that carries a digest that is not a digest.
pub(crate) fn install_only_proof(
    agent: PackageIdentity,
    agent_apk_sha256: String,
    runner: String,
) -> AgentInstallProof {
    AgentInstallProof {
        installed: InstalledAgentIdentity {
            bundle_id: agent.package,
            version: agent.version,
            build: agent.build,
            executable_name: runner,
            signer_identity_sha256: agent_apk_sha256.clone(),
        },
        // The same digest twice, for the same reason `build_snapshot` carries it twice: on
        // Android the artifact that was selected *is* the file on the phone, because
        // `install_agent_apks` is what put it there.
        artifact_sha256: agent_apk_sha256,
        // `ensure_agent` returning Ok means the server answered and could read the
        // accessibility tree — the liveness this flag names.
        protected_auth_ready: true,
        session_created: false,
        stream_started: false,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_install_only_proof_satisfies_the_contract_it_names() {
        // The whole reason this shape exists: a caller is promised that checking on a phone
        // installed nothing it did not have to, and started nothing at all. Android could
        // not make that promise before — `repair_agent_install_only` fell to the trait's
        // `unsupported(...)`, so Settings' Check and Repair, the bulk repair, every legacy
        // script job and the nurture comment preflight all failed on every phone in a
        // twenty-device fleet, for one missing method.
        let proof = install_only_proof(
            PackageIdentity {
                package: "io.appium.uiautomator2.server".to_string(),
                version: "7.1.1".to_string(),
                build: "71".to_string(),
            },
            "a".repeat(64),
            "io.appium.uiautomator2.server.test/androidx.test.runner.AndroidJUnitRunner"
                .to_string(),
        );

        proof
            .validate_install_only()
            .expect("the proof this driver hands back must pass the check its callers run");
        assert!(!proof.session_created, "install-only opens no session");
        assert!(!proof.stream_started, "install-only starts no producer");
        assert!(proof.protected_auth_ready);
        // One file, named twice: `install_agent_apks` put the artifact there, so the
        // installed digest and the qualified digest are the same by construction.
        assert_eq!(
            proof.artifact_sha256,
            proof.installed.signer_identity_sha256
        );
        assert_eq!(proof.installed.bundle_id, "io.appium.uiautomator2.server");
    }

    #[test]
    fn a_proof_with_a_digest_that_is_not_one_is_refused() {
        // `validate_install_only` is called on the driver's own result before it is
        // returned, so this is the guard that stops a malformed proof reaching storage and
        // being found out later.
        let proof = install_only_proof(
            PackageIdentity {
                package: "io.appium.uiautomator2.server".to_string(),
                version: "7.1.1".to_string(),
                build: "71".to_string(),
            },
            "not-a-digest".to_string(),
            "runner".to_string(),
        );

        assert!(proof.validate_install_only().is_err());
    }

    use riviu_core::flow::qualified_geometry_profile_id;

    use super::*;

    /// SM-G955F on the plugged-in fleet, read 17/08/2026.
    fn fleet_facts() -> AndroidCapabilityFacts {
        AndroidCapabilityFacts {
            agent: PackageIdentity {
                package: "io.appium.uiautomator2.server".into(),
                version: "10.6.2".into(),
                build: "274".into(),
            },
            target: PackageIdentity {
                package: "com.ss.android.ugc.trill".into(),
                version: "38.3.2".into(),
                build: "380302".into(),
            },
            agent_apk_sha256: "7d74eee1536e949d92b026fcd2c5c885cea2518b55b158bf00a84a972d3f5e22"
                .into(),
            display: DisplayGeometry {
                width: 1080,
                height: 2220,
                density: 420,
                rotation: 0,
            },
            product_type: "SM-G955F".into(),
            os_version: "9".into(),
            control_surface_live: true,
            runner: "io.appium.uiautomator2.server.test/androidx.test.runner.AndroidJUnitRunner"
                .into(),
        }
    }

    #[test]
    fn the_fleets_geometry_survives_the_coherence_rule_it_will_be_hashed_under() {
        let geometry = qualified_geometry(fleet_facts().display);
        assert_eq!((geometry.pixel_width, geometry.pixel_height), (1080, 2220));
        assert_eq!(geometry.scale_x, 2.625);
        assert_eq!(geometry.scale_y, 2.625);
        // Derived rather than read, so this is exact rather than nearly. A snapshot whose
        // logical size times its scale is not its pixel size is a typo, not a device.
        assert_eq!(geometry.logical_width * geometry.scale_x, 1080.0);
        assert_eq!(geometry.logical_height * geometry.scale_y, 2220.0);
        assert_eq!(geometry.orientation, ScreenOrientation::Portrait);
    }

    #[test]
    fn a_flow_snapshot_can_actually_be_built_for_an_android_phone() {
        // The regression this whole file exists for: before it, `inspect_device_for_target`
        // fell through to the trait default and Flow could not start on any Android device.
        // The assertion that matters is that preflight's own gate now passes -- a profile id
        // can be computed, which is what `build_preflight` refuses to run without.
        let snapshot = build_snapshot(fleet_facts());
        assert_eq!(snapshot.target_app.bundle_id, "com.ss.android.ugc.trill");
        assert!(snapshot.protected_auth_ready);
        assert!(!snapshot.installed_agent.bundle_id.trim().is_empty());
        assert!(!snapshot.installed_agent.executable_name.trim().is_empty());
        assert!(!snapshot.agent_version.trim().is_empty());
        assert!(qualified_geometry_profile_id(&snapshot).is_ok());
    }

    #[test]
    fn a_blind_agent_is_reported_as_not_ready_rather_than_as_a_working_phone() {
        // A uiautomator2 server can bind its port and answer /status while having lost
        // `UiAutomation`, at which point every query blocks. Flow reads
        // `protected_auth_ready` to decide whether the control surface is live, so this is
        // the field that has to tell the truth about that phone.
        let mut facts = fleet_facts();
        facts.control_surface_live = false;
        assert!(!build_snapshot(facts).protected_auth_ready);
    }

    #[test]
    fn two_phones_that_differ_only_in_density_get_different_profile_ids() {
        // Measured on the plugged-in fleet: one phone reports density 480 where the rest
        // report 420, at the same 1080x2220. A coordinate picked against one is a different
        // logical point on the other, so the profile id has to separate them -- otherwise
        // Flow would replay a tap at the wrong place and call the plan qualified.
        let mut denser = fleet_facts();
        denser.display.density = 480;
        assert_ne!(
            qualified_geometry_profile_id(&build_snapshot(fleet_facts())).unwrap(),
            qualified_geometry_profile_id(&build_snapshot(denser)).unwrap()
        );
    }

    #[test]
    fn every_rotation_is_a_distinct_orientation() {
        // The mapping does not have to agree with iOS's names, but it does have to be
        // injective: a landscape phone that hashed the same as a portrait one would let a
        // plan compiled upright run sideways.
        let ids = [0u8, 1, 2, 3].map(|rotation| {
            let mut facts = fleet_facts();
            facts.display.rotation = rotation;
            qualified_geometry_profile_id(&build_snapshot(facts)).unwrap()
        });
        for (index, id) in ids.iter().enumerate() {
            assert!(
                !ids[index + 1..].contains(id),
                "rotation {index} collides with a later one"
            );
        }
    }

    #[test]
    fn a_rotated_phone_carries_the_size_it_is_showing_not_its_natural_one() {
        // `dumpsys display` swaps `real` on rotation and `wm size` does not, which is why
        // the driver reads the former. This is the assertion that the swap survives all the
        // way into what Flow persists.
        let mut facts = fleet_facts();
        facts.display = DisplayGeometry {
            width: 2220,
            height: 1080,
            density: 420,
            rotation: 1,
        };
        let geometry = build_snapshot(facts).geometry.expect("geometry");
        assert_eq!((geometry.pixel_width, geometry.pixel_height), (2220, 1080));
        assert!(geometry.logical_width > geometry.logical_height);
    }
}
