## 11. Build TikTok mới (đọc APK, đối chiếu id)

TikTok làm rối tên view id và đổi chúng ở mỗi build: `dou` (45.7.3) thành `dwh` (46.4.3).
Id của chuỗi cũng đổi theo build, kéo theo token `@<số>` trong content-desc. Vì vậy khi
TikTok bắt cập nhật, không được chép bảng selector từ build cũ sang build mới. Bộ công cụ
[`tools/tiktok-apk/`](../../tools/tiktok-apk/) dùng Python 3.12 thuần stdlib. Nó chỉ đọc
APK gốc và đưa ra **ứng viên** cho người duyệt; ứng viên không phải bằng chứng.

### Ràng buộc

- Chỉ đọc APK gốc lấy từ máy. Không sửa, ký lại hay cài bất kỳ APK nào đã bị sửa.
- APK, dump, bảng chuỗi, index và mã decompile (jadx) chỉ nằm trong
  `target/tiktok-<version>-research/`, thư mục bị git-ignore. Không đưa chúng vào repo.
  Code và fixture chỉ được chứa id, chuỗi UI ngắn và tên class ngắn.
- Giữ lại thư mục research của build cũ để còn index mà so.
- Không đổi bảng nào nếu chưa đo trên **một** máy thật. Bảng mới phải ghi provenance
  gồm máy, ngày, versionCode và ngôn ngữ. Ứng viên `ambiguous`, `low` hay `none` không bao
  giờ được dùng làm selector; match cũ hoặc mơ hồ không cho phép tap.

### Quy trình

1. **Lấy APK.** Dùng một máy canary không bị controller giữ lease và không có job đang
   chạy. Chỉ chạy lệnh đọc, với adb server đang chạy; không kill hay restart server.

   ```powershell
   adb -s <serial> shell pm path com.zhiliaoapp.musically
   adb -s <serial> pull <đường-dẫn-base.apk> target\tiktok-<version>-research\base.apk
   ```

   Nếu `pm path` trả về nhiều split, kéo về tất cả. Các công cụ bên dưới chỉ đọc `base.apk`.
2. **Kiểm danh tính.** Lệnh sau trả mã 1 khi chữ ký không hợp lệ, signer khác
   `KNOWN_SIGNERS` hoặc version không như mong đợi. Gặp signer lạ thì dừng.

   ```powershell
   $T = "target\tiktok-<version>-research"; $SDK = "$env:LOCALAPPDATA\Android\Sdk"
   py -3.12 -I tools/tiktok-apk/verify_apk.py --sdk $SDK "$T\base.apk" --expect-version <version> --json-out "$T\identity.json"
   ```
3. **Giải bảng chuỗi.** NxString gồm bảng `default` (tiếng Anh) và bảng ghi đè theo
   locale. Máy Riviu thường dùng `vi-VN`, nhưng vẫn phải ghi lại locale thật của máy.

   ```powershell
   py -3.12 -I tools/tiktok-apk/nxstring.py --apk "$T\base.apk" --list-locales
   py -3.12 -I tools/tiktok-apk/nxstring.py --apk "$T\base.apk" --locale vi-VN --out "$T\nxstring_vi-VN.tsv"
   ```
4. **Lập index layout** cho build mới. Lệnh này tự tạo `aapt2-resources.txt` nếu file chưa
   có. Với 45.7.3 nó đọc 11 969 layout trong khoảng 8 giây.

   ```powershell
   py -3.12 -I tools/tiktok-apk/layout_index.py --apk "$T\base.apk" --sdk $SDK --resources-dump "$T\aapt2-resources.txt" --out "$T\layout_index.jsonl"
   ```
5. **Liệt kê id Riviu đang dùng.** Quét trên index của build cũ, rồi quét lại trên build mới.
   Kết quả phân loại: `unique`; `ambiguous` (khai báo ở nhiều layout, nên phải scope theo
   ancestor hoặc text); `code-only` (gán từ code, không có trong layout); `missing` (build
   không có id này). Id viết dạng chuỗi trần như `"s0v"` thì scan không thấy, phải thêm bằng
   `--id`. Cột `near_version` chỉ là ước lượng theo khoảng cách dòng.

   ```powershell
   $OLD = "target\tiktok-<cũ>-research"
   py -3.12 -I tools/tiktok-apk/riviu_ids.py --index "$OLD\layout_index.jsonl" --resources-dump "$OLD\aapt2-resources.txt" --version <cũ> --id s0v --out "$T\ids_old.tsv"
   ```
6. **So hai build.** Công cụ xếp hạng ứng viên theo layout, vị trí trong cây, chuỗi gắn kèm
   và hình dạng cục bộ. Mức tin cậy là `high`, `medium`, `low`, `ambiguous` hoặc `none`.

   ```powershell
   py -3.12 -I tools/tiktok-apk/diff_ids.py --old "$OLD\layout_index.jsonl" --new "$T\layout_index.jsonl" --ids-file "$T\ids_old.tsv" --out "$T\candidates.tsv"
   ```
7. **Giải token `@<số>`** lấy từ dump uiautomator của build mới (`--scan` nhận file hoặc thư mục). Cột locale để trống nghĩa là máy sẽ
   hiện tiếng Anh. Lệnh trả mã 1 khi có token không thuộc build này.

   ```powershell
   py -3.12 -I tools/tiktok-apk/resolve_desc.py --apk "$T\base.apk" --locale vi-VN --scan "$T\dumps"
   ```
8. **Duyệt ứng viên.** Ghi kết luận vào `$T`, không ghi vào docs. `high` chỉ có nghĩa là
   "đo id này trước". Id `code-only`, chẳng hạn `s0v` hoặc `nr_`/`nrb`, không có bằng chứng
   layout nên phải tìm bằng dump runtime. Id chung như `title`, `tv_title`, `desc` hay `bix`
   luôn cần scope. Muốn biết một nhãn thấy trên màn hình (tiếng Anh hoặc tiếng Việt) ứng
   với string id nào và layout nào dùng id đó thì chạy lệnh dưới. Một nhãn thường có nhiều
   id; cột `layouts` để trống nghĩa là chuỗi được gán từ code.

   ```powershell
   py -3.12 -I tools/tiktok-apk/find_string.py --apk "$T\base.apk" --locale vi-VN --index "$T\layout_index.jsonl" "Add sound" "Thêm âm thanh"
   ```

   Chỉ khi cần đọc code thì mới decompile, và chỉ decompile dex chứa hằng
   số: `dex_lookup.py const <apk> 0x7f0a....`, sau đó `dex_lookup.py classdef`.
9. **Đo trên một máy thật**, theo từng trường và từng ngôn ngữ đang hỗ trợ. Thêm fixture
   đã sanitize (không chứa secret hay tài khoản thật), thêm negative case, rồi mới sửa bảng
   `tiktok_*` theo [quy trình thiết bị mới](10-thiet-bi-moi.md). Phần còn lại của fleet
   đi qua gate thông thường.

Test của bộ công cụ chạy trên dữ liệu tổng hợp, không dùng dữ liệu TikTok:
`py -3.12 -m unittest discover -s tools/tiktok-apk -p "test_*.py"`.
