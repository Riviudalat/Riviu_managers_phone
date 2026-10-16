# TikTok Global 45.7.3 (`com.zhiliaoapp.musically`)

Đây là kiến thức lâu dài của Riviu về build 45.7.3, build duy nhất được phép chạy effect
trên fleet Android. Nguồn gồm phân tích tĩnh APK gốc (2026-10-09), audit tận dụng APK,
probe sound picker trên máy #44, probe share/deep link trên máy #46 và dump đăng nhập
trên máy #31. Thư mục research nằm trong `target/` (git-ignore) và không thuộc repo.
Quy trình đọc một build mới nằm ở [Build TikTok mới](../11-build-tiktok-moi.md).

Mức bằng chứng dùng trong tài liệu:

- **APK**: đọc tĩnh từ APK, chỉ là ứng viên.
- **Máy**: dump uiautomator trên máy thật (S8+, Android 9) hoặc fixture đã commit.
- **EMU**: emulator Android Studio, chỉ có màn đăng ký/đăng nhập.
- **Chưa đo**: cần đọc trên máy thật trước khi code dựa vào.

Fleet chạy giao diện **tiếng Anh**. Chuỗi tiếng Việt bên dưới chỉ dùng để đối chiếu dump cũ.
Tài liệu chỉ chứa id, chuỗi UI ngắn và tên class. Không chứa dump, bảng chuỗi, mã
decompile, tên tài khoản hay secret.

## 1. Danh tính APK

| Thuộc tính | Giá trị |
|---|---|
| Package | `com.zhiliaoapp.musically` |
| versionName / versionCode | 45.7.3 / `2024507030` |
| Kích thước | 565 763 976 byte |
| SHA-256 của APK | `E76790E76417964F60EC5C58032008DFF2CB0B9D7AD40E322D5120BD3F921C22` |
| SHA-256 chứng chỉ signer (đầu) | `9041803e91bcb814…` (`CN=musical.ly, O=musical.ly Inc.`) |
| Lược đồ chữ ký | v1, v2, v3 và SourceStamp |
| ABI | arm64-v8a, armeabi-v7a |

Bản APKPure trùng từng byte với `base.apk` kéo từ máy #44. Kiểm lại bằng
`verify_apk.py --expect-version 45.7.3` trước khi cài lên máy nào; signer lạ thì dừng.

## 2. Phát hiện về cấu trúc

1. **Chuỗi nằm trong bảng NxString, không nằm trong `resources.arsc`.** Bảng `default` là
   tiếng Anh (38 200 mục). Mỗi locale có bảng ghi đè riêng, ví dụ `vi-VN` có 34 829 mục. Byte được
   XOR với `0x0F`, và `tools/tiktok-apk/nxstring.py` giải được. Bảng `vi` (574 mục) gần như
   chỉ là bản sao tiếng Anh, nên phải ghi locale thật của máy (`vi_VN` hay `vi`).
2. **Token `@<số thập phân>`** trong `content-desc` là id của một chuỗi mà accessibility
   không giải được. Số này giống nhau ở mọi ngôn ngữ nhưng đổi theo từng build, nên chỉ
   dùng làm mốc **cho riêng 45.7.3**. Ba token Riviu bám vào:

   | Id view | Token | en | vi |
   |---|---|---|---|
   | `cqf` | `@2131823090` | Post comment | Đăng bình luận |
   | `sn3` | `@2131823130` | Record video | Quay video |
   | `bix` | `@2131823215` | Back | Quay lại |

   Trong fixture còn gặp `@2131823053` (Remove sound, `doi`), `@2131823218` (Close,
   `kbm`), `@2131828743` (Close, `q2n`: dấu × của phần thưởng nổi trên feed, không phải
   nút của lớp hướng dẫn) và `@2131823216` (Bottom sheet). Trên máy #31 tiếng Việt, token
   cuối được giải thành "Trang tính dưới cùng". Lệnh `resolve_desc.py --scan` đọc được cả
   dump dạng `<node>` lẫn dạng tag theo tên class.
3. **Id gán từ code, không khai báo trong layout nào:** `s0v` (handle trên hồ sơ, icon
   `s0t`), `nr_`/`nrb` (tab Home/Profile), `f72`, `rzs`, `rv2`, `rzt`, `s10`, `f6y`,
   `f71`, `f6w`, `f74`, `drt`, `sy3`, `sy6`, `rjy`. Không có bằng chứng layout cho các id
   này, nên phải tìm chúng bằng dump runtime. Chúng có thể phụ thuộc cờ thử nghiệm, vì vậy
   luôn kèm kiểm class, parent và text.
4. **Màn chọn cách đăng nhập là Jetpack Compose.** Tiêu đề không có resource-id. Mỗi lựa
   chọn là một `android.view.View` có `content-desc` bằng đúng nhãn, không phải `Button`.
   Điều này đã đo trên máy (vi) và EMU (en).
5. **jadx chạy toàn bộ APK lặng lẽ bỏ sót class.** Chỉ decompile dex chứa hằng số cần tìm:
   tìm bằng `dex_lookup.py const`, rồi `dex_lookup.py classdef`.
6. **Thẻ quảng cáo trên feed có hai bề mặt** (`crates/core/src/nurture/ad_card.rs`; nguồn:
   `aapt2-resources.txt`, `layout_index.jsonl`, bảng chuỗi `default`, và
   `commercialize/feed/CommerceVideoDelegateKt` đã decompile; thẻ kết thúc mới chỉ thấy trên
   ảnh chụp `acceptance-20261010/evidence-h/b064-norail.png` của máy
   `ce031713b064841e0c`, 10/10/2026):
   - Lúc đang phát: thẻ có thanh hành động như thường, thêm khung nút quảng cáo
     `ad_feed_card_container_v2` (con của `y8z` trong layout `b7l`/`d07`, `visibility=gone`
     khi không phải quảng cáo) và nhãn "Sponsored" từ server dưới tên tác giả. Nuôi chỉ xem,
     không tim/lưu/bình luận/follow.
   - **Máy** (`ce0417141d4decde0c`, en, 10/10/2026): thẻ quảng cáo có thể không có
     `ad_feed_card_container` lẫn "Sponsored". Dấu hiệu đã đo là nhãn dưới caption
     `:id/hnj` (`Button`, text đúng "Ad", trong `lzt`), kèm desc nhạc
     "Sound: Promoted Music…". Desc nhạc chỉ là bằng chứng phụ, không tự đánh dấu thẻ.
   - Thẻ kết thúc: khi video quảng cáo hết, delegate bơm lớp Lynx qua `view_stub_ad_mask`
     (con của `a4v`, gốc thẻ) và đặt cả `widget_container` thành INVISIBLE, nên thanh hành
     động, caption và tác giả **biến khỏi cây** trong khi tab For You và thanh Home vẫn còn.
     Lớp này vẽ logo nhà quảng cáo, một nút CTA và nút Close. Chuỗi CTA trong bảng `default`:
     "Book now" (`0x7f11114c`, nút trên ảnh chụp), "Shop now", "Learn more", "Download",
     "Install now", "Sign up", "Order now", "Apply now", "Contact us", "Play now", "Play game",
     "Get quote", "Get offer", "Watch now", "View now", "View profile", "Visit website",
     "Send message", "Interested", "Open now", "Get showtimes"; bảng `vi` (574 mục) không có
     bản dịch, nên bộ nhận chỉ bật ở `en`. Close gửi `action_ad_hide_lynx_mask`: delegate chỉ
     ẩn lớp phủ và đặt lại `widget_container` VISIBLE, không mở trang nào; vuốt lên trên lớp
     phủ là sự kiện `action_ad_swipe_up_video` do chính lớp Lynx nhận rồi gọi
     `VerticalViewPager` sang thẻ kế, nên pager không nhận cử chỉ thô. Nuôi không bao giờ bấm
     CTA; chỉ vuốt thẳng, và bấm Close đúng một lần mỗi thẻ khi vuốt chưa rời được. Id
     `feed_ad_lynx_mask_container_spark` và `tv_sponsored` (chỉ trong mask splash `aax`) chưa
     thấy trên dump nào, nên chưa dùng.
7. **Toast yêu thích "Saved · Manage" nằm trong PopupWindow riêng.**
   - **APK**: lớp `X.0vrI`, tag `add_to_favorites_bottomwindow`, layout `em`; rộng hết màn,
     cách đáy pager khoảng 72dp, tự đóng sau 3 s. Snapshot chỉ đọc một cửa sổ không thấy nó.
   - **Máy** (`ce0417141d4decde0c`, SM-G955N, Android 9, 10/10/2026): DOWN của cú vuốt rơi
     lên toast, chạm làm toast đóng, InputDispatcher hủy cử chỉ ("touched window was
     removed") và UiAutomator2 trả `invalid element state` / "Unable to perform W3C
     actions". Riviu gọi đó là `GestureCancelled`
     ([`driver.rs`](../../../crates/core/src/driver.rs)): không chứng minh cử chỉ đã hay
     chưa tác động, nên điều hướng đọc lại màn hình còn tap effect giữ trạng thái chưa rõ.

**Cửa sổ focus không phân biệt màn soạn bài.** Đo trong r128–r129 (15–16/10/2026, 30 máy
SM-G955F/N, 237 lần soạn, chẩn đoán `settleAfterCreate` / `settleAfterGalleryEntry` /
`settleAfterEditNext`): trước và sau mỗi lần bấm Create, ô thư viện hay Next, `mCurrentFocus`
luôn là `com.ss.android.ugc.aweme.splash.SplashActivity`. Camera, picker và editor dùng chung
một activity, nên `dumpsys window` không dùng được làm cổng "màn đã đổi". Lần đọc cây đầu tiên
sau khi bấm mất p50 1,8 s, p90 2,7 s, tối đa 9,8 s, và chưa lần nào trả về màn cũ.
Ảnh chụp sau Next của picker (r134, box 21-50): 19/19 khung chụp 0,1–0,4 s sau ACK vẫn là
picker, 19/19 khung từ 0,7 s trở đi đã là editor. Vì vậy khung ảnh đầu của bước chờ editor
chờ 1 s (gf93).

## 3. Bảng id Riviu dùng cho 45.7.3

Bảng lấy từ các match arm `"45.7.3"` trong code (75 id, không id nào thiếu trong build).
Cột "Layout" là số layout khai báo id. Số lớn nghĩa là id dùng chung, phải scope.
"code" nghĩa là id được gán từ code.

**Đăng bài: composer, picker, editor, sound**

| Id | Layout | Là gì | Scope trong Riviu |
|---|---|---|---|
| `upload_hot_area` | 2 | Lối vào upload trên camera | Đúng id trên màn camera |
| `sn3` | 3 | Nút chụp (shutter) | Kèm token `@2131823130` |
| `k7u` | 4 | RecyclerView danh sách album | Chỉ khi en/45.7.3; đo trên `ce031713b0c610ab0c` |
| `k_x` | 3 | Ô chọn ảnh, góc mang số thứ tự | Trong lưới picker; Next xác nhận đúng số ảnh |
| `wjp` | 5 | Next của album ("Next (n)") | Có biến thể "Confirm"; kiểm text |
| `p31` / `p2y` | 2 / 7 | Next của editor | **Máy** (#44 `nav_editor.xml`, #46 B1j): `p2y` là khung bấm của Next, chứa `p31`; "Your Story" (`zy4`) nằm trong khung bấm `sh3` ngay bên trái. Chỉ dùng `p31`/text, không đoán vị trí |
| `dou` | 2 | Chip "Add sound" | Cùng hàng với `doi` (Remove sound), `dop` (Sounds) |
| `tv_top_text` | 5 | Chữ chạy trên chip sound: "Add sound" hoặc tên sound đã chọn | Trong chip `dou` |
| `wrv` | 1 | Nhãn tab sound: For You / Hot / Favorites / Recent | Kiểm text tab |
| `opr` | 1 | Hàng sound bấm được trong picker | Bọc `vertical_item_music_new_rl` |
| `vertical_item_music_new_rl` | 5 | Nội dung một hàng sound | Trong `opr` |
| `title` | 410 | Tên sound trong hàng | Chỉ trong hàng sound |
| `z3k` | 9 | Tên nghệ sĩ trong hàng | Chỉ trong hàng sound |
| `nms` | 7 | Animation Lottie "đang phát" | Chỉ có trên hàng đang phát |
| `viewpager_container` | 4 | Pager của sheet sound | Trong sheet `fl4` |
| `bix` | 61 | Icon Back | Rất chung; kèm token `@2131823215` |
| `gpr` | 5 | EditText mô tả bài đăng | Còn dùng ở màn LIVE; kiểm ancestor |
| `gpv` | 2 | EditText tiêu đề ("Add a catchy title") | Trên màn đăng |
| `sa3` | 7 | Nút Post | Kiểm text "Post"; sheet khác cũng dùng `sa3` |
| `tv_title` | 304 | Tiêu đề chung | Không bao giờ dùng trần |

**Feed, hồ sơ, tìm kiếm**

| Id | Layout | Là gì | Scope trong Riviu |
|---|---|---|---|
| `nr_` / `nrb` | code | Tab Home / Profile | Desc "Home"/"Profile" kèm class |
| `nrh` / `nrm` / `wr2` | 2 / 2 / 3 | Thanh điều hướng / gốc / nội dung feed | Mỗi id đúng một node, `wr2` là con trực tiếp của `nrm` |
| `hpk` | 8 | Trang feed (FrameLayout nền) | Đúng một node, không text/desc |
| `long_press_layout` | 16 | Vùng video (desc "Video") | Trong trang feed |
| `fpv` | 8 | Nút Like và Share trên rail (cùng id) | Phân biệt bằng desc "Like video. N likes" / "Share video. N shares" |
| `hlx` / `hly` / `hku` / `hko` | 1 / 2 / 4 / 6 | Nút Favorite (Lưu) | Theo bộ bốn id của `tiktok_save` |
| `e7q` | 3 | Ô "Add comment..." | Trong sheet bình luận |
| `cqf` | 7 | Gửi bình luận | Kèm token `@2131823090`; trong DM cùng id là "Send" |
| `o28` / `o3b` / `o2g` / `o2n` | 1 / 1 / 3 / 1 | Gợi ý @mention: handle / hàng / danh sách / tên | Chỉ khi en; hàng nằm trong danh sách |
| `s0v` | code | Handle `@…` trên hồ sơ | Nguồn chứng minh tài khoản; kèm class `Button` |
| `rzl` / `f72` | 7 / code | Khung hồ sơ / nút Follow | Bộ ba `rzl`, `s0v`, `f72` |
| `f6y` / `f71` / `f6w` | code | Icon quan hệ sau Follow | Icon gợi ý bên cạnh trùng id nhưng tự bấm được |
| `rzs` | code | Nhãn thống kê (Following/Followers/Likes) | Kèm `rzt` (số) |
| `rv2` | code | "Add bio" | Chỉ đọc |
| `jsr` | 133 | Icon desc "Lock" trên tab "Private videos" của hồ sơ mình | Con trực tiếp của tab, khi chứng minh hồ sơ của mình |
| `cover` | 88 | Ô bài trong lưới hồ sơ | Chỉ trên hồ sơ và phải clickable |
| `tv_top` | 10 | Nhãn "Pinned" trên ô | Kiểm text; layout khác hiện "LIVE" |
| `u_u` / `mo3` / `b96` | 8 / 5 / 20 | Thẻ / lưới / caption của tab Videos trong tìm kiếm | Caption và tác giả phải nằm trong cặp thẻ/lưới |
| `pwe` | 2 | Panel vị trí (Cancel) | Chỉ trong hộp hỏi vị trí |
| `esv` | 10 | Thân hộp thoại chuẩn | Trong `visual_area` |

**Kiểm chứng bài đã đăng (photo viewer)**

| Id | Layout | Là gì | Scope trong Riviu |
|---|---|---|---|
| `zwj` | 2 | Thời gian đăng | Đúng một node; chưa có fixture thật (mục 7 của audit) |
| `desc` | 75 | Caption | Trong viewer đã đo |
| `rey` / `red` | 2 / 4 | Caption / nút Share của viewer | Cặp cho 45.7.3 |
| `widget_container` / `q9x` / `zm7` | 14 / 1 / 12 | Khung / bộ đếm ảnh / nhãn "Photo" | `q9x` nằm trong `widget_container` |
| `view_rootview` / `q_i` / `vgh` | 7 / 2 / 6 | Gốc / pager / ảnh của viewer mở rộng | Lồng đúng thứ tự |
| `ror` | 1 | "Privacy settings" | Màn khởi động lại xác minh |

Các id `aun`, `ta8`, `d2o`, `m3q`, `mjf`, `rjy`, `rpr`, `rqb`, `rrp` và `rs_` xuất hiện
trong arm lân cận của Trill 38.3.2 hoặc Global 46.x, **không** dùng cho 45.7.3. Id sound của
45.4.3 (`dmk`, `zy1`, `wf9`, `nd_`, `bgy`, `yq_`) trỏ tới view khác hẳn trong 45.7.3; ví
dụ `bgy` là avatar trong 27 layout.

## 4. Màn mở đầu sau khi cài mới

Riviu chỉ làm **một** thao tác từ chối cho mỗi màn, sau đó đọc lại. Nếu màn vẫn còn hoặc
gặp màn lạ thì dừng và báo, không bấm lại. Code nằm ở
[`app_automation/dialogs.rs`](../../../crates/core/src/app_automation/dialogs.rs);
hướng dẫn cho người vận hành ở [Thiết bị](../../operator/devices.md).

| Màn | Nhận diện | Thao tác duy nhất | Bằng chứng |
|---|---|---|---|
| Chọn chủ đề | `tk8` chứa `jg3` "Choose what you like" và `jfw` | `cqt` **Skip**; `cnt` "Next (0)" bị khóa cho tới khi chọn | Máy (vi); fixture en là bản **dịch lại** |
| Hướng dẫn vuốt | `tv_strengthen_swipe_up_guide` "Swipe up for more" trên trang `hpk` | Vuốt lên một lần; lớp này không có nút | Máy (vi); fixture en có feed thật |
| Kiểm tra bảo mật | Sheet `fl4` desc "Bottom sheet", `xw7`/`fkw` (chữ từ server) | Button không id, desc **Close**, trong `ovy`. Không bao giờ bấm `nri` Continue | Máy (vi); chữ tiếng Anh mượn từ Trill |
| Xin quyền thông báo | Hộp hệ thống `permission_allow_button`/`permission_deny_button` | **Don’t allow** | EMU (en) |
| Xin quyền danh bạ (Android 9) | Hộp hệ thống `com.google.android.packageinstaller`, `permission_message` "Allow TikTok to access your contacts?", cùng id/class với hộp vị trí đã đo | **Deny**. Banner "Find contacts" của TikTok (chuỗi 0x7f116f22) hiện phía trên; nếu còn sau Deny thì giao người vận hành | Ảnh màn hình #32/#34 (en), chưa có XML |
| Điều khoản dịch vụ | `visual_area` desc "Dialog", `esv` | Không bấm; chuyển cho người vận hành | EMU (en) |

Hộp thoại chuẩn có cấu trúc `visual_area` → `y5e`/`xwe` → `d93` > `tx3` > `esv`, cùng thanh
nút `x4` mà các nút trong đó không có id. Không bao giờ bấm Agree, Allow, Continue, Buy,
Follow hay Sync contacts.

Hộp "Link email" (tiêu đề `xwe`; APK: `X.0Nct`, `global_email_auth_dialog`) có thể hiện
đè For You bất cứ lúc nào, không chỉ sau cài mới (máy `ce071827715f321b05`, en). Thao tác
duy nhất là **Not now**; OK liên kết các địa chỉ email của máy vào tài khoản. Code:
[`dialogs/catalog.rs`](../../../crates/core/src/app_automation/dialogs/catalog.rs).

Sheet "Get updates sent to your email?" hiện trên feed ngay sau Post (máy
`ce031713f3d3ec1d0c` ngày 2026-10-12, `ce051715e15b2c2e02` ngày 2026-10-13, en). Sheet này
**không có No thanks** như bản trong APK: gốc `sjd`, tiêu đề `bob`, thân `bo8`, Button
không id **Get updates** (đăng ký nhận email, không bao giờ bấm) và nút X tròn `dyc`
(FrameLayout clickable, không desc; icon `dy6`). Riviu chỉ bấm `dyc`. Khi chưa đóng được,
sheet che feed nên bước quan sát sau Post không thấy feed và bài thành `uncertain`.
Fixture thật: `crates/core/fixtures/tiktok-publish/musically-45.7.3-en/real-20261012-email-updates-dialog.xml`.

Banner "Continue editing this post?" hiện đè lên đỉnh trang Profile sau khi phiên soạn bài
bị ngắt giữa chừng (TikTok bị tắt khi đang soạn) và còn đó qua nhiều ngày cho tới khi có
người bấm (máy `ce051715e15b2c2e02` / @lch.trnh.slay, en, 13–14/10/2026). Banner gồm
thumbnail nháp, nút **Save draft** và nút **Edit**. **Save draft** chỉ cất bài dở vào Drafts
(lưới hồ sơ có thêm ô "Drafts: 1"), không đăng gì. **Edit** mở lại trình soạn với nháp cũ, nên
không bao giờ bấm. Banner không có trong cây giao diện nào: nó nằm trong `bk2`, một
FrameLayout `[0,0][1080,625]` không có node con, và không thêm nút "Edit" nào vào cây. Khi đọc
qua phiên UiAutomator2 dài hạn của Riviu, mọi node đầu trang nằm trọn trong vùng đó cũng biến
mất: ảnh đại diện `bf1`, tên `rz5`, handle `s0v`, nút Edit của hồ sơ và thanh `ov8` (Profile
menu, Share, Profile views). Các node kéo xuống dưới y=625 vẫn còn: hàng số liệu
`rzt`/`rzs`, bio, dải tab và lưới. Vì không đọc được handle, bước chứng minh tài khoản trước
Đăng từng dừng với `account_unreadable`. `uiautomator dump` thường (không có phiên Riviu) vẫn
thấy đầu trang. `bk2` vẫn còn, rỗng, sau khi banner đã tắt. Fixture thật:
`crates/core/fixtures/tiktok-publish/musically-45.7.3-en/real-20261014-draft-banner-session.xml`
(có banner, đọc qua phiên Riviu) và `real-20261014-draft-banner-saved.xml` (sau Save draft).

Bước chứng minh tài khoản ([`tiktok_account.rs`](../../../crates/core/src/tiktok_account.rs),
nhận diện ở `draft_banner_shape` trong `dialogs.rs`) tự xử lý banner này:

1. **Cây chỉ là lý do để nhìn.** Tab `nrb` "Profile" đang được chọn, có đúng một tab
   "Private videos", `bk2` bắt đầu ở y=0, cao từ 500 px, không có con, và không có
   `s0v`/`rz5`/`ov8`. Hình dạng này không bao giờ đủ để bấm.
2. **Xác nhận bằng ảnh.** Riviu chụp một ảnh 1080x2220, chạy OCR trong vùng `bk2`, và cần
   thấy "Continue editing this post?" cùng đúng một "Save draft" nằm ở nửa trái `bk2`
   (Edit nằm ở nửa phải).
3. **Bấm một lần.** Riviu bấm tâm khung chữ "Save draft" qua shell, như bảng nhạc. Trên
   máy 37 điểm này là (409, 244). Sau đó Riviu chứng minh lại tài khoản trên hai lần đọc
   mới; mọi quan sát trước lúc bấm đều bỏ.
4. **Không bấm lần thứ hai.** Nếu banner xuất hiện lại trong cùng lượt, bước này dừng với
   `account_draft_banner_persisted`. Nếu cây có hình dạng banner mà ảnh hoặc OCR không xác
   nhận được, Riviu không bấm gì và dừng với `account_draft_banner_unconfirmed`. Cả hai lỗi
   đều xảy ra trước Đăng và đều được thử lại tự động.

Hộp "Add to Home screen?" thấy trên máy 40/43/48 ngày 2026-10-10 do chính TikTok gây ra:
`com.ss.android.ugc.aweme.shortcut.TiktokShortcutManager` (dịch vụ `IShortcutService`) gọi
`ShortcutManager.requestPinShortcut`, rồi launcher Samsung hiện hộp xác nhận. Hộp này thuộc
launcher, không thuộc TikTok, và không ảnh hưởng tới tài khoản. Hiện chưa có classifier cho
hộp này và cũng chưa có bản dump XML. Thao tác hợp lý là **Cancel**, vì Add chỉ thêm biểu
tượng ra màn hình chính. Khi có bản dump thì bổ sung vào catalog.

## 5. Sound picker

Đo trên máy #44, giao diện en, ngày 2026-10-09:

- **Picker là một cửa sổ riêng.** Sheet (`fl4`, desc "Bottom sheet") có window token
  riêng, dù mang cùng tên activity với editor. Gốc của cửa sổ đang active chỉ chứa sheet,
  không có `tv_top_text` hay node nào của editor. Client chờ gốc của editor, hoặc duyệt
  mọi cửa sổ, sẽ thấy tập cửa sổ khác với client đọc cửa sổ active.
- **Hàng sound:** `opr` là khung bấm được, bọc `vertical_item_music_new_rl`. Số thứ hạng là
  `zyn`. `nms` chỉ có mặt khi hàng đó đang phát và mất khi tắt.
- **Chạm lại hàng đang phát sẽ bỏ chọn sound.** Chip sound bị gạch chéo. Vì vậy không có
  bước "tạm dừng" nào an toàn.
- **Không có content-desc "Pause sound"/"Play sound"** trên picker ở build này. Hai chuỗi
  đó có trong bảng nhưng không hiện ra.
- **Picker có thể mở ở tab For You chứ không phải Hot**, nên Riviu vẫn phải chạm `wrv`
  "Hot". **Quy tắc chủ sản phẩm (2026-10-12, "Bắt buộc Hot"):** nhạc của bài đăng chỉ lấy
  từ tab Hot. Hàng For You không bao giờ là pool. Không chứng minh được Hot thì dừng trước
  Đăng. Xem [contracts.md](../../development/contracts.md), đoạn "Nguồn của tên nhạc kỳ
  vọng".
- **Tab Hot có thể có hàng chip vùng** "United States / Vietnam / Global" phía trên danh
  sách, đẩy hàng đầu từ y≈1370 xuống y≈1686 trên khung 1080×2220 (máy 44
  `ce031713d903001b01`, 10/10/2026). Không giả định vị trí hàng cố định.
- **Tab Hot là bảng xếp hạng nhạc theo vùng (APK, 2026-10-12).**
  - Trang của mỗi chip là `…music.vertical.tab.hot.HotMusicRegionPage`. Nó tải qua
    `MusicChartRepo` → `MusicChartApi.getMusicList`, tức GET `/tiktok/music/chart/v1/`
    với `chart_region=<mã vùng>`, `scene=2`, `source=creation_music_panel` và trang 30 hàng.
    Request không tự thêm tham số riêng theo tài khoản; riêng `use_past_emf_music_id` giữ
    lại một hàng quảng bá đã ghim. Lớp mạng vẫn gắn tham số chung của thiết bị và tài khoản,
    nên chưa loại trừ được việc server cá nhân hóa. Ứng viên: hai máy cùng chip vùng thấy
    cùng danh sách. **Chưa đo.**
  - Ba chip theo thứ tự:
    1. vùng của SIM, nếu không có thì vùng của locale máy;
    2. `tv_user_data_region`: `store_region` của tài khoản, ẩn khi trùng chip 1;
    3. "Global", hoặc vùng theo IP khi cờ server `music_panel_global_use_ip_region` bật.
  - Chip mặc định là chip **đầu tiên**. Chip đang chọn chỉ khác màu nền/màu chữ;
    `selected` luôn là false (dump `nav_picker_hot.xml`, máy #44). Vì vậy XML không cho
    biết danh sách đang hiện thuộc vùng nào. Trên máy #44, chip 1 là "United States", nên
    "Hot" mặc định ở đó là bảng của Mỹ, không phải Việt Nam.
  - Danh sách "hot" của trang chọn nhạc toàn màn hình cũ (`//choosemusic/home` →
    `ChooseMusicActivity`) dùng endpoint khác (`/aweme/v1/hot/music/`) và mang tiêu đề
    "For You". Đó không phải tab Hot.
- **Hàng đã chọn có khung đỏ 4 px quanh thumbnail** (x 110..234; ảnh chưa chọn chỉ
  ~115..230). Đây là bằng chứng ảnh của hàng đã chọn, kể cả khi OCR không đọc được tiêu
  đề đỏ. Hằng số đo nằm ở
  [`tiktok_sound/visual.rs`](../../../crates/core/src/tiktok_sound/visual.rs).
- Một lượt `uiautomator dump` mới đọc được picker trong khoảng 2,6 s ở 28/28 lần, dù preview
  đang phát hay đã dừng, và dù animation scale là 1 hay 0. Main thread vẫn phản hồi (khoảng
  0,2 s). Như vậy lỗi treo khoảng 10,5 s nằm ở đường đọc của session UiAutomator2 sống lâu,
  không ở trạng thái của picker. Đây là suy luận từ việc không tái hiện được, chưa phải phép
  đo đường helper.
- TikTok tự gắn một sound gợi ý khi vào editor. Đã thấy ở 3/3 editor đo được: `nav_editor.xml`
  trên #44, A2 và A9b trên #46 (2026-10-09). Sheet không tự mở; nó chỉ mở khi chạm `dou`.
  Đây là gợi ý kiểu For You, không phải Hot, nên Riviu không giữ sound này làm nhạc của bài
  (quy tắc "Bắt buộc Hot" ở trên). Riviu chọn một hàng Hot thay thế, rồi chứng minh chip editor
  mang đúng tên hàng đó.
- Xác định foreground bằng `mCurrentFocus` hoặc mục `ACTIVITY` của TikTok. Dòng `ACTIVITY`
  đầu tiên của `dumpsys activity top` có thể là app khác.

## 6. Share intent và deep link

Probe trên máy #46, en, ngày 2026-10-09. Theo quyết định của chủ dự án, Riviu mặc định chạm
qua UI như người dùng. Deep link chỉ được dùng ở chỗ có ích và đã được duyệt, ví dụ mở bài
đích theo id thay cho Copy link. Muốn thêm lối tắt bỏ qua cả một luồng màn hình thì hỏi trước.

| Đường vào | Kết quả |
|---|---|
| `snssdk1233://aweme/detail/<id>` | Máy #46: mở DetailActivity đúng bài. Máy #32/#34 (tài khoản fleet, 2026-10-09): mở bài trong trình xem kiểu feed (tab Home, thanh Not interested/Interested), không có `bj1` |
| `snssdk1233://music/detail/<id>` | Mở MusicDetailActivity ngay lần đầu |
| `snssdk1233://user/profile/<uid>` | Mở hồ sơ (TikTokHostActivity) |
| `snssdk1233://setting` | Mở "Settings and privacy" |
| `snssdk1233://<route lạ>` | **Nhận lặng lẽ**: không có gì mới hiện ra, màn cũ giữ nguyên, `am start -W` treo |
| `https://www.tiktok.com/@<handle>` | Mở hồ sơ |
| `https://www.tiktok.com/@<handle>/video/<id>`, `/photo/<id>`, link rút gọn `vt.tiktok.com` | Chèn bài vào đầu feed For You (MainActivity), không mở trang chi tiết |
| `https://www.tiktok.com/music/<slug>-<id>` | Mở trang nhạc, có lần đầu báo "Your network is unstable" |
| Đường dẫn https lạ | Android từ chối (`unable to resolve Intent`) |
| `SEND` / `SEND_MULTIPLE` ảnh tới `SystemShareActivity` | Sheet "Share on TikTok" với **Photo** / **Message**; Photo mở đúng photo editor thường, ảnh theo thứ tự URI. `am` không tạo được danh sách Uri |

Hệ quả cho code:

- **Copy link làm tăng bộ đếm share.** Bộ đếm của bài đọc 0 ngay sau Copy link và 1 share
  khoảng một phút sau. Ngay bên trái Copy link là **Repost**. Mở bài đích theo id; share
  sheet chỉ là đường dự phòng một lần khi trang chi tiết không được chứng minh, và
  evidence ghi `copyLinkCountsAsShare`.
- **Copy link có thể trả link rút gọn `https://www.tt.site/t/<code>/`** (đo 13/10/2026 trên
  ce031713b0, ce04171415, ce11171beb), bên cạnh `vt.tiktok.com`. Link này 301 về
  `www.tiktok.com/@handle/photo/<id>`. Mọi nơi nhận link rút gọn dùng chung
  `tiktok_share::is_short_link_host`; chỉ đích `tiktok.com` sau redirect mới là proof.
- **Biến thể trình xem feed chưa giải thích được.** Bảng router của APK
  (`SmartRouter$$Mapping`) ánh xạ `//aweme/detail` và `//aweme/detaillist` thẳng tới
  `DetailActivity`; research không tìm thấy cờ thí nghiệm hay interceptor nào chuyển route
  này sang feed. Giả thuyết còn mở: cấu hình phía server theo tài khoản. Không coi thẻ feed
  là proof chỉ vì id.
- Sau mỗi deep link phải kiểm activity đích và nội dung màn hình trước khi làm gì, vì route
  sai không báo lỗi. Dùng deadline hữu hạn và không dùng `-W`.
- **Message** trên share sheet và **Send to friends**/**Save draft** trong menu Discard là
  effect. Luôn tìm Discard theo text, vì vị trí Y của nó thay đổi.
- Các activity chồng lên nhau trong cùng một task TikTok. Back từ Home có thể rơi vào một
  trang cũ.

### Lấy link bài vừa đăng bằng tay (đo 14/10/2026, máy 36)

Máy 36 (`ce04171435f104080c`, SM-G955F, @dongchoinoinoi_dalat), campaign r115 `21c2c7b5`, bài
đã gửi khoảng 10 phút, job soạn đã hoãn lấy link cho bộ xác minh. Làm qua controller
(Inspector + `device_tap`, cùng lease), ảnh và XML từng bước ở
`target/apply-apk-20261009/linkflow-p36-20261014/` (ngoài Git).

| Bước | Màn hình / thao tác | Phần tử đo được |
|---|---|---|
| 0 | Ngay sau Post TikTok mở feed **Friends** đang phát **chính bài vừa đăng**: tên tài khoản, "Photo · 2s ago", đúng caption, nút quảng bá "Spend … get … views" (chỉ có trên bài của mình) | `title` = tên hiển thị, `desc` = caption, Share `content-desc="Share video.  shares"` |
| 1 | Bấm tab Profile | `nrb` `content-desc="Profile"`; kết quả: `s0v` "@handle" |
| 2 | Bấm Profile lần nữa để tải lại; lưới trả về bài mới ở ô đầu (3 lượt xem) | ô lưới `emr`/`cover` **không có selector duy nhất** (mọi ô giống nhau) |
| 3 | Mở ô đầu: `device_tap` (179, 1114) trên 1080x2220 | trình xem: `title`, `zm7` "Photo", `zwj` "· 10m ago", `desc` caption, `bj1` Back |
| 4 | Bấm Share | `content-desc="Share video.  shares"` (Button); sheet có **Edit post / Pin / Privacy settings** = bài của mình |
| 5 | Ghi sentinel vào clipboard (`device_set_clipboard`), bấm **Copy link** | `content-desc="Copy link"` (nhãn `ut0`); sheet đóng, trình xem trở lại |
| 6 | Đọc clipboard (`device_get_clipboard`) | `https://vt.tiktok.com/ZSgJBTM6A/` (khác sentinel) |
| 7 | Redirect HTTPS 301 | `https://www.tiktok.com/@dongchoinoinoi_dalat/photo/7696402901803158805` |
| 8 | oEmbed (`/oembed?url=…/video/<id>`) | `author_unique_id` khớp, `title` = caption, `embed_product_id` = id |

- Bước 4–5 đã lưu thành Flow **"Copy link bài đang mở (TikTok 45.7.3 en)"** (Inspector →
  Lưu thành Flow). Lúc đo, bước 1–3 không lưu được thành Flow (ô lưới không có selector,
  tải lại Profile không làm hiện phần tử mới). Inspector bản sau thêm "ô thứ N trong lưới",
  kết quả "màn hình giữ nguyên" có lựa chọn rõ, bước vuốt và nút Flow "Đọc clipboard"; ghi
  trọn luồng bằng UI trên máy thật chưa thử.
- Bảng Chia sẻ có hai dạng, cùng `fl4` "Bottom sheet" > header `ti2` (`tv_title` "Send to",
  `lpe` Search, `l86` Close). Dòng Copy link `dka` chỉ có ở dạng đầy đủ (bài của mình mở từ
  Profile). Dạng "Send to" (máy 44, Share trên feed) chỉ có Cast/Pin/Create group/Captions/
  Playback speed/Privacy settings: đóng bằng `l86`, không bấm ô nào khác.
- Feed sau Post đang phát đúng bài của mình (tên, "Xs ago", caption, nút quảng bá). Đây là
  đường ngắn hơn đi qua Profile nếu code chứng minh đủ các dấu hiệu đó trước khi bấm Share;
  chưa dùng trong code (`execution.rs` vẫn đi Profile vì Share trên feed có thể thuộc bài
  người khác).

### Đường nhạc qua link: music/detail → Use sound

Đo trên máy #46 (en, 2026-10-09) bằng các dump của probe share/deep link: trang nhạc
(B3c, B1f, B1a, B1o), recorder (B1g), picker (B1h, B1i), editor (B1j), thoát (B1k–B1o).
Editor được so với `nav_editor.xml` của probe sound picker (#44). Dòng ghi **APK** là đọc
tĩnh manifest và bảng chuỗi `default` của 45.7.3, nên chỉ là ứng viên.

- **Định tuyến (APK).**
  - `DeepLinkActivityV2` nhận VIEW + BROWSABLE cho scheme `snssdk1233` (và bảy scheme
    khác) mà không khai host hay path, nên route nào cũng được nhận lặng lẽ.
  - `com.ss.android.ugc.aweme.music.ui.MusicDetailActivity` không export. Chỉ tới được
    trang này qua SmartRouter `//music/detail`.
  - Link https `/music/…` đi qua `AppLinkHandlerV2`.
- **Các route nhạc khác trong bảng router (APK, 2026-10-12).** Không route nào mở một
  trang native liệt kê đúng bảng của tab Hot:
  - `//choosemusic/home` → `ChooseMusicActivity`: trang chọn nhạc cũ của recorder. Nó trả
    kết quả về nơi gọi.
  - `//assmusic/category` → `MusicDetailListActivity`: một playlist theo `mc_id`.
  - `//music/dsp/*`, `//music/library/*`, `//music/player/*`: TikTok Music và thư viện
    nhạc của tôi.
  - `//profile/search_music`, `//search`: tìm theo từ khóa.
  - Bảng xếp hạng nhạc trong app là trang Spark/Lynx
    `aweme://roma_redirect/?spark_page=music_charts…`, lấy từ cờ server `music_chart_url`.
    Huy hiệu hạng trên trang nhạc (`MusicChartRankStruct.desc`) mở trang này; huy hiệu bị
    ẩn với tài khoản doanh nghiệp. Bundle Lynx tải lúc chạy nên APK không cho biết
    endpoint. Trang có đọc được bằng uiautomator không: **Chưa đo**.
- **Trang nhạc (Máy).**
  - Với sound gốc có chứa một bài nhạc, `title` ghi "Contains: <tên bài>". Ô tìm kiếm trên
    cùng `tv_search_sug_word` ghi đúng "<tên bài>".
  - `b9_` "Original sound by: " đi cùng `b96` (tác giả). `used_count` ghi "‎42K posts" và
    mở đầu bằng ký tự U+200E.
  - Chữ **Use sound** `vu_` không clickable. Khung bấm là cha của nó, `vu2`
    `[535,1890][1054,2052]`, ở nửa phải.
  - Ngay bên trái là **Add to Story**: `tv_share_btn` nằm trong khung bấm `uvw`
    `[26,1889][545,2052]`. Đây là effect Story, không bao giờ bấm. Cũng không bấm
    "Share sound" (Button không id trong `ovy`) và "Save".
  - Link https có thể hiện trang lỗi mạng ở lần mở đầu: `xwe` "Your network is unstable.
    Tap to retry.", `message_tv` "Unstable network. Please try again.", nút `d1z` "Retry".
- **Recorder sau Use sound (Máy)** chính là recorder của luồng `Create`.
  - Có `sn3` (@2131823130) và lối vào thư viện `upload_hot_area` `[0,1892][179,2071]`. Bên
    trong là `cry`, nhưng `cry` không clickable.
  - Thanh chế độ `wru` đang chọn **15s**.
  - Chip `dou` nằm ở `[278,134][703,250]`:
    - `tv_top_text` ghi đủ "<tên bài>"; dấu "…" chỉ là phần vẽ;
    - `dos` mang desc "Sound: <tên sound gốc> by <tác giả>";
    - `doi` (@2131823053) là Remove sound.
- **Picker (Máy)** dùng cùng id với luồng thường: `tv_title` "Recents", `k_x`, `wjp`
  "Next (3)". Sau khi chọn ảnh, ô Select multiple nhường chỗ cho `crb` "AutoCut", cạnh đó
  có `mbe` "Use layout". Không bấm hai nút này.
- **Editor (Máy).** Tập id giống editor của luồng thường, chỉ thiếu `lj2`. Sound đặt từ
  Use sound **được giữ**; TikTok không thay bằng sound gợi ý. Các khác biệt đã đo:
  - Chip nằm ở y 134–250 thay vì 100–216, tức thấp hơn 34 px. Menu Discard thấp hơn 32 px
    (`[121,291]` so với `[121,259]`). Vùng OCR chip trong
    [`selection_recovery.rs`](../../../crates/core/src/tiktok_sound/selection_recovery.rs)
    được đo trên editor thường, nên phải đo lại trước khi dùng cho đường này.
  - `dou`/`dos` là Button trên recorder nhưng là LinearLayout trên editor; selector không
    được dựa vào class. `dos` không có desc trên editor.
- **Thoát (Máy).**
  - Back trên editor mở menu Discard.
  - Discard trở về picker, vẫn giữ 1/2/3 và "Next (3)", nằm trên recorder vẫn còn sound.
  - Back không đóng recorder. `kbm` (@2131823218, Close) đưa về trang nhạc.
- **Tên dùng để so (Máy).**
  - Chữ trên chip khớp `tv_search_sug_word`, và khớp `title` sau khi bỏ "Contains: ".
  - Nó không khớp desc của `dos`, cũng không khớp tên sound gốc.
  - Điều này mới đo trên một sound gốc có chứa bài nhạc.
- **Chưa đo:**
  - sound là bài nhạc thường;
  - đăng một ảnh;
  - video: chế độ 15s có cắt clip không? Bảng `default` có chuỗi "This sound isn’t
    available for videos over 60 seconds";
  - id không tồn tại hoặc bị chặn theo vùng;
  - tài khoản doanh nghiệp (chuỗi "This sound isn’t licensed for commercial use.");
  - màn hình sau khi bấm Post trên đường này.

## 7. Đăng nhập và đăng xuất

Bảng dựa trên dump máy #31 (vi, cài mới) và EMU (en, khách). Handle ghi là `<username>`,
tên hiển thị là `<display name>`. Mật khẩu không bao giờ được ghi vào file hay log.

| Màn | Nhận diện | Hành động | Ghi chú |
|---|---|---|---|
| Chọn cách đăng nhập (Compose) | Tiêu đề không id "Log in to TikTok"; gốc `ifa` > `tk8` > `v6m` | `View` desc "Use phone / email / username" | `y76` đổi nghĩa theo lối vào: **Close** trên máy, "Report a problem" trên EMU. Nhận theo desc, không theo id |
| Form đăng nhập | Tab `q60` > `emr` > `android:id/text1` ("Phone", "Email / Username"), pager `q5z` | Tab Email/Username, gõ vào EditText **không id** trong `gtm` > `gu_` | Tiêu đề là `ovm` "Log in" trên máy, `q62` trên EMU: chấp nhận cả hai |
| Lưu đăng nhập | `CheckBox pbt` (mặc định đã tick), `pbu` | Giữ nguyên | |
| Tiếp tục | `Button f79` "Continue" | Bấm **một lần** | Bị khóa khi ô trống |
| Bước mật khẩu | `bpr` > `esq` "Enter password"; EditText không id, `password=true`, trong `kky` > `px4` | Gõ mật khẩu, `f79` một lần | Có `kkz` "Need help logging in?", không phải "Forgot password?" |
| Bảng bảo mật sau đăng nhập | Như mục 4 | **Close** | Không bấm `nri` |
| Hồ sơ của mình | `nrb` được chọn, `Button s0v` "@`<username>`", `rz5` `<display name>` | Đọc `s0v` để chứng minh tài khoản | Tên hiển thị không chứng minh tài khoản |
| Lối vào đăng xuất | Button không id, desc "Profile menu", góc phải trên hồ sơ | Chưa đo | |
| Đăng xuất | "Settings and privacy" → "Log out" → hộp xác nhận "Are you sure you want to log out?" (hai biến thể) | Chưa đo | Có thể có hỏi "Save login info" |
| Tài khoản đã lưu | Layout `xe`, danh sách `o5u` | Chưa đo | |

OTP, captcha, sai mật khẩu, "too many attempts" hay trang "verify it's you" đều làm dừng máy
đó và báo cho người vận hành, không thử lại. Chưa có dump tiếng Anh thật của máy cho bất kỳ màn
nào ở bảng này.

## 8. Emulator

- AVD Medium_Phone, Android 17, x86_64 dịch arm64, WHPX. Cài mất khoảng 7 phút.
- Tới được: màn đăng ký, màn chọn cách đăng nhập, form điện thoại và email, hộp đồng ý
  Điều khoản, hộp xin quyền thông báo.
- **Không tới được feed.** TikTok crash hai lần trong bộ dịch ARM (`reg_alloc.cc:113 CHECK
  failed`). Đây là lỗi của emulator, không phải TikTok phát hiện emulator.
- Kết luận: emulator chỉ dùng cho màn đăng nhập. Feed, hồ sơ và composer phải đo trên máy
  thật. Không gõ credential vào emulator.

## 9. Mười lăm mục của audit

Trạng thái tính tại commit `2a4190b`. Khi sửa một mục, cập nhật dòng của mục đó.
"Cần đọc máy" nghĩa là phải có một lần đọc thật trên 45.7.3/en trước khi code dựa vào.

| # | Mục | Trạng thái |
|---|---|---|
| 1 | Guard khởi động lại (`verification_restart.rs`) chỉ từ chối `uploading`/`processing`/`posting`, nên lọt toast "Post is being processed" | Mở; không cần đọc máy |
| 2 | Không đọc toast kết quả sau Post ("Couldn't post. Draft saved.", giới hạn tần suất, "Video posted!"); chỉ ghi làm bằng chứng, không retry | Mở; cần đọc máy |
| 3 | `account_blocker` không nhận màn chọn đăng nhập Compose (không có `:id/title`, lựa chọn là `View`) | Mở; cần đọc máy (en) |
| 4 | Fixture màn mở đầu tiếng Anh là bản dịch, không phải dump thật | Mở; cần đọc máy |
| 5 | Kiểm desc đúng bằng "Comments" chưa từng thấy trên 45.7.3 | Mở; cần đọc máy (photo viewer mở rộng) |
| 6 | Kiểm "Back" trên toàn cây bỏ sót `bix` `@2131823215` (`photo_proof.rs`, `tiktok_share/verification.rs`) | Mở; không cần đọc máy |
| 7 | `zwj` (giờ đăng) cùng `q9x`, `k7u`, `u_u`/`mo3`/`b96`, `tv_top` chưa có fixture thật đã commit | Mở; dump đã có |
| 8 | `sound_link` chỉ khớp "Sound:", bỏ sót "Original sound by …" | Mở; cần đọc máy |
| 9 | Rehearsal tương tác so đúng chữ "Uploading"/"Posting", trong khi 45.7.3 có "…" ở cuối | Mở; không cần đọc máy |
| 10 | Không đọc chữ kết quả gửi bình luận ("Comment posted", "Couldn’t post comment") và "Requested" | Mở; cần đọc máy |
| 11 | Một số match `contains` lỏng hơn định dạng của TikTok (" profile", "Follow ", "comments") | Mở; không cần đọc máy |
| 12 | "Removed for violating Community Guidelines" và "Shared successfully" không có trong bảng lẫn dump; hộp vị trí ghi "at any time" | Cần đọc máy |
| 13 | `resolve_desc.py --scan` chỉ đọc tag `<node>` | **Đã sửa**: đọc cả tag theo tên class |
| 14 | Comment test trong `tiktok_drawer.rs` gọi `@2131823091` là "Send" của DM, thật ra là "Reply with a video" | Mở; không cần đọc máy |
| 15 | REPORT §6 sai: `y76` là Close trên máy, tiêu đề là `ovm` chứ không phải `q62`, bước mật khẩu có `kkz` | **Đã sửa** trong mục 7; bản en cần đọc máy |

## 10. Ngôn ngữ ứng dụng và ngôn ngữ máy

Riviu đọc ngôn ngữ từ cài đặt Android (`am get-config`), không đọc ngôn ngữ riêng của
TikTok. Máy Riviu 31 (09/10/2026) cho thấy bản cài mới 45.7.3 theo ngôn ngữ hệ thống. Nếu
người dùng đã đặt **Ngôn ngữ ứng dụng** trong TikTok thì TikTok giữ ngôn ngữ đó dù hệ thống
là English; trường hợp này **chưa đo** trên máy.

Nhãn đường đổi tay trong TikTok, lấy từ bảng chuỗi APK bằng `find_string.py --locale vi-VN`
(12/10/2026). Đây là **ứng viên tĩnh**: chưa đo trên máy, không dùng để tap.

| Bước | en (`default`) | vi-VN | String id |
|---|---|---|---|
| Menu hồ sơ → | Settings and privacy | Cài đặt và quyền riêng tư | 11 id, ví dụ `0x7f1108d8` (nhiều id cùng nhãn, nhãn không chọn được id) |
| | Language | Ngôn ngữ | `0x7f113da2` (còn `0x7f111e74`, `0x7f119059` là "Languages") |
| | App language | Ngôn ngữ ứng dụng | `0x7f11170a`, `0x7f1168b4` |
| Lựa chọn | English | Tiếng Anh | `0x7f113496`, `0x7f119037` (danh sách ngôn ngữ có thể hiện tên bản ngữ "English"; chưa đo) |

Deep link `snssdk1233://setting` mở "Settings and privacy" (§6). Riviu không dùng đường
này để đổi ngôn ngữ: lựa chọn của owner 12/10/2026 là đổi ngôn ngữ hệ thống qua Riviu
Helper 0.8.0 (`systemLocale`), không chạm màn TikTok.
