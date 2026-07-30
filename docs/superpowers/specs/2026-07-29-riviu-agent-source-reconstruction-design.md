# Riviu Agent Source Reconstruction Design

**Status:** Da duoc user duyet ngay 29/07/2026. Project 1 duoc lap ke hoach rieng
truoc khi thuc thi.

## 1. Muc tieu

Tao mot `RiviuAgent.ipa` co source build duoc, ky bang Apple team cua Riviu va
thay the runtime RT-MMO hien tai ma khong lam mat stream, gesture, clipboard hay
binh luan chu TikTok. Ket qua can la source tuong duong ve hanh vi, khong phai
ban sao source nguyen ban da bi mat khi compile.

Agent moi phai bo ba phu thuoc cua artifact hien tai:

- bundle va enterprise identity cua ben phat hanh RT-MMO;
- token co dinh cua binary RT-MMO;
- lich phat hanh va nguy co thu hoi certificate cua artifact RT-MMO.

## 2. Ngoai pham vi

- Reverse toan bo desktop `RouterMMO_iOS.exe`.
- Phuc hoi ten bien, comment hay cau truc source nguyen ban mot cach byte-perfect.
- Mo rong MDM, backup, RSD hay cac milestone Device Bridge khac.
- Thay artifact production truoc khi text comment qua live gate.
- Sua truc tiep hoac phat hanh lai binary RT-MMO nhu mot ban Riviu.

## 3. Du kien va bang chung hien co

Artifact production hien tai la `sidecars/wda/RiviuAgent.ipa`, payload
`777wealth.app`, bundle `com.mrph.svc`. Archive chua:

- outer executable `WebDriverAgentRunner-Runner`;
- plugin `WebDriverAgentRunner.xctest`;
- `WebDriverAgentLib.framework`;
- `WebDriverAgentRunner.xctest.dSYM` voi DWARF image khoang 1.1 MB;
- provisioning profile va code-signature metadata.

Thu muc `C:\RouterMMO iOS` chi co binary desktop va hai IPA; khong co project
Xcode, Swift hay Objective-C source. Repo `cloneroutermmoios` phuc dung client,
server va lifecycle phia desktop, khong chua source standalone agent. Vi vay
client clone khong du de build mot IPA doc lap.

## 4. Cac huong da can nhac

### A. Patch va re-sign binary

Nhanh cho thay doi branding, bundle hoac mot so constant. Huong nay khong tao
source bao tri duoc, van phu thuoc implementation RT-MMO va co the hong call graph,
signature hoac bootstrap khi layout binary thay doi.

### B. Decompile toan bo

Co the thu duoc pseudocode va Objective-C metadata, nhung compiler da lam mat
nhieu y nghia source. Ket qua kho review, kho test va kho nang cap theo WDA/iOS.

### C. Reconstruction theo baseline va differential oracle - duoc chon

Pin mot commit Appium WebDriverAgent lam baseline. Nhom forensic dung Mach-O,
DWARF, Objective-C metadata, route table va live response cua RT-MMO de tao cac
behavior contract. Source Riviu duoc viet tren baseline cong khai va chi them cac
delta da co test. Artifact RT-MMO chi la oracle va rollback cho den khi agent moi
qua live gate.

## 5. Ranh gio evidence va implementation

```text
RT-MMO IPA (immutable)              Pinned WebDriverAgent baseline
        |                                      |
hash + Mach-O + DWARF + routes                  |
        |                                      |
        v                                      v
Forensic inventory -> behavior contracts -> Riviu-owned source overlay
                                              |
                                              v
                                      RiviuAgent candidate IPA
                                              |
                                   differential + live gates
```

Forensic output chi duoc chua metadata, call graph, route schema, pseudocode ngan
can thiet de giai thich behavior va ket qua probe da redact. Khong sua artifact
goc. Moi lan extract phai ghi SHA-256 cua archive va tung Mach-O de ket qua co the
lap lai.

Implementation khong phu thuoc dia chi ham hay offset cua mot release RT-MMO.
Moi behavior can co contract va test doc lap; binary patch khong nam trong duong
build production.

## 6. Cau truc source dich

```text
sidecars/wda/riviu-agent/                source va Xcode config do Riviu so huu
  AgentHost/                             standalone outer app/bootstrap
  AgentRunner/                           XCTest runner entrypoint
  AgentServer/                           auth, health, protocol va route wiring
  AgentInput/                            gesture, text va clipboard adapters
  AgentStream/                           MJPEG lifecycle/settings
  Tests/                                 unit va contract tests
tools/rtmmo-re/                          deterministic forensic tooling
  baselines/wda-15.1.4.json              immutable upstream package lock
docs/re/rtmmo-agent/                     inventory, delta map va probe reports
```

Static framework plist cua oracle ghi WebDriverAgent `15.1.4`, trong khi
`sidecars/wda/WebDriverAgent/` hien la stock `16.0.0`. Baseline matcher pin npm
artifact `15.1.4` va git head `20b705f8f96dee2939c022de6352720a311adb71`, extract
vao cache ignored `target/rtmmo-re/baselines/`; no khong ghi de thu muc stock 16.0.0.

Delta cua Riviu nam trong target/source rieng hoac mot patch series co version ro
rang. Build phai pin Xcode, iOS SDK va WDA commit trong manifest.

## 7. Forensic pipeline

Pipeline chay uu tien tren macOS vi co `otool`, `nm`, `dwarfdump`, `codesign`,
`security cms` va Xcode toolchain. Windows co the chay parser deterministic bang
Python/LIEF va contract test, nhung Mac la build/sign authority.

Moi lan phan tich tao cac artifact sau:

1. Archive inventory: path, size, SHA-256, plist, provisioning va entitlements.
2. Mach-O inventory: architecture, load commands, encryption flag, linked
   frameworks, Objective-C class/category/protocol va dynamic exported symbols;
   private extern khong duoc tinh la export.
3. DWARF map: compile unit, source path, function/range va line table con lai.
4. HTTP inventory: route path co static evidence; method, session requirement,
   auth requirement va body la typed contract assertion cho toi khi contract/probe
   rieng xac nhan.
5. Baseline match: WDA release/commit gan nhat va danh sach class/route khac biet.
6. Delta map: bootstrap, auth, session, gesture, MJPEG va text-input call graph neu
   binary con du evidence; neu stripped thi ghi boundary va khong tu suy dien edge.

Gate A phai nhan IPA nhu mot input doc lap, recompute inventory tu byte artifact
va yeu cau inventory report khop tuyet doi truoc khi dung no de bind baseline
delta. Inventory tu khai va delta duoc tai sinh tu inventory gia khong tao thanh
evidence chain hop le.

Tooling phai co fixture test va khong ghi token, provisioning secret hay device ID
that vao report versioned.

Ket qua Gate A do ngay 29/07 cho thay ba runtime image da stripped; dSYM chi con
ba ham runner co range/line table. Vi vay Gate A luu toan bo exported symbol,
Objective-C metadata, route/body schema va provenance con phuc hoi duoc, dong thoi
ghi ro call graph nao vang mat. Khong suy dien call edge tu ten selector. Call graph
theo feature chi tro thanh bang chung khi Project 2/3 bo sung contract hoac probe
tuong ung; day la entry gate cua feature, khong phai ly do gia mao static evidence.

## 8. Protocol va auth cua agent Riviu

Candidate giu endpoint compatibility can thiet de driver hien tai co the A/B test,
nhung advertise identity rieng:

```json
{
  "agentVersion": "0.1.0",
  "protocolVersion": 2,
  "features": ["stream", "tap", "swipe", "clipboard"]
}
```

`text` chi duoc them vao response va release manifest sau Gate D. `pushMedia`
duoc them khi route do co contract + read-back test rieng, khong ke thua tu feature
list cua artifact oracle.

Trong A/B test, candidate dung control/MJPEG port rieng de khong tranh port voi
oracle. Manifest production moi quyet dinh port cuoi cung.

Desktop sinh token ngau nhien 256-bit cho agent Riviu, luu trong OS credential
store va truyen bang DVT process environment. Agent dung token do cho protected
routes. Khong hard-code token vendor vao source, IPA, argv, log hay manifest.
Quy tac nay chi ap dung cho candidate protocol v2; production RT-MMO hien tai van
dung fixed artifact token cho den khi Gate E chuyen runtime.

## 9. Cac delta phai phuc dung

### 9.1 Standalone bootstrap

Outer app phai plain-launch duoc, nap dung XCTest runtime/runner, bind control port
va bind MJPEG port bang environment. Readiness chi dat khi protected health,
session va frame MJPEG dau tien deu thanh cong.

### 9.2 Session va gesture

- Session tao sau khi app dich da foreground.
- Native tap/swipe phai khong dung W3C `/actions` tren TikTok.
- Logical coordinates mac dinh 375x667 nhung phai advertise qua capability, khong
  hard-code model iPhone vao desktop.
- Session moi khong duoc mo sau khi MJPEG da bat neu stream cu chua dung.

### 9.3 Text input

Day la gate kho nhat. Forensic phai trace tu `/wda/keys` den event synthesizer,
so sanh RT-MMO voi WDA baseline va tach ba kha nang:

1. RT-MMO chi sua session/focus/timing.
2. RT-MMO sua WDA typing/event construction.
3. RT-MMO dung private framework, entitlement hoac bootstrap provenance rieng.

Implementation chi duoc advertise `text` sau khi Unicode text hien trong Settings
va TikTok. HTTP 200 tu `/wda/keys` khong phai bang chung thanh cong.

### 9.4 MJPEG

Agent phai ton trong `MJPEG_SERVER_PORT`, chi co mot upstream reader va ho tro
settings can thiet ma khong wedge control channel. Readiness can frame JPEG hop le,
khong chi can port mo.

## 10. Go/no-go gates

### Gate A - Forensic completeness

- Xac dinh encryption status, architecture, entitlements va linked frameworks.
- Parse duoc dSYM/Objective-C metadata va baseline match.
- Tao route inventory va delta map co bang chung.
- Baseline delta phai duoc recompute tu npm tarball dung integrity, source tree
  trung byte va inventory digest; version/git head don le khong du de PASS.
- Moi metadata value phai co source/image provenance. Neu binary stripped lam mat
  call graph thi report phai ghi ro boundary va project sau phai co contract/probe
  truoc khi implement delta theo feature.

### Gate B - Riviu standalone host

- Candidate do Riviu build va sign plain-launch tren iPhone test.
- Protected health, fresh session va session-before-stream deu pass lap lai.

### Gate C - Control parity

- 50 native tap, 20 swipe va 5 phut MJPEG khong mat session.
- Clipboard, Unicode trong Settings va recovery bounded pass.

### Gate D - TikTok text parity

- It nhat 10 lan mo composer va go Unicode qua fresh text session.
- It nhat 5 binh luan duoc frame xac nhan `Open -> SendArmed -> Open`.
- Khong dem ACK, draft cu, emoji fallback hay comment khong co read-back evidence.

### Gate E - Product replacement

- Candidate dung bundle/signing identity cua Riviu va manifest protocol v2.
- Desktop preflight, repair, rollback N-1 va secret scan pass.
- Chay soak tren fleet canary truoc khi promote.

Neu Gate D chua dat, artifact hien tai van la production runtime. Candidate khong
duoc gan feature `text` va desktop khong duoc tu dong chuyen sang no.

## 11. Kiem thu

- Unit test cho parser inventory, redaction, route normalization va baseline diff.
- Golden fixture cho Mach-O/plist/DWARF metadata, khong version binary that vao test.
- Contract suite chay cung payload tren oracle va candidate, so sanh status class,
  side effect va frame evidence thay vi so sanh raw response mot cach mong manh.
- Xcode unit/integration test cho auth middleware, environment parsing, route
  registration va lifecycle state machine.
- Live test phai chay doc quyen mot runtime tren mot device; khong de desktop,
  harness va oracle tranh usbmux cung luc.

Moi claim ve text, stream hoac gesture phai gan artifact directory, trace va frame
evidence. Test khong duoc dua vao `/source` sau tren TikTok vi accessibility tree
da duoc chung minh la khong day du.

## 12. Signing, release va rollback

- Development candidate ky bang Apple team cua Riviu tren Mac va dung bundle
  `com.riviu.managersphone.agent`.
- Release manifest ghi agent/protocol version, iOS range, Xcode build, bundle,
  signer, SHA-256, features va artifact N-1.
- Free team chi phu hop device nghien cuu; fleet can provisioning phu hop voi so
  luong may va chu ky mong muon.
- RT-MMO oracle khong bi overwrite. Khi A/B test can doi artifact, lifecycle phai
  kill/uninstall dung bundle va xac minh process fingerprint.

## 13. Phan ra cac project thuc thi

Day la master design. Khong viet mot implementation plan duy nhat cho toan bo
pham vi. Moi project sau co spec/plan, test va review gate rieng:

1. **Forensic inventory va baseline match:** tooling deterministic, Mach-O/DWARF
   inventory, route map va bao cao Gate A. Day la project thuc thi dau tien.
2. **Standalone host va control parity:** source/Xcode target, auth, session,
   native gesture va MJPEG de dat Gate B-C.
3. **Text-input parity:** trace/reconstruct typing delta, Settings probe va TikTok
   live Gate D. Khong trien khai product migration trong project nay.
4. **Product migration:** protocol v2, per-install credential, manifest, repair,
   rollback va canary Gate E.

Project sau chi bat dau khi gate truoc co evidence dat. Neu Gate A cho thay mot
dependency khong the tai tao bang signing identity cua Riviu, bao cao phai chot
root cause va dieu kien can them truoc khi vao project 2.

## 14. Tieu chi hoan thanh

Cong viec chi duoc goi la bo phu thuoc RT-MMO khi:

1. Source agent moi build sach tren Mac tu commit da pin.
2. IPA ky bang identity cua Riviu va dung token Riviu sinh.
3. Desktop chi giao tiep voi protocol Riviu, khong can `RIVIU_RTMMO_TOKEN`.
4. Stream, gesture, clipboard va text comment qua tat ca live gates.
5. Current production artifact chi con la rollback co thoi han, sau do duoc go khoi
   goi cai dat chinh.

Cho den luc do, UI va tai lieu phai noi ro runtime nao dang duoc dung; rebrand IPA
khong duoc tinh la source independence.
