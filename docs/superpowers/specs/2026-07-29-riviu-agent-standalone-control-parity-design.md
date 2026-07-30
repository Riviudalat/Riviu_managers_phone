# Riviu Agent Standalone And Control Parity Design

**Status:** Da duoc user duyet theo master design va lenh trien khai ngay
29/07/2026. Day la Project 2; Project 3 text-input va Project 4 product migration
khong nam trong checkpoint nay.

## 1. Muc tieu

Tao source va duong build cua mot Riviu Agent candidate doc lap tu WDA `15.1.4`
da pin, sau do chung minh hai nhom hanh vi:

1. **B0 standalone feasibility:** app do Riviu build plain-launch duoc va nhan
   automation session tren iPhone test ma khong can `xcodebuild test` hay RT-MMO.
2. **Gate B-C:** candidate co auth Riviu, fresh session, native tap/swipe,
   clipboard va MJPEG on dinh theo contract da version.

Candidate chi la artifact A/B cho den khi vuot Gate D/E. Production
`sidecars/wda/RiviuAgent.ipa`, `agent-manifest.json`, bundle `com.mrph.svc` va
credential RT-MMO khong bi sua trong Project 2.

## 2. Evidence boundary

Gate A PASS xac nhan oracle dung WDA `15.1.4`, co outer XCTest runner app,
embedded `WebDriverAgentRunner.xctest`, `WebDriverAgentLib.framework` va ba ham
runner con DWARF. Runtime image stripped nen khong co feature call graph cua
standalone bootstrap.

Vi vay Project 2 khong suy dien rang chi goi `FBWebServer` trong mot app thuong la
du. Candidate dau tien phai dung XCTest runner do Xcode sinh tu UI-test target;
plain launch, protected health va fresh automation session la probe quyet dinh.
Neu B0 khong cap duoc automation session, dung o feasibility report va khong port
them auth/control delta len mot host sai.

Khong port selector oracle-only nhu `AMPatcher`, `handleRTRefreshStream:` hay
private bootstrap chi tu ten selector. Moi delta feature phai co contract hoac
probe truoc khi vao source.

## 3. Cac huong da can nhac

### A. Vendor toan bo WDA 15.1.4

De mo Xcode project ngay nhung tao mot ban sao source lon, kho theo doi provenance
va de vo tinh sua code baseline.

### B. Baseline lock + Riviu overlay - duoc chon

Tarball npm `15.1.4` duoc verify theo lock Gate A, extract vao `target/`, sau do
ap patch series Riviu co hash. Source build la baseline byte-exact cong delta nho,
review duoc. Stock WDA `16.0.0` trong `sidecars/wda/WebDriverAgent/` khong bi dung
lam baseline va khong bi ghi de.

### C. Viet automation server tu dau

Loai bo WDA dependency nhung phai phuc dung XCTest/testmanagerd, HTTP, session,
gesture va MJPEG cung luc. Huong nay tang rui ro ma khong them gia tri cho Gate
B-C.

## 4. Cau truc source

```text
sidecars/wda/riviu-agent/
  README.md                         ownership, build va gate status
  baseline-lock.json               ref toi npm WDA 15.1.4 + patch digests
  Config/
    RiviuAgent.xcconfig             bundle/version/deployment target
  AgentHost/
    README.md                       outer XCTest runner boundary
    patches/                        signed typed artifact attestation
  AgentRunner/
    patches/                        runner/bootstrap delta co evidence
  AgentServer/
    patches/                        auth, identity, protected health
  AgentInput/
    patches/                        sessionless native tap/swipe
  AgentStream/
    README.md                       baseline MJPEG + lifecycle contract
  Contracts/
    control-v2.json                 versioned request/response schema
  Scripts/
    prepare.py                      verify/extract/apply deterministic
    build_candidate.py              Xcode build/sign/package/manifest
    probe_gate_bc.py                sequential Mac/device probe
  Tests/
    test_prepare.py                 Windows-compatible source tests
    fixtures/                       synthetic tar/contract fixtures only

target/riviu-agent/                 ignored generated source/build/output
docs/re/riviu-agent/                generated probe reports, no secrets
```

Patch series la source delta cua Riviu. `prepare.py` phai fail neu archive,
baseline tree, patch digest hoac output digest lech; khong duoc silently dung WDA
16.0.0 hien tai. Digest tinh moi regular file va canonical mode `0644`/`0755`
trong cay extract, gom ca `project.pbxproj`, build config, `.plist` va executable
bit cua build script. POSIX extraction phai restore mode tu tar. Xcconfig ben
ngoai generated tree co SHA-256 rieng trong lock va signed attestation; thay doi
bat ky build input nao phai lam mot attestation thay doi, khong chi
Objective-C/Swift.

## 5. Standalone host checkpoint B0

Candidate dung `WebDriverAgentRunner` UI-test target cua baseline de Xcode tao
outer `*-Runner.app` va embed `.xctest`. Build script dat identity rieng, package
thanh IPA va launch bang DVT app launch voi environment, khong goi
`xcodebuild test-without-building` trong probe.

Environment candidate:

| Bien | Bat buoc | Gia tri |
|---|---|---|
| `USE_PORT` | co | control port A/B, mac dinh `8916` |
| `MJPEG_SERVER_PORT` | co | MJPEG port A/B, mac dinh `9094` |
| `RIVIU_AGENT_TOKEN` | co | random 256-bit token, chi qua DVT environment |
| `WDA_PRODUCT_BUNDLE_IDENTIFIER` | co | bundle candidate da ky |

B0 chi PASS khi 5 cold-launch lap lai deu dat chuoi:

```text
kill exact candidate -> process absent + ports closed
-> DVT plain launch + stable new PID
-> GET /status -> protected GET /riviu/health
-> foreground Settings -> POST /session -> session command succeeds
```

HTTP port len nhung khong co automation session la FAIL. Token khong duoc nam
trong argv, source, IPA, manifest, log hay report.

Build setting co so la `com.riviu.managersphone.agent`; runner da ky bat buoc co
bundle cuoi `com.riviu.managersphone.agent.xctrunner`. Build fail neu plist va
codesign identity khong cung dung gia tri nay.

Sau metadata attestation nam truc tiep trong signed bundle
`PlugIns/WebDriverAgentRunner.xctest/Info.plist`:
source SHA-256, xcconfig SHA-256, protocol integer `2`, Objective-C tests `PASS`,
Xcode version va Xcode build. Nam string duoc expand tu command-line build setting sau khi unit
test dat; protocol la `<integer>2</integer>` trong source. Xcconfig bat
`INFOPLIST_EXPAND_BUILD_SETTINGS=YES`. Khong dung custom user-defined
`INFOPLIST_KEY_RiviuAgent*`: Xcode khong sinh arbitrary key tu co che do cho target
upstream dang dung Info.plist san.

Voi Xcode >=26, candidate host chi duoc package sau khi co du bon runtime:
`Testing.framework/Testing`, `_Testing_Foundation.framework/_Testing_Foundation`,
`lib_TestingInterop.dylib` va `libXCTestSwiftSupport.dylib`. Build copy hai
device runtime con thieu tu active iPhoneOS platform, re-sign dependency -> xctest
-> outer app, roi bat buoc `codesign --verify --deep --strict` lai.

## 6. Protocol v2 va auth

Header candidate la `X-Riviu-Token`; khong ke thua ten header vendor. Token la
chuoi khong rong dai dien 256 bit va duoc so sanh constant-time. Thieu token env
lam candidate fail startup. Thieu/sai header tra HTTP 401 voi body W3C nho, khong
echo token. Auth duoc chan tai `FBHTTPConnection` truoc route dispatch de bao gom
ca server-key route nhu `/health` va `/wda/shutdown`; boc rieng command handler la
khong du.

Chi `GET /status` duoc mien auth de discovery. `/health`, `/wda/healthcheck`,
`/wda/locked`, screenshot, session, input va clipboard deu protected.
MJPEG la socket rieng nhung cung nam trong auth boundary: bind loopback va phai
nhan `X-Riviu-Token` dung truoc khi dang ky stream client; missing/wrong/correct
tra 401/401/200.

`GET /riviu/health` tra:

```json
{
  "value": {
    "agentVersion": "0.1.0",
    "protocolVersion": 2,
    "features": ["stream", "tap", "swipe", "clipboard"],
    "logicalWidth": 375,
    "logicalHeight": 667,
    "state": "ready"
  }
}
```

`GET /status` giu response WDA va them cung identity duoi key
`value.riviuAgent`. `text` va `pushMedia` khong xuat hien trong health, status
hay candidate manifest Project 2.
Neu MJPEG bind loi, health bo feature `stream` va tra `state=degraded`; chi bind
thanh cong moi tra payload `ready` o tren.

## 7. Route contract

| Method | Path | Auth | Session | Body / ket qua |
|---|---|---|---|---|
| GET | `/status` | exempt | none | WDA status + `riviuAgent` |
| GET | `/riviu/health` | protected | none | identity, features, logical size, state |
| GET | `/wda/locked` | protected | none | auth/readiness proof |
| POST | `/session` | protected | none | WDA capabilities; live probe policy bat buoc `firstMatch` |
| GET | `/session/{sessionId}` | protected | required | active-session control check |
| DELETE | `/session/{sessionId}` | protected | required | close exact session |
| POST | `/wda/tap` | protected | none | required finite `x`, `y` |
| POST | `/wda/swipe` | protected | none | finite from/to; `delay` in `[0, 5]` |
| POST | `/wda/setPasteboard` | protected | none | baseline base64 clipboard schema |
| POST | `/wda/getPasteboard` | protected | none | byte-exact read-back |
| GET | `/screenshot` | protected | none | diagnostic only |
| POST | `/session/{sessionId}/element` | protected | required | Settings SearchField probe |
| POST | `/session/{sessionId}/element/{elementId}/click` | protected | required | focus control probe |
| POST | `/session/{sessionId}/element/{elementId}/clear` | protected | required | clear control probe |
| GET | `/session/{sessionId}/element/{elementId}/text` | protected | required | exact Unicode read-back |
| POST | `/session/{sessionId}/wda/keys` | protected | required | control probe only; no `text` feature |

Sessionless gesture handlers tao truc tiep `XCPointerEventPath` va
`XCSynthesizedEventRecord`, roi gui bang
`FBXCTestDaemonsProxy synthesizeEventWithRecord:timeout:error:` voi deadline 5
giay va chi ACK khi callback khong loi va BOOL result la true. Chung khong goi W3C
`/actions`, `XCUICoordinate` gesture, `pressForDuration:thenDragToCoordinate:` hay
`fb_waitUntilStable`. Day la delta co evidence tu cac selector oracle
`handleHCTap:`, `handleHCSwipe:`, `hcEmit:offsets:tag:` va log HCTouch; path
XCUICoordinate cua baseline da tung wedge TikTok nen khong dat parity.

Tap tao mot pointer path down/up tai `{x,y}`. Swipe tao down tai `from`, move toi
`to` theo `delay`, roi up. Interface orientation map truc tiep tu
`XCUIDevice.sharedDevice.orientation`; handler khong query active app,
accessibility hierarchy hay hit point. Body khong phai dictionary, them key la,
Boolean/string/NaN/Infinity deu tra invalid-argument.
Client co the bieu dien tap bang swipe 1 px, con server van giu route tap/swipe
rieng theo contract.

Route clipboard phai duoc them vao Project 2 contract truoc khi claim parity vi
Gate A contract ban dau chua chot body/read-back cua route nay.

## 8. Session va stream lifecycle

Fresh session chi tao sau khi target app da foreground. Probe va client phai dung
stream/relay cu truoc khi tao session moi; MJPEG reader chi ket noi sau session.
Candidate khong prime `snapshotMaxDepth` theo rule stock neu probe B0 cho thay
standalone session khong can; moi thay doi phai co trace rieng.

MJPEG dung implementation baseline da harden, ton trong `MJPEG_SERVER_PORT`, bind
loopback va auth truoc stream. Probe chi tinh first-frame sau khi Pillow decode
JPEG hop le; port open, marker SOI/EOI gia hoac mot frame som khong du. Gate C yeu
cau mot reader, reconnect bounded, cadence lien tuc va khong mat control session
trong 5 phut.

## 9. Error va report contract

- Config/integrity/signing error fail truoc install. Probe verify candidate
  manifest, locked source digest va IPA SHA-256, sau do uninstall exact bundle,
  fresh-install va doi chieu installed identity truoc moi launch.
- 401 la auth failure, khong bi phan loai thanh transport/session.
- Invalid body tra 400; invalid/stale session giu W3C session error.
- Probe stage JSON/Markdown, chay `rtmmo-re verify-redaction` tren ca ba file roi
  moi atomic-publish; evidence kem artifact/manifest/source SHA-256, bundle,
  signer hash/team, Xcode, iOS, trace step va outcome.
- Secret/device identity duoc redact; verifier tu Gate A chay tren moi report.
- Moi live claim co artifact directory. Tren Windows, live field ghi ro
  `PENDING_MAC_DEVICE`, khong PASS.

## 10. Gate B-C

### B0 - Standalone feasibility

- Source prepare lap lai va Xcode build thanh cong tu baseline lock.
- Objective-C `UnitTests` compile/chay thanh cong truoc runner build va ket qua
  duoc dong vao signed Info.plist, sau do manifest chi doc lai tu app da verify.
- IPA do Riviu sign, plain-launch bang app launch.
- 5/5 cold launch co protected health va fresh automation session.

### Gate B - Standalone host

- Auth matrix missing/wrong/correct token cho ket qua 401/401/200.
- Manifest/source/IPA digest chain va fresh installed identity khop candidate.
- Chuoi foreground target -> fresh session -> MJPEG first frame lap lai 5/5.
- Token scan raw manifest, decompressed IPA entries, reconstructed source, locked
  xcconfig, argv, guarded log va serialized report sach. Xcconfig phai duoc rehash
  va khop digest manifest truoc khi ghi `xcconfigTokenScanClean`.
- Moi cold launch co witness process cu bien mat, hai port dong va PID moi on dinh.
  Lookup sau health/session/JPEG dau phai khop PID DVT launch; terminate truoc vong
  ke hoac cleanup cuoi phai tra dung PID da xac nhan. PID fingerprint cua nam vong
  phai khac nhau.

### Gate C - Control parity

- 50 tap va 20 swipe co causal frame evidence tren vung Settings da dinh nghia,
  moi action co bon control samples (initial + ba frame moi), doc lai dung switch
  element ID ban dau va khong mat session.
- MJPEG 5 phut co JPEG hop le, >=1 FPS, max frame gap <=2 giay, reconnect <=1;
  protected health va active-session command thanh cong moi 5 giay; cycle <=5s,
  completion gap <=5.5s va schedule lateness <=0.5s, khong catch-up sau stall.
- Clipboard ASCII + Unicode set/get byte-exact only after foregrounding the
  candidate with `kill_existing=false`, proving its PID did not change, and reading
  the same bundle/PID from `/wda/activeAppInfo`; an ACK while Settings remains
  foreground is not evidence.
- Unicode `/wda/keys` trong Settings co read-back de lam control probe, nhung
  feature `text` van tat.
- Project 2 khong do soft/hard desktop recovery: candidate chua duoc noi vao
  desktop runtime, nen moi fault trong probe lam Gate C FAIL. Recovery budget va
  hard recycle la acceptance cua Project 4 khi desktop chuyen sang candidate;
  MJPEG reader trong Project 2 chi co bounded reconnect `<=1` va max gap `<=2s`.

## 11. Testing va execution boundary

Windows chay duoc:

- lock/integrity/extraction/path traversal tests;
- patch apply/reproducibility va source invariant tests;
- JSON Schema/route/auth matrix fixture tests;
- report redaction va secret scan;
- Rust workspace regression.

Truoc khi ap input patch, `Contracts/native-input-v1.json` phai rang buoc body,
event timeline, direct synthesizer API va danh sach high-level API bi cam. Static
oracle evidence va live trace 28/07 la provenance cua contract; side effect tren
candidate van chi PASS sau Gate C.

Mac + iPhone bat buoc cho:

- Xcode compile/link/sign/package;
- plain launch va testmanagerd automation session;
- protected HTTP live, first JPEG, gesture, clipboard, Settings Unicode;
- Gate B-C report.

Project 2 ket thuc o Gate C. TikTok text comment, feature `text`, production
manifest replacement va bo `RIVIU_RTMMO_TOKEN` thuoc Project 3/4.
