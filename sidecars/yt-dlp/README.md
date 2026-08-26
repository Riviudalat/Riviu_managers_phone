# yt-dlp

One binary, used for one thing: asking TikTok what a post says and what pictures
it has, from the operator's own machine, before any phone is touched.
`crates/core/src/tiktok_web.rs` is the only caller.

Nothing here is written by this project. yt-dlp is Unlicense; see
[`../../NOTICE`](../../NOTICE).

## Why a whole extractor for one HTTP request

Because it is not one HTTP request. Measured 26/08/2026 on this machine: a plain
GET of a post URL carrying a browser user-agent returns **HTTP 200 and 1462
bytes with no post data in it**. TikTok answers a bare request with a shell.

yt-dlp gets through by solving a JS challenge and retrying with the cookie it
yields, which its log narrates:

```
[TikTok] Downloading webpage
[TikTok] Solving JS challenge using native Python implementation
[TikTok] Downloading webpage with challenge cookie
```

Reimplementing that here would mean re-solving a problem a maintained project
already solves, against a target that changes it on its own schedule.

## Not committed, and how to get one

The `.exe` is **not** in git — 17 MB of third-party binary that goes stale is not
something to carry in history. `.gitignore` excludes it. Fetch one:

```bash
curl -sL -o sidecars/yt-dlp/yt-dlp.exe \
  https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe
```

`resolve_ytdlp` searches, in order: `RIVIU_YTDLP_PATH`, next to the running
executable, `binaries/` beside it, `Resources/` above it, this directory, then
`PATH`. So a `cargo run` picks this copy up and an installed build picks up the
one the installer placed beside the app.

**It is optional.** With no yt-dlp anywhere, every lookup fails as
`WebLookupError::NoBinary`, the campaign writes its comments from what the phones
can see, and nothing else changes.

## It will rot, and that is expected

TikTok breaks extractors periodically and the fix is always a newer yt-dlp — so
`RIVIU_YTDLP_PATH` exists to point a run at one without a rebuild, and whatever
ships in an installer should be downloaded at build time rather than pinned by
hash. The sibling project `Riviudalat/RiviudownloadTik` already does exactly that
in its release workflow, and its `src-tauri/binaries/README.md` is the reference.

## What one lookup returns, measured

Run against the seven real targets in this box's `riviu.db` on 26/08/2026:

| | |
|---|---|
| caption (`description`) | 157, 171, 184, 216, 399 characters — the tree truncates at ~116 |
| carousel slides (`imagePost.images`) | 2, 5, 7, 8 pictures at 1416x2008 |
| ASR transcript (`subtitleInfos`) | **0 of 7** — six are photo posts, and the one video reported `hasOriginalAudio: false` |
| refused outright | 2 of 7, `Your IP address is blocked from accessing this post`, identical on three attempts |
| transient failure | ~1 run in 5, `Unable to extract universal data for rehydration`, cleared on retry |

Two consequences are written into the code and should stay there: an IP block is
**not** retried, and a `/photo/` URL is rewritten to `/video/` before it is
passed in — the `/photo/` form is rejected as `Unsupported URL` while the exact
same post resolves under `/video/`.
