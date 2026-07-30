# RT-MMO Agent Forensic Inventory Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Complete Gate A for the bundled RT-MMO oracle by producing a deterministic, redacted IPA/Mach-O/DWARF inventory, a pinned Appium WebDriverAgent 15.1.4 baseline comparison, and an evidence-backed delta report without modifying or launching the production artifact.

**Architecture:** Add a cross-platform Rust CLI at `tools/rtmmo-re` that reads the IPA in place, parses archive/plist/Mach-O/code-sign/DWARF data, extracts Objective-C and route evidence, and emits stable JSON. A checked-in route contract and immutable npm baseline lock feed a report generator that returns `pass` or `blocked` for Gate A. Device probes, binary patching, signing, and candidate agent source belong to later projects.

**Tech Stack:** Rust 2021, `object` 0.39.1, `gimli` 0.34.0, `zip` 4.2.0, `plist` 1.10.0, `clap` 4.6.4, Serde, SHA-2, regex, walkdir, Cargo tests.

**Implementation status (29/07/2026):** Complete after final-review hardening.
Gate A recomputes the npm integrity plus tarball/source/inventory SHA-256 evidence chain, retains
baseline and oracle provenance, filters ObjC table noise, inventories exported
symbols plus DWARF ranges/line tables, validates typed route assertions while
calling static evidence path-only, and rejects raw/decoded/duplicate-key redaction
leaks. The original checklist below is the
execution record; the generated decision is `docs/re/rtmmo-agent/gate-a.md`.

---

## Repository Rules

- The worktree already contains user changes. Do not stage, commit, reset, format,
  or revert files outside this plan. Each task ends with a diff/test checkpoint.
- Never extract over `sidecars/wda/RiviuAgent.ipa` or edit bytes inside it.
- Do not launch the desktop, harness, RT-MMO, tidevice, or an iPhone in Project 1.
- Generated reports may contain bundle IDs and framework names, but no agent token,
  device UDID, build-machine username/home path, provisioning device list, Apple
  credential, or raw certificate body.
- `docs/claude/**` is user-owned transcript output and remains untouched.

## File Map

- Modify: `Cargo.toml` - add the forensic CLI to the workspace.
- Create: `tools/rtmmo-re/Cargo.toml` - isolated tool dependencies.
- Create: `tools/rtmmo-re/src/cli.rs` - stable CLI contract.
- Create: `tools/rtmmo-re/src/model.rs` - serialized schema shared by all stages.
- Create: `tools/rtmmo-re/src/redact.rs` - report-boundary secret redaction.
- Create: `tools/rtmmo-re/src/archive.rs` - safe IPA/ZIP and plist inventory.
- Create: `tools/rtmmo-re/src/macho.rs` - Mach-O headers/load commands/sections.
- Create: `tools/rtmmo-re/src/codesign.rs` - embedded signature and profile metadata.
- Create: `tools/rtmmo-re/src/dwarf.rs` - DWARF compile-unit/function inventory.
- Create: `tools/rtmmo-re/src/objc.rs` - Objective-C metadata and route candidates.
- Create: `tools/rtmmo-re/src/routes.rs` - checked-in oracle route contract.
- Create: `tools/rtmmo-re/src/baseline.rs` - npm lock verification/source comparison.
- Create: `tools/rtmmo-re/src/report.rs` - deterministic JSON/Markdown and Gate A.
- Create: `tools/rtmmo-re/src/lib.rs` - library entrypoints.
- Create: `tools/rtmmo-re/src/main.rs` - CLI dispatch and exit codes.
- Create: `tools/rtmmo-re/tests/cli.rs` - end-to-end CLI tests.
- Create: `tools/rtmmo-re/baselines/wda-15.1.4.json` - immutable upstream lock.
- Create: `tools/rtmmo-re/contracts/oracle-routes.json` - known route semantics.
- Create: `docs/re/rtmmo-agent/README.md` - method and evidence provenance.
- Generate: `docs/re/rtmmo-agent/inventory.json` - redacted oracle inventory.
- Generate: `docs/re/rtmmo-agent/baseline-diff.json` - WDA delta sets.
- Generate: `docs/re/rtmmo-agent/gate-a.md` - human-readable decision.
- Modify: `AGENTS.md` - record the measured Gate A result.

### Task 1: Scaffold the Cross-Platform CLI

**Files:**
- Modify: `Cargo.toml`
- Create: `tools/rtmmo-re/Cargo.toml`
- Create: `tools/rtmmo-re/src/lib.rs`
- Create: `tools/rtmmo-re/src/main.rs`

- [ ] **Step 1: Add a failing workspace assertion**

Run before editing:

```powershell
cargo metadata --no-deps --format-version 1 | Select-String 'rtmmo-re'
```

Expected: no match, proving the tool is absent.

- [ ] **Step 2: Add the workspace member and crate manifest**

Add `"tools/rtmmo-re"` to the root workspace members and create:

```toml
[package]
name = "rtmmo-re"
version.workspace = true
edition.workspace = true
license.workspace = true
authors.workspace = true

[dependencies]
anyhow.workspace = true
base64.workspace = true
clap = { version = "4.6.4", features = ["derive"] }
gimli = { version = "0.34.0", default-features = false, features = ["read-all", "std"] }
object = "0.39.1"
plist = "1.10.0"
regex = "1"
serde.workspace = true
serde_json.workspace = true
sha2.workspace = true
walkdir = "2"
zip = { version = "4.2.0", default-features = false, features = ["deflate"] }

[dev-dependencies]
tempfile = "3"
```

- [ ] **Step 3: Add the minimal buildable library and version binary**

Create `src/lib.rs`:

```rust
pub const SCHEMA_VERSION: u32 = 1;
```

Create `src/main.rs`:

```rust
fn main() {
    println!("rtmmo-re {}", env!("CARGO_PKG_VERSION"));
}
```

- [ ] **Step 4: Verify the scaffold**

Run:

```powershell
cargo test -p rtmmo-re
cargo run -q -p rtmmo-re
```

Expected: tests pass and stdout is `rtmmo-re 0.1.0`.

- [ ] **Step 5: Record a clean checkpoint**

Run:

```powershell
git diff --check -- Cargo.toml Cargo.lock tools/rtmmo-re
git status --short -- Cargo.toml Cargo.lock tools/rtmmo-re
```

Expected: only planned scaffold files and dependency lock changes are listed.

### Task 2: Define the Report Schema and Redaction Boundary

**Files:**
- Create: `tools/rtmmo-re/src/model.rs`
- Create: `tools/rtmmo-re/src/redact.rs`
- Modify: `tools/rtmmo-re/src/lib.rs`

- [ ] **Step 1: Write failing redaction and deterministic-schema tests**

Add tests covering these exact behaviors:

```rust
#[test]
fn redacts_vendor_tokens_and_udids_before_serialization() {
    let input = "header RTmmo-SAMPLE_TOKEN device 0123456789abcdef0123456789abcdef01234567";
    let (text, count) = redact::text(input);
    assert_eq!(text, "header <redacted-agent-token> device <redacted-device-id>");
    assert_eq!(count, 2);
}

#[test]
fn normalizes_build_machine_home_paths() {
    assert_eq!(redact::path("/Users/builder/project/File.m").0,
               "<home>/project/File.m");
    assert_eq!(redact::path(r"C:\Users\builder\project\File.m").0,
               r"<home>\project\File.m");
}

#[test]
fn file_digest_serializes_with_camel_case_fields() {
    let digest = model::FileDigest {
        path: "Payload/App".into(),
        size: 7,
        sha256: "ab".repeat(32),
    };
    let value = serde_json::to_value(digest).unwrap();
    assert_eq!(value["sha256"], "ab".repeat(32));
    assert_eq!(value["size"], 7);
}
```

Run `cargo test -p rtmmo-re redacts_vendor_tokens_and_udids`; expected failure:
modules `model` and `redact` do not exist.

- [ ] **Step 2: Add concrete serialized models**

Define camelCase `Serialize`/`Deserialize` types for:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FileDigest {
    pub path: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BundleInfo {
    pub plist_path: String,
    pub bundle_id: Option<String>,
    pub executable_path: Option<String>,
    pub short_version: Option<String>,
    pub build_version: Option<String>,
    pub minimum_os_version: Option<String>,
    pub dt_xcode: Option<String>,
    pub dt_sdk_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MachOInfo {
    pub path: String,
    pub sha256: String,
    pub architecture: String,
    pub is_64: bool,
    pub little_endian: bool,
    pub uuid: Option<String>,
    pub crypt_id: Option<u32>,
    pub linked_dylibs: Vec<String>,
    pub sections: Vec<String>,
    pub symbol_count: usize,
    pub objc_classes: Vec<String>,
    pub objc_methods: Vec<String>,
    pub route_candidates: Vec<String>,
    pub entitlements: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DwarfInfo {
    pub path: String,
    pub compile_units: usize,
    pub subprograms: usize,
    pub attribute_errors: usize,
    pub source_paths: Vec<String>,
    pub function_names: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactInventory {
    pub schema_version: u32,
    pub artifact: FileDigest,
    pub entries: Vec<FileDigest>,
    pub bundles: Vec<BundleInfo>,
    pub machos: Vec<MachOInfo>,
    pub dwarf: Vec<DwarfInfo>,
    pub provisioning_entitlements: BTreeMap<String, serde_json::Value>,
    pub redaction_count: usize,
}
```

Use `BTreeMap` and sort every vector before serialization.

- [ ] **Step 3: Implement redaction without logging the matched value**

Compile private regexes with `OnceLock` for `RTmmo-[A-Za-z0-9_-]+`, a
case-insensitive boundary-delimited 40-hex device identifier, `/Users/<name>`, and
`C:\Users\<name>`. Replace home prefixes with `<home>`. Return the rewritten string
and replacement count. Never return or hash the matched secret or username.

- [ ] **Step 4: Run focused and crate tests**

```powershell
cargo test -p rtmmo-re redact
cargo test -p rtmmo-re model
cargo test -p rtmmo-re
```

Expected: all pass.

### Task 3: Read the IPA Safely and Inventory Plists

**Files:**
- Create: `tools/rtmmo-re/src/archive.rs`
- Modify: `tools/rtmmo-re/src/lib.rs`

- [ ] **Step 1: Write a failing synthetic IPA test**

Build an in-memory ZIP containing `Payload/Fixture.app/Info.plist` and
`Payload/Fixture.app/Fixture`. Assert:

```rust
let inventory = archive::read_ipa(&ipa_path).unwrap();
assert_eq!(inventory.entries.len(), 2);
assert_eq!(inventory.bundles[0].bundle_id.as_deref(), Some("com.riviu.fixture"));
assert_eq!(
    inventory.bundles[0].executable_path.as_deref(),
    Some("Payload/Fixture.app/Fixture")
);
assert!(inventory.entries.windows(2).all(|pair| pair[0].path <= pair[1].path));
```

Add a second test with `/absolute`, `../escape`, and `Payload/../../escape` names;
`read_ipa` must reject the archive before parsing content.

Run `cargo test -p rtmmo-re archive`; expected failure: `archive::read_ipa` absent.

- [ ] **Step 2: Implement safe in-place ZIP reading**

`read_ipa` must:

1. Hash the IPA with streaming SHA-256.
2. Open `zip::ZipArchive<File>` without extracting it.
3. Reject names whose `Path::components()` include root, prefix, or parent-dir.
4. Read each regular entry, hash it, and append `FileDigest`.
5. Parse every `Info.plist` with `plist::Value::from_reader`.
6. Resolve `CFBundleExecutable` relative to its plist directory.
7. Sort all output by normalized `/` path.

Expose an internal `ArchiveData` containing the public file/bundle inventory plus
an in-memory `BTreeMap<String, Vec<u8>>` for later parsers. Do not serialize raw
entry bytes.

- [ ] **Step 3: Add an integration assertion for the bundled artifact**

Use `env!("CARGO_MANIFEST_DIR")/../../sidecars/wda/RiviuAgent.ipa` and assert:

```rust
assert_eq!(outer.bundle_id.as_deref(), Some("com.mrph.svc"));
assert_eq!(framework.short_version.as_deref(), Some("15.1.4"));
assert_eq!(outer.dt_xcode.as_deref(), Some("2630"));
assert!(inventory.entries.iter().any(|e| e.path.ends_with("DWARF/WebDriverAgentRunner")));
```

- [ ] **Step 4: Verify archive behavior**

Run `cargo test -p rtmmo-re archive -- --nocapture`; expected: synthetic and
bundled-artifact tests pass without writing extracted files.

### Task 4: Parse Mach-O Headers, Encryption, UUIDs, Dylibs, and Sections

**Files:**
- Create: `tools/rtmmo-re/src/macho.rs`
- Modify: `tools/rtmmo-re/src/lib.rs`

- [ ] **Step 1: Write a failing handcrafted Mach-O test**

Construct a little-endian 64-bit ARM64 Mach-O header followed by
`LC_UUID` and `LC_ENCRYPTION_INFO_64`. Use UUID bytes `00..0f` and `cryptid=0`.
Assert:

```rust
let info = macho::inspect("Fixture", &bytes).unwrap();
assert_eq!(info.architecture, "aarch64");
assert!(info.is_64);
assert!(info.little_endian);
assert_eq!(info.uuid.as_deref(), Some("00010203-0405-0607-0809-0a0b0c0d0e0f"));
assert_eq!(info.crypt_id, Some(0));
```

Run `cargo test -p rtmmo-re macho`; expected failure: parser absent.

- [ ] **Step 2: Implement the object parser**

Use `object::File::parse` for generic architecture, section, and symbol data. For
`object::File::MachO64`, iterate `macho_load_commands()` and match:

```rust
match command.variant()? {
    LoadCommandVariant::Uuid(value) => uuid = Some(format_uuid(value.uuid)),
    LoadCommandVariant::EncryptionInfo64(value) => {
        crypt_id = Some(value.cryptid.get(endian));
    }
    LoadCommandVariant::Dylib(value) => {
        let name = command.string(endian, value.dylib.name)?;
        linked_dylibs.push(String::from_utf8_lossy(name).into_owned());
    }
    _ => {}
}
```

Collect `segment/section` names through `ObjectSection`, count non-empty symbols,
sort/deduplicate dylibs and sections, and return a typed error for non-Mach-O data.

- [ ] **Step 3: Inspect every executable named by plist plus every DWARF image**

Add `archive::macho_candidates(&ArchiveData)` that includes:

- each resolved `CFBundleExecutable` entry;
- each `*/Contents/Resources/DWARF/*` regular entry;
- no PNG, plist, signature resource, or provisioning profile.

- [ ] **Step 4: Verify against the real oracle**

Assert all four known Mach-O images parse, every architecture is `aarch64`, and
the framework result reports version-independent linked framework names. The
measured runtime images (outer app, XCTest bundle, and framework) each report
`cryptId=0`; the `MH_DSYM` image has no encryption load command and therefore
reports `cryptId=null`. Record both the values and command absence in the report.

Run `cargo test -p rtmmo-re macho -- --nocapture`; expected: all pass.

### Task 5: Parse Embedded Entitlements and Provisioning Metadata

**Files:**
- Create: `tools/rtmmo-re/src/codesign.rs`
- Modify: `tools/rtmmo-re/src/macho.rs`
- Modify: `tools/rtmmo-re/src/archive.rs`

- [ ] **Step 1: Write failing SuperBlob and profile tests**

Create a synthetic big-endian code-sign SuperBlob with slot type `5`, magic
`0xfade7171`, and XML plist payload:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict><key>get-task-allow</key><false/></dict></plist>
```

Assert `codesign::entitlements_from_superblob` returns
`{"get-task-allow": false}`. Create a synthetic CMS-like byte buffer containing a
plist with `ProvisionedDevices`; assert `profile_entitlements` returns the nested
`Entitlements` dictionary and omits `ProvisionedDevices` entirely.

Run `cargo test -p rtmmo-re codesign`; expected failure: module absent.

- [ ] **Step 2: Implement bounded big-endian blob parsing**

Add helpers that check every offset/length with `checked_add` before slicing.
Accept only the embedded-signature magic `0xfade0cc0`, entitlement slot type `5`,
and XML entitlement magic `0xfade7171`. Unknown slots are ignored; malformed
offsets return a typed error rather than panic.

- [ ] **Step 3: Wire `LC_CODE_SIGNATURE` into Mach-O inventory**

When a load command has `cmd()==object::macho::LC_CODE_SIGNATURE`, parse its
`LinkeditDataCommand`, slice `dataoff..dataoff+datasize`, and attach normalized
entitlements to `MachOInfo`. Convert plist values recursively to JSON while
preserving booleans, integers, strings, arrays, and dictionaries.

- [ ] **Step 4: Parse the embedded mobileprovision without shell tools**

Find the first XML or binary plist payload inside the CMS bytes, parse it, copy
only non-sensitive identity fields into bundle metadata, and return its
`Entitlements` dictionary. Explicitly drop `ProvisionedDevices`, developer
certificate bytes, and any password-like key before serialization.

- [ ] **Step 5: Verify code-sign parsing**

Run:

```powershell
cargo test -p rtmmo-re codesign
cargo test -p rtmmo-re bundled_artifact
```

Expected: no panic, entitlements are structured JSON, and no raw certificate or
device list appears in debug output.

### Task 6: Recover DWARF and Objective-C Metadata

**Files:**
- Create: `tools/rtmmo-re/src/dwarf.rs`
- Create: `tools/rtmmo-re/src/objc.rs`
- Modify: `tools/rtmmo-re/src/lib.rs`

- [ ] **Step 1: Write failing Objective-C string-table tests**

Test null-separated data containing duplicates, invalid UTF-8, one class, two
methods, route strings, and a vendor-token fixture. Assert sorted unique output
and replacement of the token before it enters `MachOInfo`.

```rust
assert_eq!(objc::strings(b"FBSession\0FBSession\0typeText:\0").values,
           vec!["FBSession", "typeText:"]);
```

Run `cargo test -p rtmmo-re objc`; expected failure: module absent.

- [ ] **Step 2: Extract Objective-C and route sections**

Read `__objc_classname`, `__objc_methname`, and `__cstring` through
`ObjectSection::uncompressed_data`. Class and method values come only from their
dedicated sections. Route candidates come from printable `__cstring` values that
start with `/` and match `[A-Za-z0-9_{}:/.\-]+`; normalize concrete session IDs to
`{sessionId}` and redact before storing.

- [ ] **Step 3: Write a failing bundled-dSYM integration test**

Read the dSYM Mach-O from the IPA and assert:

```rust
let info = dwarf::inspect(dsym_path, dsym_bytes).unwrap();
assert!(info.compile_units > 0);
assert!(info.subprograms > 0);
assert!(!info.source_paths.is_empty());
assert!(info.function_names.iter().all(|name| !name.contains("RTmmo-")));
```

Expected failure before implementation: `dwarf::inspect` absent.

- [ ] **Step 4: Implement DWARF loading with `object` plus `gimli`**

Load each `gimli::SectionId` by trying both its ELF spelling (`.debug_info`) and
Mach-O spelling (`__debug_info`), using an empty `Vec<u8>` when both are absent.
Borrow with `EndianSlice<RunTimeEndian>`, iterate units, read unit
`name`/`comp_dir`, and collect `DW_TAG_subprogram` names through
`dwarf.attr_string`. Redact, sort, and deduplicate paths/names. Invalid individual
attributes increment `attribute_errors`; a malformed unit fails that image without
aborting other IPA entries.

- [ ] **Step 5: Run metadata tests**

```powershell
cargo test -p rtmmo-re objc
cargo test -p rtmmo-re dwarf -- --nocapture
```

Expected: deterministic class/method/route sets and non-empty dSYM evidence.

### Task 7: Pin WDA 15.1.4 and Compute the Baseline Delta

**Files:**
- Create: `tools/rtmmo-re/baselines/wda-15.1.4.json`
- Create: `tools/rtmmo-re/src/baseline.rs`
- Modify: `tools/rtmmo-re/src/model.rs`
- Modify: `tools/rtmmo-re/src/lib.rs`

- [ ] **Step 1: Check in the immutable baseline lock**

Use exactly:

```json
{
  "package": "appium-webdriveragent",
  "version": "15.1.4",
  "gitHead": "20b705f8f96dee2939c022de6352720a311adb71",
  "tarball": "https://registry.npmjs.org/appium-webdriveragent/-/appium-webdriveragent-15.1.4.tgz",
  "integrity": "sha512-1tPVzIVPsBKynbTFqJyk3Hrf/FZ6kDmeP81P24hJ6q3gYHd2ljsI6OYEhINSbzxDdDmgTuWyYoUa1YtFvZC8oA=="
}
```

- [ ] **Step 2: Write failing integrity and source-scan tests**

Create a temporary source tree with `FBSession.h`, `FBSession.m`, and route
registration strings. Assert:

```rust
let source = baseline::scan_source(root.path()).unwrap();
assert!(source.objc_classes.contains("FBSession"));
assert!(source.objc_methods.contains("typeText:"));
assert!(source.route_candidates.contains("/session/{sessionId}/wda/keys"));
```

Also hash fixture bytes with SHA-512 and assert npm integrity verification accepts
the matching base64 value and rejects a one-byte mutation.

Run `cargo test -p rtmmo-re baseline`; expected failure: module absent.

- [ ] **Step 3: Implement source scanning and set comparison**

Walk only `.h`, `.m`, `.mm`, and `.swift` files. Parse Objective-C declarations,
selectors, and string-literal route candidates with compiled regexes. Return:

```rust
pub struct BaselineDiff {
    pub package_version: String,
    pub git_head: String,
    pub class_overlap: Vec<String>,
    pub class_oracle_only: Vec<String>,
    pub method_overlap: Vec<String>,
    pub method_oracle_only: Vec<String>,
    pub route_overlap: Vec<String>,
    pub route_oracle_only: Vec<String>,
}
```

All sets are sorted. The comparison must never infer that an oracle-only string is
custom code unless its source section/provenance is also recorded.

- [ ] **Step 4: Acquire and verify the exact upstream source in ignored cache**

Run:

```powershell
New-Item -ItemType Directory -Force target\rtmmo-re\baselines | Out-Null
npm pack appium-webdriveragent@15.1.4 --pack-destination target\rtmmo-re\baselines
$archive = 'target\rtmmo-re\baselines\appium-webdriveragent-15.1.4.tgz'
$hash = [Security.Cryptography.SHA512]::HashData([IO.File]::ReadAllBytes($archive))
$integrity = 'sha512-' + [Convert]::ToBase64String($hash)
$expected = 'sha512-1tPVzIVPsBKynbTFqJyk3Hrf/FZ6kDmeP81P24hJ6q3gYHd2ljsI6OYEhINSbzxDdDmgTuWyYoUa1YtFvZC8oA=='
if ($integrity -ne $expected) { throw "WDA 15.1.4 integrity mismatch" }
tar -xf target\rtmmo-re\baselines\appium-webdriveragent-15.1.4.tgz `
  -C target\rtmmo-re\baselines
```

Expected: the PowerShell integrity check succeeds and extraction creates
`target/rtmmo-re/baselines/package`. The existing
`sidecars/wda/WebDriverAgent` 16.0.0 tree remains byte-unchanged.

- [ ] **Step 5: Run baseline tests**

Run `cargo test -p rtmmo-re baseline`; expected: integrity, scanning, and sorted
delta tests pass.

### Task 8: Validate the Oracle Route Contract

**Files:**
- Create: `tools/rtmmo-re/contracts/oracle-routes.json`
- Create: `tools/rtmmo-re/src/routes.rs`
- Modify: `tools/rtmmo-re/src/model.rs`

- [ ] **Step 1: Add the evidence-backed route contract**

Use schema version 1 and these known routes:

```json
{
  "schemaVersion": 1,
  "routes": [
    {"method":"GET","path":"/status","auth":"exempt","session":"none","evidence":"crates/ios-driver/src/wda.rs"},
    {"method":"GET","path":"/wda/locked","auth":"protected","session":"none","evidence":"sidecars/pymobiledevice3/riviu_pmd.py"},
    {"method":"POST","path":"/session","auth":"protected","session":"none","evidence":"crates/ios-driver/src/wda.rs"},
    {"method":"DELETE","path":"/session/{sessionId}","auth":"protected","session":"required","evidence":"sidecars/pymobiledevice3/riviu_pmd.py"},
    {"method":"POST","path":"/wda/swipe","auth":"protected","session":"none","evidence":"crates/ios-driver/src/wda.rs"},
    {"method":"POST","path":"/wda/tap","auth":"protected","session":"none","evidence":"crates/ios-driver/src/wda.rs"},
    {"method":"POST","path":"/session/{sessionId}/wda/keys","auth":"protected","session":"required","evidence":"crates/ios-driver/src/wda.rs"},
    {"method":"GET","path":"/screenshot","auth":"protected","session":"none","evidence":"crates/ios-driver/src/wda.rs"}
  ]
}
```

- [ ] **Step 2: Write failing contract-validation tests**

Assert methods are from a closed enum, paths begin with `/`, session-required
routes include `{sessionId}`, exempt auth is limited to the explicit health list,
and no duplicate `(method,path)` pair exists. A fixture marking `/wda/keys` exempt
must fail validation.

- [ ] **Step 3: Implement parsing and static-evidence cross-checking**

Parse the JSON into typed enums. Compare normalized contract paths with the union
of Mach-O route candidates and baseline source routes. Store each route as
`confirmed`, `documented-only`, or `oracle-only`; do not send HTTP requests.

- [ ] **Step 4: Verify the route contract**

Run `cargo test -p rtmmo-re routes`; expected: checked-in contract passes and the
invalid auth/session fixtures fail with specific messages.

### Task 9: Wire the CLI, Generate Gate A Evidence, and Update Handoff Docs

**Files:**
- Create: `tools/rtmmo-re/src/cli.rs`
- Create: `tools/rtmmo-re/src/report.rs`
- Modify: `tools/rtmmo-re/src/main.rs`
- Create: `tools/rtmmo-re/tests/cli.rs`
- Create: `docs/re/rtmmo-agent/README.md`
- Generate: `docs/re/rtmmo-agent/inventory.json`
- Generate: `docs/re/rtmmo-agent/baseline-diff.json`
- Generate: `docs/re/rtmmo-agent/gate-a.md`
- Modify: `AGENTS.md`

- [ ] **Step 1: Write failing end-to-end CLI tests**

Use `std::process::Command` and a synthetic IPA. Assert:

```rust
let output = run(["inventory", "--ipa", ipa, "--output", report]);
assert!(output.status.success());
let value: serde_json::Value = serde_json::from_slice(&fs::read(report).unwrap()).unwrap();
assert_eq!(value["schemaVersion"], 1);
assert!(!String::from_utf8_lossy(&fs::read(report).unwrap()).contains("RTmmo-"));
```

Add tests that malformed IPA exits `2`, leaked-report verification exits `3`, and
valid deterministic output is byte-identical across two runs.

- [ ] **Step 2: Implement the CLI contract**

Expose:

```text
rtmmo-re inventory --ipa PATH --output PATH
rtmmo-re baseline-verify --lock PATH --archive PATH
rtmmo-re baseline-diff --inventory PATH --source PATH --archive PATH --lock PATH --output PATH
rtmmo-re gate-a --ipa PATH --inventory PATH --baseline PATH --routes PATH --baseline-source PATH --baseline-archive PATH --baseline-lock PATH --manifest PATH --output PATH
rtmmo-re verify-redaction --input PATH...
```

Use exit codes `0=success`, `2=input/parse failure`, `3=redaction failure`, and
`4=Gate A blocked`. Write reports through a temporary sibling followed by atomic
rename; append one trailing newline to JSON and Markdown.

- [ ] **Step 3: Define Gate A checks and Markdown rendering**

Gate A passes only when:

- the inventory is recomputed from the explicitly supplied IPA and exactly
  matches the supplied inventory report;
- bundled IPA hash equals `agent-manifest.json`;
- all four known Mach-O images parse; the three runtime images report an
  encryption value while the `MH_DSYM` image reports the measured command absence;
- code-sign and provisioning entitlements parse without device lists;
- dSYM has at least one compile unit and subprogram;
- framework plist version equals baseline lock `15.1.4`;
- baseline diff is recomputed from the integrity-verified archive, byte-matching
  extracted source and inventory digest;
- baseline/oracle metadata retain complete provenance and route contract validates
  all eight typed entries while static inventory confirms only their paths;
- all generated outputs pass `verify-redaction`.

Render every check with measured evidence and a final `Decision: PASS` or
`Decision: BLOCKED`. A blocked decision is a valid forensic result and prevents
Project 2; it is not converted to success.

- [ ] **Step 4: Generate the real reports**

Run:

```powershell
cargo run -q -p rtmmo-re -- inventory `
  --ipa sidecars\wda\RiviuAgent.ipa `
  --output docs\re\rtmmo-agent\inventory.json
cargo run -q -p rtmmo-re -- baseline-diff `
  --inventory docs\re\rtmmo-agent\inventory.json `
  --source target\rtmmo-re\baselines\package `
  --archive target\rtmmo-re\baselines\appium-webdriveragent-15.1.4.tgz `
  --lock tools\rtmmo-re\baselines\wda-15.1.4.json `
  --output docs\re\rtmmo-agent\baseline-diff.json
cargo run -q -p rtmmo-re -- gate-a `
  --ipa sidecars\wda\RiviuAgent.ipa `
  --inventory docs\re\rtmmo-agent\inventory.json `
  --baseline docs\re\rtmmo-agent\baseline-diff.json `
  --routes tools\rtmmo-re\contracts\oracle-routes.json `
  --baseline-source target\rtmmo-re\baselines\package `
  --baseline-archive target\rtmmo-re\baselines\appium-webdriveragent-15.1.4.tgz `
  --baseline-lock tools\rtmmo-re\baselines\wda-15.1.4.json `
  --manifest sidecars\wda\agent-manifest.json `
  --output docs\re\rtmmo-agent\gate-a.md
cargo run -q -p rtmmo-re -- verify-redaction `
  --input docs\re\rtmmo-agent\inventory.json `
  --input docs\re\rtmmo-agent\baseline-diff.json `
  --input docs\re\rtmmo-agent\gate-a.md `
  --input docs\re\rtmmo-agent\README.md
```

Expected: first, second, and redaction commands exit `0`; Gate A exits `0` for
PASS or `4` for an evidence-backed BLOCKED report.

- [ ] **Step 5: Write methodology and update AGENTS.md from the decision**

`docs/re/rtmmo-agent/README.md` must list input hashes, exact commands, schema
version, npm lock provenance, redaction rules, and the fact that no device or
production process was touched.

If Gate A is PASS, add this exact handoff invariant to `AGENTS.md`:

```markdown
- Gate A forensic inventory đã PASS; Project 2 chỉ được dùng các delta và bằng
  chứng đã version trong `docs/re/rtmmo-agent/`. Xem `gate-a.md` trước khi sửa
  standalone host hoặc WDA baseline.
```

If Gate A is BLOCKED, add this exact invariant instead:

```markdown
- Gate A forensic inventory đang BLOCKED; không bắt đầu standalone host. Các
  check thiếu và điều kiện tiếp tục nằm trong `docs/re/rtmmo-agent/gate-a.md`.
```

- [ ] **Step 6: Run full verification**

```powershell
cargo test -p rtmmo-re -- --test-threads=1
cargo test --workspace -- --test-threads=1
cargo fmt --all -- --check
git diff --check
git status --short
```

Expected: forensic tests and workspace tests pass. If global format check still
reports the documented pre-existing format debt in untouched files, run
`rustfmt --check` only on Rust files created by this plan and record both outputs
in `gate-a.md`; do not format unrelated files.

- [ ] **Step 7: Final review checkpoint**

Confirm all of the following before marking Project 1 complete:

- no RT-MMO token or 40-hex device identifier appears in generated reports;
- no `/Users/<name>` or `C:\Users\<name>` build-machine path appears in reports;
- original IPA SHA-256 still matches `agent-manifest.json`;
- `sidecars/wda/WebDriverAgent/package.json` still reports `16.0.0`;
- baseline cache reports `15.1.4` and is under ignored `target/`;
- no iPhone, desktop runtime, relay, or stream was launched by this project;
- the Gate A decision and next permitted project are explicit.

Do not start Project 2 in the same checkpoint. Gate A evidence receives review
before standalone source work begins.
