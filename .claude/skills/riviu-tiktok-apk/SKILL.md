---
name: riviu-tiktok-apk
description: >
  Dùng khi TikTok bắt cập nhật build mới, khi selector/resource id/content-desc
  của Riviu ngừng khớp sau một bản TikTok, hoặc khi cần biết UI của một build có
  gì (layout, id, chuỗi theo ngôn ngữ, token @<số>). Phân tích chỉ đọc APK gốc kéo
  từ máy bằng tools/tiktok-apk theo runbook build mới; ứng viên không phải bằng
  chứng. Không sửa, ký lại hay cài APK đã sửa, không đưa mã decompile vào repo,
  không dùng deep-link/intent để bỏ qua màn hình.
metadata:
  version: "1.0.0"
  origin: local
  project: riviu-manager
---

# Đọc APK TikTok cho Riviu

Skill tự soạn cho Riviu. Nó giúp tìm **ứng viên** id/chuỗi của một build TikTok từ APK gốc để người duyệt đem đi đo trên máy thật. Nó không phải bộ reverse tổng quát và không thay phép đo trên thiết bị.

- Runbook gốc, giữ lệnh đầy đủ: [Build TikTok mới](../../../docs/agents/11-build-tiktok-moi.md). Skill này chỉ tóm tắt; khi lệnh hai nơi khác nhau thì runbook đúng.
- Công cụ: [`tools/tiktok-apk/`](../../../tools/tiktok-apk/), Python 3.12 thuần stdlib, chạy bằng `py -3.12 -I`.
- Sửa bảng `tiktok_*` sau khi đo: [quy trình thiết bị mới](../../../docs/agents/10-thiet-bi-moi.md). Selector không khớp mà chưa rõ do build hay do session/perception thì dùng `riviu-device-diagnostics` trước.

## 1. Luật cứng

- Chỉ đọc APK gốc kéo từ máy (`pm path` rồi `adb pull` trên một máy canary không bị controller giữ lease). Không sửa, ký lại hay cài APK đã sửa. Không kill hay restart adb server.
- Mã decompile, dump, bảng chuỗi, index và APK chỉ nằm trong `target/tiktok-<version>-research/` (git-ignore). Code và fixture chỉ được chứa id, chuỗi UI ngắn và tên class ngắn; không chép mã TikTok vào repo.
- Không chép id hay token `@<số>` từ build này sang build khác. TikTok làm rối tên id và đổi chúng ở mỗi build (`dou` ở 45.7.3 thành `dwh` ở 46.4.3); id chuỗi cũng đổi theo.
- Ứng viên không phải bằng chứng. Phải đọc trên **một** máy thật, theo từng trường và từng ngôn ngữ, trước khi đổi bất kỳ bảng nào; bảng mới ghi provenance gồm máy, ngày, versionCode và ngôn ngữ.
- Ứng viên `ambiguous`, `low` hay `none`, hoặc match cũ/mơ hồ, không bao giờ cho phép tap.

## 2. APK nói được gì, không nói được gì

| APK cho biết | APK không cho biết |
|---|---|
| Layout và resource id (kể cả id dùng chung cần scope theo ancestor) | Logic phía server |
| Chuỗi NxString của mọi locale (byte XOR 0x0F); bảng `default` là tiếng Anh | UI thử nghiệm A/B theo từng tài khoản |
| Ý nghĩa token `@<số>` trong content-desc/text của đúng build đó | Chữ do server gửi về, ví dụ tiêu đề sheet kiểm tra bảo mật |
| Danh sách activity trong manifest | Trang Lynx/web tải lúc chạy |
| Mã Java đã làm rối; chỉ decompile dex chứa hằng số cần tìm | Bên trong thư viện native `.so` (ngoài phạm vi, không đào) |

Phần cột phải chỉ biết được bằng dump uiautomator trên máy thật.

## 3. Quy tắc sản phẩm

Riviu điều hướng TikTok như một người dùng: chạm qua từng màn hình. Kiến thức từ APK dùng để **nhận ra** màn hình và nút, không dùng để tìm deep-link, intent hay activity cho phép nhảy cóc qua màn hình.

## 4. Các bước (chi tiết ở runbook)

1. Kéo `base.apk` (và mọi split) vào `target/tiktok-<version>-research/`.
2. `verify_apk.py`: kiểm chữ ký, signer và version. Gặp signer lạ thì dừng.
3. `nxstring.py`: liệt kê locale, giải bảng chuỗi của locale máy đang dùng.
4. `layout_index.py`: lập index layout của build mới.
5. `riviu_ids.py`: liệt kê id Riviu đang dùng trên build cũ (`unique`, `ambiguous`, `code-only`, `missing`).
6. `diff_ids.py`: xếp hạng ứng viên giữa hai build (`high` chỉ nghĩa là "đo id này trước").
7. `resolve_desc.py`: giải token `@<số>` lấy từ dump của build mới.
8. Duyệt ứng viên, ghi kết luận vào thư mục research. `find_string.py` tìm string id và layout từ một nhãn thấy trên màn hình; `dex_lookup.py const`/`classdef` tìm đúng dex trước khi decompile.
9. Đo trên một máy thật, thêm fixture đã sanitize và negative case, rồi mới sửa bảng.

Test của bộ công cụ: `py -3.12 -m unittest discover -s tools/tiktok-apk -p "test_*.py"` (dữ liệu tổng hợp, không dùng dữ liệu TikTok).

## 5. Emulator

Với 45.7.3, emulator Android Studio (x86_64 dịch arm64) chỉ tới được các màn đăng ký/đăng nhập. Feed crash trong bộ dịch ARM của emulator (lỗi của emulator, không phải TikTok phát hiện emulator). Vì vậy không dùng emulator cho feed, profile hay composer; các màn đó phải đo trên máy thật. Không nhập credential vào emulator.

## 6. Bàn giao

Báo build và versionCode đã phân tích, công cụ đã chạy, id/chuỗi nào mới là ứng viên và id nào đã đo trên máy thật (máy, ngày, ngôn ngữ). Nói rõ phần chỉ có bằng chứng tĩnh và phần chưa đo.
