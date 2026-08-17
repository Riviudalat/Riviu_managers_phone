//! Getting a publish campaign's images onto an Android device and into its media
//! library.
//!
//! The iOS contract this implements has four steps with distinct meanings — stage the
//! files somewhere the photo app cannot see, validate them, *import* them so the
//! composer can pick them, and clean up only what was imported. Android keeps all
//! four, and the split falls out of one measured fact:
//!
//! | path | file on disk | MediaStore |
//! |---|---|---|
//! | `…/Pictures/.riviu-publish/<c>/x.png` | yes | **not visible** |
//! | `…/Pictures/riviu-<c>-<sha>/x.png` | yes | visible |
//! | after `mv` from the first to the second | yes | **visible** |
//!
//! (Redmi Note 12, Android 15, 11/08/2026.) A dot-prefixed directory is excluded from
//! media scanning, so staging into one is genuinely invisible; the import is then a
//! `mv`, which within one volume is a rename — cheap and atomic.
//!
//! **Two fleet members behave differently, and the import asks rather than assumes.**
//! Measured the same day on an SM-N950F (Android 8.0, API 26):
//!
//! | | Redmi Note 12 / API 35 | SM-N950F / API 26 |
//! |---|---|---|
//! | `mv` alone reaches MediaStore | yes, ~1.5 s | **never** |
//! | `MEDIA_SCANNER_SCAN_FILE` needed | no | **yes**, and it works |
//! | `is_pending` column exists | yes, and it starts at 1 | **no such column** |
//!
//! So the import polls, broadcasts a scan only if polling came up empty, and clears
//! `is_pending` only on a device that reports the column. Both branches verify with
//! `content query`, never with an exit code: `am broadcast` on this project has a
//! measured case of returning `result=0` while doing nothing, and `content query` on
//! API 26 prints an `SQLiteException` while exiting 0.
//!
//! One consequence for [`prepare`]: it asserts the staged files are *not* visible, and
//! that assertion holds on both phones — but for different reasons. On API 35 the dot
//! prefix excludes them from scanning; on API 26 nothing is scanned until something
//! broadcasts. The assertion is still the right one; the explanation is not portable.
//!
//! Two rules that are not style:
//!
//! * **Every identifier that reaches `adb shell` is code, not data** — the same reason
//!   [`crate::adb::validate_package_name`] exists, whose negative test is literally
//!   `"com.x; rm -rf /sdcard/DCIM"`.
//! * **Cleanup deletes MediaStore rows by `_id`, never by a `_data LIKE '%riviu%'`
//!   pattern.** The measured device already had an unrelated
//!   `/storage/emulated/0/riviufarm-shot.png` from a co-resident farm tool; a loose
//!   pattern would have deleted somebody else's file.

use std::path::Path;
use std::time::Duration;

use anyhow::{anyhow, Context};
use serde_json::{json, Value};

use crate::adb::AdbProgram;

/// Where staged files live: dot-prefixed, so MediaStore does not scan them.
const STAGE_ROOT: &str = "/sdcard/Pictures/.riviu-publish";
/// Where imported files live: a plain directory, so MediaStore does.
const IMPORT_PARENT: &str = "/sdcard/Pictures";
/// MediaStore's images table.
const IMAGES_URI: &str = "content://media/external/images/media";
/// Generous: `pm`/`content` calls on this fleet are 1–2 s, and a push is bounded by
/// file size.
const PUSH_TIMEOUT: Duration = Duration::from_secs(120);

/// Only the characters a filename and a shell argument can both carry safely.
///
/// Campaign ids are namespaced as `<request-id>:<bundle-id>`, and `:` is neither a
/// legal filename character on every filesystem nor something to hand a shell.
pub fn sanitise_id(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-') {
                character
            } else {
                '-'
            }
        })
        .collect();
    // Collapse runs so `a::b` does not become `a--b` in one place and `a-b` in another.
    let mut collapsed = String::with_capacity(cleaned.len());
    let mut previous_dash = false;
    for character in cleaned.chars() {
        if character == '-' {
            if !previous_dash {
                collapsed.push('-');
            }
            previous_dash = true;
        } else {
            collapsed.push(character);
            previous_dash = false;
        }
    }
    collapsed.trim_matches('-').to_string()
}

/// Refuse an identifier the device shell would act on.
///
/// Mirrors [`crate::adb::validate_package_name`]: this value is interpolated into a
/// shell command on the phone, so a `;` or a `..` in it is a command, not a name.
pub fn validate_shell_id(id: &str) -> anyhow::Result<&str> {
    if id.is_empty() || id.len() > 128 {
        anyhow::bail!(
            "publish identifier must be 1..=128 characters, got {}",
            id.len()
        );
    }
    if id.starts_with('-') {
        anyhow::bail!("publish identifier {id:?} would be read as a flag");
    }
    if id.contains("..") {
        anyhow::bail!("publish identifier {id:?} contains a path traversal");
    }
    if !id
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        anyhow::bail!(
            "publish identifier {id:?} has characters the device shell would act on; only \
             alphanumerics, dot, underscore and hyphen are allowed"
        );
    }
    Ok(id)
}

/// The import id, which is also the visible directory's basename.
///
/// Deriving the directory from the id means cleanup needs nothing but the id — no
/// side table, and no chance of deleting a directory that belonged to another
/// campaign. The manifest hash is included so re-staging different content cannot
/// reuse a directory that still holds the old files.
///
/// **It is also the selection key inside TikTok's own picker.** Measured on an SM-N950F,
/// 11/08/2026: after an import, TikTok's album dropdown lists this exact string as an
/// album — `riviu-picker-check-one-8e69493351ef`, with its file count `1` beside it — so
/// the post path can pick the campaign's album by a string **this code wrote itself**
/// instead of trusting that the newest images in `Gần đây` are ours. That removes the
/// only unfounded assumption the Android publish path would otherwise need.
///
/// Unmeasured, and the reason to keep the id short: whether a long id is truncated with
/// an ellipsis in that dropdown. The measured 36-character name rendered in full.
pub fn import_id(campaign_id: &str, manifest_sha256: &str) -> String {
    let campaign = sanitise_id(campaign_id);
    let short: String = manifest_sha256.chars().take(12).collect();
    format!("riviu-{campaign}-{short}")
}

fn stage_dir(campaign: &str) -> String {
    format!("{STAGE_ROOT}/{campaign}")
}

fn import_dir(import_id: &str) -> String {
    format!("{IMPORT_PARENT}/{import_id}")
}

/// One file as it exists on both sides.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagedFile {
    pub name: String,
    pub bytes: u64,
    pub sha256: String,
}

/// A MediaStore row, reduced to what a cleanup needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaRow {
    pub id: String,
    pub data: String,
}

/// Parse `content query` output.
///
/// The observed shape is
/// `Row: 0 _id=1000011139, _data=/storage/emulated/0/DCIM/Camera/x.png, date_added=…`.
/// `No result found.` is an empty list, not an error — an absent row is the expected
/// answer after a successful cleanup.
pub fn parse_media_rows(stdout: &str) -> Vec<MediaRow> {
    stdout
        .lines()
        .filter_map(|line| {
            let field = |key: &str| -> Option<String> {
                let start = line.find(key)? + key.len();
                let rest = &line[start..];
                let end = rest.find(',').unwrap_or(rest.len());
                Some(rest[..end].trim().to_string())
            };
            let id = field("_id=")?;
            let data = field("_data=")?;
            (!id.is_empty() && !data.is_empty()).then_some(MediaRow { id, data })
        })
        .collect()
}

/// The first line in a `content` call's output that says it failed, if any.
///
/// **`content` reports provider failures on stderr and still exits 0.** Measured on an
/// SM-N950F (Android 8.0, 11/08/2026): `content update --bind is_pending:i:0` against a
/// MediaStore with no such column printed a full `SQLiteException` with a stack trace
/// and returned `rc=0`. Because `AdbProgram::shell` hands back stdout and treats a zero
/// exit as success, that failure arrived here as an empty string and an `Ok` — the step
/// reported having cleared a flag it had not touched.
///
/// So every `content` call in this module redirects stderr into stdout **on the device**
/// (`2>&1`, device-side `sh`, portable) and is checked with this. The exit code is not
/// evidence; the text is.
pub fn content_error(output: &str) -> Option<&str> {
    output.lines().map(str::trim).find(|line| {
        line.contains("Error while accessing provider")
            || line.contains("SQLiteException")
            || line.starts_with("Error: ")
            || line.starts_with("Unsupported argument")
    })
}

/// Whether this MediaStore has the scoped-storage `is_pending` column at all.
///
/// A query, not an API-level check, and the reason is measured: on API 26 (SM-N950F,
/// Android 8.0) `content query --projection _id:is_pending` prints
/// `SQLiteException: no such column: is_pending` and **exits 0**. The exit code says
/// nothing; the error text is the whole answer. Pre-scoped-storage devices have no
/// pending concept, so there is nothing there to clear and skipping the step is correct
/// rather than a shortcut.
pub fn reports_pending_column(stdout: &str) -> bool {
    !stdout.contains("no such column: is_pending")
}

/// Keep only the rows MediaStore reports as pending.
///
/// `is_pending` arrives as a plain `is_pending=1` / `is_pending=0` field, so the filter
/// is a line test applied before the shared row parser — the projection has to ask for
/// the column, and asking for it everywhere would widen every other query for nothing.
pub fn parse_pending_rows(stdout: &str) -> Vec<MediaRow> {
    stdout
        .lines()
        .filter(|line| line.contains("is_pending=1"))
        .flat_map(parse_media_rows)
        .collect()
}

/// `<sha256>  <path>` from `sha256sum`.
pub fn parse_sha256sum(stdout: &str) -> Option<String> {
    stdout
        .split_whitespace()
        .next()
        .filter(|hash| hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()))
        .map(|hash| hash.to_ascii_lowercase())
}

/// Push a campaign's files into the hidden staging directory and prove they arrived.
///
/// The readback is **size and sha256**, both from the device. `crate::frames::ensure_apk`
/// deliberately checks only the byte count, because a hash there would cost an extra
/// adb round trip for an artifact this project owns; here the contract asks for the
/// hash, and `sha256sum` on the device measured 83 ms for a 2 MB image, so a carousel
/// of at most eleven files pays under a second.
/// Where a phone keeps the pictures and videos a person would call "mine".
///
/// The camera roll and the general picture directory, in that order. Deliberately not a
/// recursive sweep of `/sdcard`: that is full of app caches, thumbnails and downloads, and an
/// Export that hands the operator ten thousand WhatsApp thumbnails has not exported anything.
///
/// `Movies` is included because that is where a phone puts video that did not come from the
/// camera — the two together are what the gallery shows.
const MEDIA_ROOTS: [&str; 4] = [
    "/sdcard/DCIM/Camera",
    "/sdcard/DCIM",
    "/sdcard/Pictures",
    "/sdcard/Movies",
];

/// Extensions a gallery would show. Anything else on those paths is not media.
const MEDIA_EXTENSIONS: [&str; 8] = ["jpg", "jpeg", "png", "webp", "gif", "mp4", "mov", "3gp"];

/// One media file on the phone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteMedia {
    pub path: String,
    pub name: String,
}

/// Pick the media files out of a `find` listing.
///
/// Pure so the filtering can be tested without a phone — which matters more than usual here,
/// because the failure mode of getting it wrong is copying gigabytes of somebody's cache onto
/// their desktop.
///
/// De-duplicated by path, because `MEDIA_ROOTS` deliberately contains both `/sdcard/DCIM` and
/// `/sdcard/DCIM/Camera`: the second is where the camera writes and the first catches the
/// screenshots and downloads a phone scatters beside it, so a file under Camera is listed
/// twice and must be fetched once.
pub fn parse_media_listing(stdout: &str) -> Vec<RemoteMedia> {
    let mut seen = std::collections::HashSet::new();
    let mut found = Vec::new();
    for line in stdout.lines() {
        let path = line.trim();
        if path.is_empty() || !path.starts_with('/') {
            continue;
        }
        // A hidden component ANYWHERE, not just a hidden file name. `/sdcard/DCIM/.thumbnails`
        // is full of real `.jpg` files that are not the operator's photos, and the whole
        // point of a dot-prefixed directory on Android is that the gallery does not show it
        // — so neither should this. Checking only the file name let every thumbnail through.
        if path.split('/').any(|part| part.starts_with('.')) {
            continue;
        }
        let Some(name) = path.rsplit('/').next() else {
            continue;
        };
        if name.is_empty() {
            continue;
        }
        let Some((_, extension)) = name.rsplit_once('.') else {
            continue;
        };
        let extension = extension.to_ascii_lowercase();
        if !MEDIA_EXTENSIONS.contains(&extension.as_str()) {
            continue;
        }
        if !seen.insert(path.to_string()) {
            continue;
        }
        found.push(RemoteMedia {
            path: path.to_string(),
            name: name.to_string(),
        });
    }
    found
}

/// A local name that cannot collide and cannot escape the destination directory.
///
/// Two phones both have `IMG_0001.jpg`, and so does the same phone in two directories. The
/// index keeps them apart; stripping everything but the file name keeps a crafted remote path
/// from writing outside where the operator pointed.
pub fn local_media_name(index: usize, remote_name: &str) -> String {
    let safe: String = remote_name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let safe = safe.trim_matches('.').to_string();
    if safe.is_empty() {
        format!("{index:04}-media")
    } else {
        format!("{index:04}-{safe}")
    }
}

/// Copy the phone's photos and videos into `dest_dir`.
///
/// Read back rather than trusted: `adb pull` exiting zero is not evidence a file arrived, the
/// same rule the push side applies to its own transfers. A file that did not land is skipped
/// with a warning instead of failing the whole export — one unreadable picture must not cost
/// the operator the other four hundred.
pub async fn pull_media(
    adb: &AdbProgram,
    serial: &str,
    dest_dir: &Path,
) -> anyhow::Result<riviu_core::MediaPullReport> {
    std::fs::create_dir_all(dest_dir)
        .with_context(|| format!("create the export directory {}", dest_dir.display()))?;

    // `-maxdepth 2` keeps this to the gallery's own layout (a root and its dated
    // subdirectories) rather than walking whatever an app has nested underneath. Missing
    // directories are normal on a phone that has never used the camera, so their errors are
    // discarded rather than treated as a failure.
    let script = MEDIA_ROOTS
        .iter()
        .map(|root| format!("find {root} -maxdepth 2 -type f 2>/dev/null"))
        .collect::<Vec<_>>()
        .join("; ");
    let listing = adb
        .shell(serial, &script)
        .await
        .context("list the phone's media directories")?;
    let media = parse_media_listing(&listing);

    if media.is_empty() {
        // A genuinely empty gallery. Not an error, and the caller must be able to say so
        // rather than implying something went wrong.
        return Ok(riviu_core::MediaPullReport::default());
    }

    let mut pulled = Vec::with_capacity(media.len());
    for (index, item) in media.iter().enumerate() {
        let dest = dest_dir.join(local_media_name(index, &item.name));
        let dest_arg = dest.display().to_string();
        if let Err(error) = adb
            .device(
                serial,
                &["pull", item.path.as_str(), dest_arg.as_str()],
                PULL_TIMEOUT,
            )
            .await
        {
            tracing::warn!(serial, path = %item.path, %error, "could not pull one media file");
            continue;
        }
        match std::fs::metadata(&dest) {
            Ok(meta) if meta.len() > 0 => pulled.push(dest),
            // `adb pull` can exit zero having written nothing at all; the file on disk is
            // the only evidence that counts.
            _ => {
                tracing::warn!(serial, path = %item.path, "adb pull reported success but no bytes landed");
                let _ = std::fs::remove_file(&dest);
            }
        }
    }
    // Found media and fetched none of it is a failure, and it must not be reported as an
    // empty gallery — those are the same number and opposite meanings. This is not
    // hypothetical: AGENTS.md 9.12 records `adb` silently writing nothing when the
    // destination path is mangled, and the read-back above turns exactly that into a count
    // of zero.
    anyhow::ensure!(
        !pulled.is_empty(),
        "found {} media files on {serial} and none of them arrived in {}; \
         adb reported success but wrote no bytes",
        media.len(),
        dest_dir.display()
    );
    // Both numbers travel together from here on. Returning only `pulled` made a phone with
    // five hundred photos of which twenty copied indistinguishable from a phone that has
    // twenty photos: the per-file failures above went to the log, and the operator was
    // handed a number that reads as a success. What was missing was any way for the count
    // to admit them.
    if pulled.len() < media.len() {
        tracing::warn!(
            serial,
            found = media.len(),
            fetched = pulled.len(),
            "some media did not arrive"
        );
    }
    Ok(riviu_core::MediaPullReport {
        fetched: pulled,
        found: media.len(),
    })
}

/// A phone's camera roll can be large and USB 2.0 is not fast. Generous, because the failure
/// this guards is a genuinely big video, not a hang.
const PULL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(600);

pub async fn stage(
    adb: &AdbProgram,
    serial: &str,
    campaign_id: &str,
    source_root: &Path,
) -> anyhow::Result<Value> {
    anyhow::ensure!(
        source_root.is_dir(),
        "publish media source root is not a directory: {}",
        source_root.display()
    );
    let campaign = sanitise_id(campaign_id);
    let campaign = validate_shell_id(&campaign)?;
    let remote_root = stage_dir(campaign);

    // Start from empty: a re-stage after a partial failure must not inherit files the
    // new manifest does not list.
    adb.shell(
        serial,
        &format!("rm -rf {remote_root} && mkdir -p {remote_root}"),
    )
    .await
    .context("prepare the staging directory")?;

    let mut entries: Vec<std::path::PathBuf> = Vec::new();
    for entry in std::fs::read_dir(source_root).context("read the publish source root")? {
        let entry = entry?;
        if entry.file_type()?.is_file() {
            entries.push(entry.path());
        }
    }
    // Deterministic order, so the manifest hash does not depend on directory order.
    entries.sort();

    let mut staged: Vec<StagedFile> = Vec::new();
    for path in &entries {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| anyhow!("publish source file has a non-UTF-8 name"))?;
        let safe = sanitise_id(name);
        validate_shell_id(&safe)?;
        // Two source names that sanitise to the same thing would silently overwrite each
        // other on the device, and the carousel would come out a picture short with a
        // manifest that still claims the full count. `a b.png` and `a-b.png` both become
        // `a-b.png`, so this is reachable from an ordinary folder of images.
        if let Some(clash) = staged.iter().find(|file| file.name == safe) {
            anyhow::bail!(
                "two source files map to the same device name {safe:?} (one of them is \
                 {:?}); rename them so the carousel keeps every image",
                clash.name
            );
        }
        let bytes = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
        let local_sha = riviu_core::frame_sha256(&bytes);
        let remote = format!("{remote_root}/{safe}");
        adb.device(
            serial,
            &["push", &path.display().to_string(), &remote],
            PUSH_TIMEOUT,
        )
        .await
        .with_context(|| format!("push {} to {remote}", path.display()))?;

        // The push's own success is not evidence. Read both back.
        let size = adb
            .shell(serial, &format!("wc -c < {remote}"))
            .await
            .context("read back the staged size")?
            .trim()
            .parse::<u64>()
            .unwrap_or_default();
        anyhow::ensure!(
            size == bytes.len() as u64,
            "{remote} is {size} bytes on the device, {} locally",
            bytes.len()
        );
        let device_sha = adb
            .shell(serial, &format!("sha256sum {remote}"))
            .await
            .context("read back the staged sha256")
            .ok()
            .and_then(|stdout| parse_sha256sum(&stdout))
            .ok_or_else(|| anyhow!("could not read a sha256 for {remote}"))?;
        anyhow::ensure!(
            device_sha == local_sha,
            "{remote} hashes {device_sha} on the device, {local_sha} locally"
        );
        staged.push(StagedFile {
            name: safe,
            bytes: size,
            sha256: local_sha,
        });
    }
    anyhow::ensure!(
        !staged.is_empty(),
        "publish source root {} has no files",
        source_root.display()
    );

    let manifest = json!({
        "schemaVersion": 1,
        "campaignId": campaign_id,
        "files": staged
            .iter()
            .map(|file| json!({ "name": file.name, "bytes": file.bytes, "sha256": file.sha256 }))
            .collect::<Vec<_>>(),
    });
    let manifest_bytes = serde_json::to_vec(&manifest).context("serialise the manifest")?;
    let manifest_sha256 = riviu_core::frame_sha256(&manifest_bytes);

    Ok(json!({
        "ok": true,
        "udid": serial,
        "campaignId": campaign_id,
        "remoteRoot": remote_root,
        "fileCount": staged.len(),
        "manifestSha256": manifest_sha256,
        "manifestBytes": manifest_bytes.len(),
        "readback": "size+sha256",
        "hiddenFromMediaStore": true,
    }))
}

/// Validate the staged tree and hand back the id the import will use.
///
/// Deliberately re-reads the device rather than trusting the staging step's return
/// value: `prepare` exists precisely so that a stage which happened minutes ago is
/// still true now.
pub async fn prepare(
    adb: &AdbProgram,
    serial: &str,
    campaign_id: &str,
    manifest_sha256: &str,
) -> anyhow::Result<Value> {
    let campaign = sanitise_id(campaign_id);
    let campaign = validate_shell_id(&campaign)?;
    let remote_root = stage_dir(campaign);
    let listing = adb
        .shell(serial, &format!("ls -1 {remote_root} 2>/dev/null"))
        .await
        .context("list the staged campaign")?;
    let files: Vec<&str> = listing
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    anyhow::ensure!(
        !files.is_empty(),
        "nothing is staged at {remote_root}; stage_publish_media must run first"
    );
    // Still hidden: if these were already visible, the import step would be a no-op and
    // the operator would have no way to tell an import happened.
    let rows = media_rows_under(adb, serial, &remote_root).await?;
    anyhow::ensure!(
        rows.is_empty(),
        "{} staged file(s) are already visible to MediaStore; the staging directory is \
         not hidden and the two-step contract is broken",
        rows.len()
    );
    Ok(json!({
        "campaignId": campaign_id,
        "importId": import_id(campaign_id, manifest_sha256),
        "state": "ready",
        "files": files.len(),
    }))
}

/// Move the staged files into a visible directory and prove MediaStore saw them.
///
/// The proof is a `content query`, not `mv`'s exit code — "the file moved" and "another
/// app can see it" are different claims, and only the second one lets TikTok's picker
/// find the images.
pub async fn import(
    adb: &AdbProgram,
    serial: &str,
    campaign_id: &str,
    manifest_sha256: &str,
) -> anyhow::Result<Value> {
    let campaign = sanitise_id(campaign_id);
    let campaign = validate_shell_id(&campaign)?;
    let remote_root = stage_dir(campaign);
    let id = import_id(campaign_id, manifest_sha256);
    validate_shell_id(&id)?;
    let visible = import_dir(&id);

    adb.shell(serial, &format!("mkdir -p {visible}"))
        .await
        .context("create the visible import directory")?;
    // `mv` within one volume is a rename, so this is cheap and leaves no half-copied
    // file for MediaStore to index.
    adb.shell(
        serial,
        &format!("mv {remote_root}/* {visible}/ && rmdir {remote_root} 2>/dev/null; true"),
    )
    .await
    .context("move the staged files into the visible directory")?;

    // What actually landed. Read from the device rather than carried from `stage`,
    // because the scan broadcast below needs a path per file and a name that has been
    // through a `mv` is the only one that is certainly there.
    let listing = adb
        .shell(serial, &format!("ls -1 {visible} 2>/dev/null"))
        .await
        .context("list the imported directory")?;
    let mut names: Vec<&str> = listing
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    names.sort_unstable();
    anyhow::ensure!(
        !names.is_empty(),
        "moved the staged files into {visible} but the directory is empty"
    );
    // These came back off the device and are about to be interpolated into a shell
    // command, so they are re-validated rather than trusted for having been safe once.
    for name in &names {
        validate_shell_id(name)?;
    }

    // Give the scanner a moment, then ask it — `mv`'s exit code says nothing about
    // whether another app can see the result.
    let mut rows = poll_media_rows(adb, serial, &visible).await?;
    let mut scanned = false;
    if rows.is_empty() {
        // Measured divergence, and the reason this is a fallback rather than a version
        // check: on Android 15 (Redmi Note 12) the rename alone reaches MediaStore in
        // ~1.5 s, on Android 8.0 (SM-N950F) it never does. The broadcast fixes the
        // second case — verified by `content query`, **not** by `result=0`, which this
        // repo has a measured case of lying.
        for name in &names {
            let _ = adb
                .shell(
                    serial,
                    &format!(
                        "am broadcast -a android.intent.action.MEDIA_SCANNER_SCAN_FILE \
                         -d file://{visible}/{name}"
                    ),
                )
                .await;
        }
        scanned = true;
        rows = poll_media_rows(adb, serial, &visible).await?;
    }
    anyhow::ensure!(
        !rows.is_empty(),
        "moved the files into {visible} and asked the media scanner for {} of them, but \
         MediaStore lists none, so the composer cannot pick them",
        names.len()
    );
    // By path, which is `01.png`, `02.png`, … so that the `is_pending` updates below and
    // the `mediaIds` in the evidence are in a stable, meaningful order rather than
    // whatever `content query` happened to return.
    //
    // **This does not decide the carousel's order.** The picker was measured to list an
    // album newest-first, and what a carousel actually uses is the order the cells are
    // *tapped*. Establishing that is the post path's job, and the post path does not
    // exist yet — so no claim is made here beyond determinism.
    rows.sort_by(|left, right| left.data.cmp(&right.data));

    // Clear `is_pending`, which is the whole difference between an import the composer
    // can use and one it cannot.
    //
    // **Being in MediaStore is necessary and not sufficient.** A row created by
    // `adb push` lands with `is_pending=1` — scoped storage's "an app is still writing
    // this" flag — and a pending row is invisible to every *other* app. Measured against
    // TikTok's own picker on a Redmi Note 12 (11/08/2026): the imported image was absent
    // from the grid with `is_pending=1`, and after nothing but this one update it
    // appeared as the **first** cell. It was still absent when it lived in
    // `DCIM/Camera`, after a cold start of TikTok, and after hand-setting `datetaken` —
    // so directory, cache and timestamps were all the wrong suspects.
    //
    // Clearing the flag also makes MediaProvider scan the file, which fills `_size`,
    // `width`, `height` and `date_modified` on its own. Writing those by hand is
    // therefore cargo-cult; the row that the picker accepted still had
    // `datetaken=NULL`.
    // Asked, not assumed: pre-scoped-storage MediaStore has no such column, and a
    // device that has no pending concept has nothing to clear.
    let pending_model = if device_reports_pending_column(adb, serial).await {
        let mut cleared = 0usize;
        for row in &rows {
            let command = format!(
                "content update --uri {IMAGES_URI}/{id} --bind is_pending:i:0 2>&1",
                id = row.id
            );
            // Worth reporting loudly in either failure branch, because the file is on
            // the device and invisible: the operator would otherwise see a successful
            // import and an empty picker.
            match adb.shell(serial, &command).await {
                Ok(output) => match content_error(&output) {
                    None => cleared += 1,
                    Some(detail) => {
                        tracing::warn!(id = %row.id, detail, "content update reported a failure while exiting 0")
                    }
                },
                Err(error) => {
                    tracing::warn!(id = %row.id, %error, "could not clear is_pending on an imported row")
                }
            }
        }
        anyhow::ensure!(
            cleared == rows.len(),
            "cleared is_pending on {cleared} of {} imported row(s); the rest stay \
             invisible to the composer",
            rows.len()
        );
        // `content update` exiting 0 is not evidence, for the same reason `am broadcast`
        // returning `result=0` was not: read the flag back from MediaStore.
        tokio::time::sleep(Duration::from_millis(400)).await;
        let still_pending = pending_rows_under(adb, serial, &visible).await?;
        anyhow::ensure!(
            still_pending.is_empty(),
            "{} row(s) under {visible} are still is_pending=1 after the update, so they \
             stay invisible to the composer",
            still_pending.len()
        );
        "cleared"
    } else {
        "absent"
    };

    Ok(json!({
        "campaignId": campaign_id,
        "importId": id,
        "state": "imported",
        "files": rows.len(),
        // Which of the two measured device behaviours this import went through. Both are
        // legitimate; a surprise here is the first sign a fleet member behaves unlike
        // either phone this was measured on.
        "scanBroadcast": scanned,
        "pendingModel": pending_model,
        // The visible directory is what a gallery shows as an album, so it is the
        // closest Android has to the iOS `Riviu-<importId>` album.
        "albumId": visible,
        "mediaIds": rows.iter().map(|row| row.id.clone()).collect::<Vec<_>>(),
    }))
}

/// Remove exactly what one import created.
///
/// Idempotent by construction: a second call finds no rows and no directory and still
/// reports `cleaned`, which is what the publish contract asserts on both the first
/// attempt and the retry.
pub async fn cleanup(adb: &AdbProgram, serial: &str, import_id: &str) -> anyhow::Result<Value> {
    let id = validate_shell_id(import_id)?;
    // Shell-safe is not the same as ours. Without this, any accepted string names a
    // directory under `Pictures/` that this function will `rm -rf` and whose MediaStore
    // rows it will delete — `Camera`, `Screenshots`, `WhatsApp Images`. Every id this
    // module hands out comes from `import_id`, which always carries this prefix, so
    // requiring it costs nothing and closes the whole class.
    anyhow::ensure!(
        id.starts_with("riviu-"),
        "refusing to clean up {id:?}: not an id this project created (every import id is \
         `riviu-<campaign>-<sha>`), and cleanup deletes the directory it names"
    );
    let visible = import_dir(id);
    let rows = media_rows_under(adb, serial, &visible).await?;
    // By `_id`, never by a `LIKE '%riviu%'` pattern: the measured device carried an
    // unrelated `riviufarm-shot.png` from a co-resident tool, and a loose pattern would
    // have deleted it.
    //
    // Checked with `content_error` like every other `content` call, even though the
    // read-back below is the real gate: without it a provider failure is invisible, and
    // the operator sees only "N row(s) still point into …" with no idea why.
    for row in &rows {
        match adb
            .shell(
                serial,
                &format!("content delete --uri {IMAGES_URI}/{} 2>&1", row.id),
            )
            .await
        {
            Ok(output) => {
                if let Some(detail) = content_error(&output) {
                    tracing::warn!(id = %row.id, detail, "content delete reported a failure while exiting 0");
                }
            }
            Err(error) => tracing::warn!(id = %row.id, %error, "could not delete a MediaStore row"),
        }
    }
    adb.shell(serial, &format!("rm -rf {visible}"))
        .await
        .context("remove the imported directory")?;
    // And the hidden staging root, but only when nothing else is staged in it:
    // `rmdir` refuses a non-empty directory, which is exactly the check wanted here —
    // another campaign may be mid-stage, and `rm -rf` would take it with us.
    let _ = adb
        .shell(serial, &format!("rmdir {STAGE_ROOT} 2>/dev/null; true"))
        .await;
    tokio::time::sleep(Duration::from_millis(600)).await;
    let left = media_rows_under(adb, serial, &visible).await?;
    anyhow::ensure!(
        left.is_empty(),
        "{} MediaStore row(s) still point into {visible} after cleanup",
        left.len()
    );
    Ok(json!({
        "importId": id,
        "state": "cleaned",
        "files": rows.len(),
    }))
}

/// MediaStore rows whose `_data` sits under `directory`.
///
/// Queried without a `--where` clause and filtered here, because the quoting a
/// `_data LIKE` needs does not survive being handed through `adb shell` from every
/// host shell — and a mis-quoted `--where` fails as `Unsupported argument: LIKE`,
/// which is easy to mistake for "no rows".
async fn media_rows_under(
    adb: &AdbProgram,
    serial: &str,
    directory: &str,
) -> anyhow::Result<Vec<MediaRow>> {
    rows_under(adb, serial, directory, "_id:_data", parse_media_rows).await
}

/// Wait for MediaStore to catch up, then answer with what it holds.
///
/// Measured budget: ~1.5 s on Android 15 when a rename is enough, and the same order of
/// magnitude after an explicit scan broadcast, so 8 × 600 ms is generous either way.
///
/// A query that *fails* is not "no rows" — it is retried, and if every attempt fails the
/// error is returned rather than flattened into an empty list, because an empty list is
/// what the caller reads as "nothing was imported".
async fn poll_media_rows(
    adb: &AdbProgram,
    serial: &str,
    directory: &str,
) -> anyhow::Result<Vec<MediaRow>> {
    let mut last: Option<anyhow::Error> = None;
    for _ in 0..8 {
        tokio::time::sleep(Duration::from_millis(600)).await;
        match media_rows_under(adb, serial, directory).await {
            Ok(rows) if !rows.is_empty() => return Ok(rows),
            Ok(_) => {}
            Err(error) => last = Some(error),
        }
    }
    match last {
        Some(error) => Err(error),
        None => Ok(Vec::new()),
    }
}

/// Ask the device whether its MediaStore has the scoped-storage `is_pending` column.
async fn device_reports_pending_column(adb: &AdbProgram, serial: &str) -> bool {
    let query = format!("content query --uri {IMAGES_URI} --projection _id:is_pending 2>&1");
    match adb.shell(serial, &query).await {
        Ok(stdout) => reports_pending_column(&stdout),
        // A query that could not run at all is not evidence either way; assume the
        // column is there so the import fails loudly rather than skipping a step that
        // decides whether the picker can see anything.
        Err(_) => true,
    }
}

/// Rows under `directory` that MediaStore still marks `is_pending=1`, i.e. still
/// invisible to every app but the one that wrote them.
async fn pending_rows_under(
    adb: &AdbProgram,
    serial: &str,
    directory: &str,
) -> anyhow::Result<Vec<MediaRow>> {
    rows_under(
        adb,
        serial,
        directory,
        "_id:_data:is_pending",
        parse_pending_rows,
    )
    .await
}

async fn rows_under(
    adb: &AdbProgram,
    serial: &str,
    directory: &str,
    projection: &str,
    parse: fn(&str) -> Vec<MediaRow>,
) -> anyhow::Result<Vec<MediaRow>> {
    let query = format!("content query --uri {IMAGES_URI} --projection {projection} 2>&1");
    let stdout = adb
        .shell(serial, &query)
        .await
        .context("query MediaStore for the campaign's rows")?;
    // A provider failure that exits 0 must not read as "no rows": every caller here
    // treats an empty list as a fact about the library.
    if let Some(detail) = content_error(&stdout) {
        anyhow::bail!("content query failed while exiting 0: {detail}");
    }
    // `/sdcard` is a symlink; MediaStore reports the resolved path.
    //
    // The trailing slash is load-bearing: a bare prefix test also matches a *sibling*
    // whose name merely begins with this one. `riviu-req1-<sha>` and
    // `riviu-req1x-<sha>` are both legal ids, and without the separator a cleanup of the
    // first would find — and delete — the second campaign's rows.
    let resolved = format!(
        "{}/",
        directory.replacen("/sdcard", "/storage/emulated/0", 1)
    );
    let literal = format!("{directory}/");
    Ok(parse(&stdout)
        .into_iter()
        .filter(|row| row.data.starts_with(&resolved) || row.data.starts_with(&literal))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_namespaced_campaign_id_becomes_a_usable_directory_name() {
        // Campaign ids are `<request-id>:<bundle-id>`; `:` is not a filename character
        // and not something to hand a shell.
        assert_eq!(sanitise_id("req-7:bundle-3"), "req-7-bundle-3");
        assert_eq!(sanitise_id("a::b"), "a-b", "runs collapse to one hyphen");
        assert_eq!(sanitise_id("--lead--"), "lead");
        assert_eq!(sanitise_id("keep.dots_and-dashes"), "keep.dots_and-dashes");
    }

    #[test]
    fn anything_the_device_shell_would_act_on_is_refused() {
        // Same rule as `validate_package_name`, whose own negative test is a `rm -rf`.
        for hostile in [
            "ok; rm -rf /sdcard/DCIM",
            "a/../../etc",
            "a b",
            "$(id)",
            "`id`",
            "a|b",
            "-rf",
            "",
        ] {
            assert!(validate_shell_id(hostile).is_err(), "accepted {hostile:?}");
        }
        assert!(validate_shell_id("riviu-req7-abc123def456").is_ok());
    }

    #[test]
    fn a_traversal_hidden_inside_a_legal_looking_id_is_refused() {
        // Every character here is in the allowed set, so only the explicit `..` check
        // catches it — and without that check the id escapes `Pictures/`.
        assert!(validate_shell_id("..").is_err());
        assert!(validate_shell_id("a..b").is_err());
        assert!(validate_shell_id("riviu-..-x").is_err());
    }

    #[test]
    fn the_import_id_is_the_directory_and_changes_with_the_content() {
        let first = import_id("req-7:bundle-3", &"a".repeat(64));
        let second = import_id("req-7:bundle-3", &"b".repeat(64));
        assert_eq!(first, "riviu-req-7-bundle-3-aaaaaaaaaaaa");
        assert_ne!(
            first, second,
            "different content must not reuse a directory that still holds the old files"
        );
        validate_shell_id(&first).expect("the derived id is shell-safe");
    }

    #[test]
    fn media_rows_parse_from_the_shape_the_device_prints() {
        // Copied from a real `content query` on a Redmi Note 12, 11/08/2026.
        let stdout = "Row: 0 _id=1000011139, _data=/storage/emulated/0/DCIM/Camera/a.png, \
                      date_added=1786433417\nRow: 1 _id=1000011140, \
                      _data=/storage/emulated/0/Pictures/b.png, date_added=1786433420\n";
        let rows = parse_media_rows(stdout);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].id, "1000011139");
        assert_eq!(rows[0].data, "/storage/emulated/0/DCIM/Camera/a.png");
        assert_eq!(rows[1].id, "1000011140");
    }

    #[test]
    fn no_result_found_is_an_empty_list_not_a_parse_failure() {
        // This is the expected answer after a cleanup, so it must not look like an error.
        assert!(parse_media_rows("No result found.\n").is_empty());
        assert!(parse_media_rows("").is_empty());
    }

    #[test]
    fn sha256sum_output_is_read_and_validated() {
        let good =
            "9b7afdc109915fb3aa11223344556677889900aabbccddeeff00112233445566  /sdcard/x.png";
        assert_eq!(
            parse_sha256sum(good).as_deref(),
            Some("9b7afdc109915fb3aa11223344556677889900aabbccddeeff00112233445566")
        );
        // A truncated or non-hex first word is not a hash; accepting it would make the
        // readback compare two different things and pass.
        assert_eq!(parse_sha256sum("9b7afdc1  /sdcard/x.png"), None);
        assert_eq!(
            parse_sha256sum("sha256sum: /sdcard/x.png: No such file"),
            None
        );
        assert_eq!(parse_sha256sum(""), None);
    }

    #[test]
    fn a_sibling_directory_with_a_shared_prefix_is_not_treated_as_inside_this_one() {
        // Both are legal ids, and `riviu-req1-` is a prefix of `riviu-req1x-`. Without a
        // separator in the comparison, cleaning up the first campaign would find — and
        // delete — the second one's MediaStore rows.
        let stdout = "Row: 0 _id=1, _data=/storage/emulated/0/Pictures/riviu-req1-aaaaaaaaaaaa/\
                      01.png\nRow: 1 _id=2, \
                      _data=/storage/emulated/0/Pictures/riviu-req1x-bbbbbbbbbbbb/01.png\n";
        let rows = parse_media_rows(stdout);
        assert_eq!(rows.len(), 2, "both rows parse");
        // The filter `rows_under` applies, spelled out here because that function needs a
        // device to run.
        let ours = format!(
            "{}/",
            import_dir("riviu-req1-aaaaaaaaaaaa").replacen("/sdcard", "/storage/emulated/0", 1)
        );
        let mine: Vec<&MediaRow> = rows
            .iter()
            .filter(|row| row.data.starts_with(&ours))
            .collect();
        assert_eq!(mine.len(), 1, "only this campaign's row");
        assert_eq!(mine[0].id, "1");
    }

    #[test]
    fn cleanup_refuses_an_id_this_project_did_not_create() {
        // `cleanup` derives a directory under `Pictures/` from the id and `rm -rf`s it, so
        // "shell-safe" is not a sufficient gate: `Camera`, `Screenshots` and
        // `WhatsApp.Images` are all shell-safe. Only ids `import_id` could have produced
        // may get through, and the prefix is what identifies those.
        for hostile in [
            "Camera",
            "Screenshots",
            "DCIM",
            "WhatsApp.Images",
            "riviufarm",
        ] {
            assert!(
                validate_shell_id(hostile).is_ok(),
                "{hostile} is shell-safe, which is the point"
            );
            assert!(
                !hostile.starts_with("riviu-"),
                "{hostile} must not pass the ownership check"
            );
        }
        // And every id this module hands out does pass it.
        assert!(import_id("req-7:bundle-3", &"a".repeat(64)).starts_with("riviu-"));
    }

    #[test]
    fn two_source_names_that_sanitise_alike_are_refused_rather_than_overwriting() {
        // `a b.png` and `a-b.png` both become `a-b.png`, so the second push would land on
        // the first and the carousel would come out an image short — with a manifest that
        // still claims the full count.
        assert_eq!(sanitise_id("a b.png"), sanitise_id("a-b.png"));
        assert_eq!(sanitise_id("a b.png"), "a-b.png");
    }

    #[test]
    fn a_pending_row_is_told_apart_from_a_published_one() {
        // Both lines copied from a real `content query --projection _id:_data:is_pending`
        // on a Redmi Note 12, 11/08/2026. `is_pending=1` is exactly the state in which
        // TikTok's picker refuses to list a file that is provably on disk and provably
        // in MediaStore, so this distinction is the whole import step.
        let stdout = "Row: 0 _id=1000011193, \
                      _data=/storage/emulated/0/Pictures/riviu-x/01.png, is_pending=1\n\
                      Row: 1 _id=1000011194, \
                      _data=/storage/emulated/0/Pictures/riviu-x/02.png, is_pending=0\n";
        let pending = parse_pending_rows(stdout);
        assert_eq!(pending.len(), 1, "only the pending row may come back");
        assert_eq!(pending[0].id, "1000011193");
        // And the wider parser still sees both, so the count the import compares against
        // is the full set.
        assert_eq!(parse_media_rows(stdout).len(), 2);
        assert!(parse_pending_rows("No result found.\n").is_empty());
    }

    #[test]
    fn a_content_call_that_failed_while_exiting_zero_is_not_mistaken_for_success() {
        // Verbatim from an SM-N950F on Android 8.0, 11/08/2026. This exact output came
        // back with `rc=0` **and on stderr**, so the import counted it as a cleared flag
        // and reported `pendingModel: "cleared"` for a column the device does not have.
        // That is the bug this function and the device-side `2>&1` exist to prevent.
        let update_failure = "Error while accessing provider:media\nandroid.database.sqlite.\
                              SQLiteException: no such column: is_pending (code 1): , while \
                              compiling: UPDATE files SET is_pending=? WHERE _id = 7729\n\
                              \tat android.database.DatabaseUtils.readExceptionFromParcel\n";
        assert!(content_error(update_failure).is_some());
        assert!(content_error(update_failure)
            .unwrap()
            .contains("Error while accessing provider"));

        // A mis-quoted `--where` fails this way, and it is easy to mistake for "no rows".
        assert!(content_error("Unsupported argument: LIKE").is_some());

        // And the two answers that mean nothing went wrong.
        assert_eq!(content_error("No result found."), None);
        assert_eq!(
            content_error("Row: 0 _id=1000011193, _data=/storage/emulated/0/Pictures/a.png"),
            None
        );
    }

    #[test]
    fn a_device_without_the_pending_column_is_told_apart_from_one_with_it() {
        // Verbatim from an SM-N950F on Android 8.0, 11/08/2026 — and note that `content
        // query` printed this while **exiting 0**, which is why the text is the test
        // subject and the exit code is not.
        let api26 = "Error while accessing provider:media\nandroid.database.sqlite.\
                     SQLiteException: no such column: is_pending (code 1): , while \
                     compiling: SELECT _id, is_pending FROM images\n";
        assert!(
            !reports_pending_column(api26),
            "a pre-scoped-storage device has no pending flag to clear"
        );
        // Redmi Note 12, Android 15.
        let api35 = "Row: 0 _id=1000011193, is_pending=1\nRow: 1 _id=1000011194, \
                     is_pending=0\n";
        assert!(reports_pending_column(api35));
        // An empty library still has the column; "no rows" must not read as "no column",
        // because that would silently skip the step that decides picker visibility.
        assert!(reports_pending_column("No result found.\n"));
    }

    #[test]
    fn the_staging_directory_is_dot_prefixed_and_the_import_one_is_not() {
        // The whole two-step contract rests on this, and it is measured: a dot-prefixed
        // directory is not scanned into MediaStore, a plain one is.
        let stage = stage_dir("campaign");
        let visible = import_dir("riviu-campaign-abc");
        assert!(
            stage.contains("/.riviu-publish/"),
            "staging must be hidden: {stage}"
        );
        assert!(
            !visible.split('/').any(|part| part.starts_with('.')),
            "the import directory must be visible: {visible}"
        );
    }

    #[test]
    fn the_export_listing_keeps_gallery_media_and_nothing_else() {
        // What Export copies is decided entirely here, and the cost of getting it wrong is
        // pouring somebody's app caches and thumbnails onto their desktop. Every line below
        // is something a real `find` over these roots produces.
        let listing = "/sdcard/DCIM/Camera/IMG_0001.jpg
/sdcard/DCIM/Camera/VID_0002.mp4
/sdcard/Pictures/Screenshots/Screenshot_1.png
/sdcard/Movies/clip.MOV
/sdcard/DCIM/.thumbnails/1234.jpg
/sdcard/Pictures/.nomedia
/sdcard/DCIM/Camera/notes.txt
/sdcard/DCIM/Camera/archive.zip
find: /sdcard/Movies: No such file or directory
";
        let media = parse_media_listing(listing);
        let names: Vec<&str> = media.iter().map(|item| item.name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "IMG_0001.jpg",
                "VID_0002.mp4",
                "Screenshot_1.png",
                "clip.MOV"
            ],
            "only gallery media, and the extension match is case-insensitive"
        );
        // A `find` error line is not a path and must never become one.
        assert!(media.iter().all(|item| item.path.starts_with('/')));
    }

    #[test]
    fn a_file_under_two_roots_is_fetched_once() {
        // MEDIA_ROOTS deliberately contains both /sdcard/DCIM and /sdcard/DCIM/Camera -- the
        // second is where the camera writes, the first catches the screenshots and downloads
        // a phone scatters beside it -- so everything under Camera is listed twice.
        let listing = "/sdcard/DCIM/Camera/IMG_1.jpg
/sdcard/DCIM/Camera/IMG_1.jpg
";
        assert_eq!(parse_media_listing(listing).len(), 1);
    }

    #[test]
    fn a_local_name_cannot_collide_or_escape_the_export_directory() {
        // Two phones both have IMG_0001.jpg, and so does one phone in two directories; the
        // index is what keeps them apart. And the name reaches the host filesystem, so a
        // crafted remote name must not be able to climb out of where the operator pointed.
        assert_eq!(local_media_name(0, "IMG_0001.jpg"), "0000-IMG_0001.jpg");
        assert_eq!(local_media_name(12, "IMG_0001.jpg"), "0012-IMG_0001.jpg");
        for hostile in [
            "../../etc/passwd",
            "..",
            "/absolute",
            r"a\b.jpg",
            "x;rm -rf y",
        ] {
            let name = local_media_name(1, hostile);
            // The property that matters is that the result is one path COMPONENT: no
            // separator of either kind, and never `.` or `..`, so joining it onto the
            // export directory cannot leave it. `..` inside a longer name is an ordinary
            // filename and banning it would be theatre.
            assert!(!name.contains('/'), "{name}");
            assert!(!name.contains('\\'), "{name}");
            assert!(!name.contains(';'), "{name}");
            assert_ne!(name, "..");
            assert_ne!(name, ".");
            let joined = std::path::Path::new("/tmp/export").join(&name);
            assert_eq!(
                joined.parent(),
                Some(std::path::Path::new("/tmp/export")),
                "{name} must stay directly inside the export directory"
            );
        }
        // A name that sanitises to nothing still produces a usable file.
        assert_eq!(local_media_name(3, "..."), "0003-media");
    }
}
