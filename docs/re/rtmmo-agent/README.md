# RT-MMO Agent Gate A Evidence

Gate A inventories the bundled production oracle without extracting over it,
launching it, or contacting an iPhone. The resulting evidence is the only input
approved for the source-reconstruction work in Project 2.

## Inputs

| Input | Version / digest |
|---|---|
| `sidecars/wda/RiviuAgent.ipa` | SHA-256 `8a24847099495ff70b998522692c43f00dd16b90f698bda6953a73f5d33002ea` |
| `appium-webdriveragent` npm tarball | `15.1.4`; SHA-256 `0c52fc0dcc6f837287be02a593d96d8ef28563c90b4d41f629830e84878f6bbb` |
| npm integrity lock | `sha512-1tPVzIVPsBKynbTFqJyk3Hrf/FZ6kDmeP81P24hJ6q3gYHd2ljsI6OYEhINSbzxDdDmgTuWyYoUa1YtFvZC8oA==` |
| Existing stock WDA package metadata | `16.0.0`; SHA-256 `f74a79196eb2616ea569646816beac32eacddefbc2068ffac1239212076dc270` |

The immutable npm metadata is checked into
`tools/rtmmo-re/baselines/wda-15.1.4.json`. The verified source is extracted only
under ignored `target/rtmmo-re/baselines/package`; it does not replace the stock
WDA 16.0.0 tree.

## Method

Schema version 1 reads the IPA in place and records archive/plist hashes,
Mach-O load commands, sections and dynamically scoped exported symbols (excluding
private externals), structured signing
entitlements, DWARF compile units/function ranges/line tables, filtered
Objective-C class/method tables, and route-like strings. The upstream source
scanner accepts only `.h`, `.m`, `.mm`, and `.swift` files and records relative
provenance for every observed value.

The baseline command verifies the npm SHA-512 integrity, scans the tarball with a
bounded tar parser, compares every source byte with the ignored extracted tree,
and binds the tarball, source-tree, and inventory SHA-256 digests into
`baseline-diff.json`.
Gate A independently rereads the IPA, requires exact equality with the supplied
inventory report, and recomputes the baseline chain instead of trusting generated
inventory or version/git-head labels in the delta. The route contract records
method, auth, session semantics and
typed required request-body fields; all eight documented paths have static
oracle or baseline evidence. This is path evidence only: the declared method,
auth, session and body fields remain contract assertions until a later contract
test or live probe verifies them. `gate-a.md` publishes the path evidence and
contract-source path separately, then lists every additional oracle-only,
baseline-only and shared undocumented path candidate instead of collapsing them
into one count.

The outer app executable is a one-slice FAT container holding ARM64 Mach-O. The
other runtime images and dSYM are thin ARM64. All three runtime images report
`cryptId=0`; the `MH_DSYM` image has no encryption load command, represented as
`null` rather than a parser failure.

Redaction runs before report serialization and publication verification scans
both raw report bytes and decoded JSON strings while rejecting duplicate keys.
It replaces vendor-token fixtures,
boundary-delimited 40-hex device identifiers, and macOS/Windows user-home
prefixes. Provisioning device lists, certificate bodies, binary plist data, and
password-like keys are omitted. `ArchiveData` uses a custom `Debug` view that
prints counts but never its raw IPA entry buffers. The upstream Git commit field
is treated as typed provenance rather than a device identifier.

## Reproduce

```powershell
npm pack appium-webdriveragent@15.1.4 --pack-destination target\rtmmo-re\baselines
cargo run -q -p rtmmo-re -- baseline-verify --lock tools\rtmmo-re\baselines\wda-15.1.4.json --archive target\rtmmo-re\baselines\appium-webdriveragent-15.1.4.tgz
tar -xf target\rtmmo-re\baselines\appium-webdriveragent-15.1.4.tgz -C target\rtmmo-re\baselines
cargo run -q -p rtmmo-re -- inventory --ipa sidecars\wda\RiviuAgent.ipa --output docs\re\rtmmo-agent\inventory.json
cargo run -q -p rtmmo-re -- baseline-diff --inventory docs\re\rtmmo-agent\inventory.json --source target\rtmmo-re\baselines\package --archive target\rtmmo-re\baselines\appium-webdriveragent-15.1.4.tgz --lock tools\rtmmo-re\baselines\wda-15.1.4.json --output docs\re\rtmmo-agent\baseline-diff.json
cargo run -q -p rtmmo-re -- gate-a --ipa sidecars\wda\RiviuAgent.ipa --inventory docs\re\rtmmo-agent\inventory.json --baseline docs\re\rtmmo-agent\baseline-diff.json --routes tools\rtmmo-re\contracts\oracle-routes.json --baseline-source target\rtmmo-re\baselines\package --baseline-archive target\rtmmo-re\baselines\appium-webdriveragent-15.1.4.tgz --baseline-lock tools\rtmmo-re\baselines\wda-15.1.4.json --manifest sidecars\wda\agent-manifest.json --output docs\re\rtmmo-agent\gate-a.md
cargo run -q -p rtmmo-re -- verify-redaction --input docs\re\rtmmo-agent\inventory.json --input docs\re\rtmmo-agent\baseline-diff.json --input docs\re\rtmmo-agent\gate-a.md --input docs\re\rtmmo-agent\README.md
```

## Decision

`gate-a.md` records `Decision: PASS`. The measured inventory contains four
Mach-O images, 533 exported symbols, and one dSYM with 42 compile units, 42 line
sequences, 83 rows and three ranged runner functions. The WDA framework matches
the byte-verified 15.1.4 source.

The runtime images are stripped, so Gate A does not claim a recovered RT-MMO
feature call graph or statically verified HTTP semantics. Project 2 may use the
versioned symbol/Objective-C/route-path/delta evidence, but must add a contract or
probe before implementing a feature-specific delta. This checkpoint performed no
device probe, relay, stream, desktop launch, harness launch, signing, IPA mutation,
or runtime HTTP request.
