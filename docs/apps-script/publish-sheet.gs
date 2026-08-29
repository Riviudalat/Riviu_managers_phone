/**
 * Riviu — nhận link bài đã đăng và ghi vào sheet đối tác.
 *
 * Dán toàn bộ file này vào Apps Script của chính spreadsheet đó
 * (Tiện ích mở rộng → Apps Script), sửa CONFIG bên dưới, rồi Triển khai → Ứng dụng web.
 *
 * ─── Bố cục nó ghi ────────────────────────────────────────────────────────────
 *
 *   cột D   link bài đăng
 *   cột B   người đăng, luôn là `bot`      ← XEM GHI CHÚ, đây là cột duy nhất tôi tự chọn
 *   cột K+  tên đối tác, trải ngang theo đúng thứ tự trong partners-setN.xlsx
 *
 * ─── Ba điều đáng đọc trước khi triển khai ───────────────────────────────────
 *
 * 1. **URL của ứng dụng web KHÔNG phải là mật khẩu.** Deploy với quyền "Bất kỳ ai"
 *    nghĩa là bất kỳ ai có URL đều gọi được, và Google không xác thực người gọi.
 *    Nên script so TOKEN trước khi ghi bất cứ thứ gì. Đổi TOKEN thành một chuỗi
 *    ngẫu nhiên dài, và điền đúng chuỗi đó vào cài đặt của app.
 *
 * 2. **`assignmentId` là khoá chống trùng, và nó là thứ giữ cho một lần timeout
 *    không thành hai dòng.** Nếu mạng đứt sau khi script đã ghi xong, desktop
 *    không phân biệt được "chưa tới" với "tới rồi mà không nghe được trả lời" —
 *    nên nó gửi lại. Script nhớ các khoá đã ghi và trả `duplicate: true` thay vì
 *    dán link lần thứ hai vào cột D.
 *
 * 3. **Triển khai lại thì URL đổi.** Mỗi lần "Triển khai → Bản triển khai mới"
 *    sinh một URL khác; bản cũ vẫn sống và vẫn chạy mã CŨ. Nếu app báo
 *    "webhook Sheet trả thứ không phải JSON" thì gần như chắc chắn là URL đang trỏ
 *    vào bản deploy cũ. Dùng "Quản lý bản triển khai → sửa → phiên bản Mới nhất"
 *    để giữ nguyên URL.
 */

const CONFIG = {
  /**
   * Đổi thành một chuỗi ngẫu nhiên dài, rồi điền y hệt vào app.
   * Để nguyên giá trị này là để ngỏ cả sheet cho bất kỳ ai đoán ra URL.
   */
  TOKEN: 'DOI-CHUOI-NAY-DI',

  /** Tên tab trong spreadsheet. Để trống thì dùng tab đầu tiên. */
  SHEET_NAME: '',

  /** Cột link bài đăng. Anh chốt cột D. */
  LINK_COLUMN: 4,

  /**
   * Cột người đăng.
   *
   * **Đây là con số duy nhất trong file này tôi phải tự chọn** — anh nói người đăng
   * là `bot` nhưng không nói cột nào, nên tôi để B và tách nó ra đây để anh sửa một
   * chỗ. Nếu sheet của anh để người đăng ở cột khác thì đổi số này, đừng sửa chỗ khác.
   */
  POSTER_COLUMN: 2,

  /** Cột đầu tiên của tên đối tác. Anh chốt cột K. */
  PARTNERS_START_COLUMN: 11,

  /**
   * Dòng đầu tiên chứa dữ liệu (bỏ qua dòng tiêu đề).
   *
   * Script ghi vào dòng trống đầu tiên TỪ ĐÂY TRỞ XUỐNG, xét theo cột link — nên một
   * dòng đã có tên đối tác mà chưa có link vẫn được coi là chỗ trống và được điền vào.
   * Đó là hành vi mong muốn: sheet của anh có sẵn danh sách quán chờ bài.
   */
  FIRST_DATA_ROW: 2,
};

/** Ứng dụng web nhận POST ở đây. */
function doPost(request) {
  try {
    const payload = JSON.parse(request.postData.contents);

    if (!CONFIG.TOKEN || CONFIG.TOKEN === 'DOI-CHUOI-NAY-DI') {
      return reply({ ok: false, error: 'script chưa đổi TOKEN' });
    }
    if (payload.token !== CONFIG.TOKEN) {
      return reply({ ok: false, error: 'token sai' });
    }
    if (!payload.postUrl) {
      return reply({ ok: false, error: 'thiếu postUrl' });
    }
    if (!payload.assignmentId) {
      // Không có khoá thì không chống trùng được, và một lần gửi lại sẽ dán hai link.
      return reply({ ok: false, error: 'thiếu assignmentId' });
    }

    // Một khoá mỗi lần, khoá theo script: hai request đồng thời của cùng một assignment
    // sẽ nối đuôi nhau, nên request thứ hai thấy khoá đã ghi và trả duplicate.
    const lock = LockService.getScriptLock();
    lock.waitLock(30000);
    try {
      const seen = PropertiesService.getScriptProperties();
      const key = 'written:' + payload.assignmentId;
      if (seen.getProperty(key)) {
        return reply({ ok: true, duplicate: true });
      }

      const sheet = CONFIG.SHEET_NAME
        ? SpreadsheetApp.getActiveSpreadsheet().getSheetByName(CONFIG.SHEET_NAME)
        : SpreadsheetApp.getActiveSpreadsheet().getSheets()[0];
      if (!sheet) {
        return reply({ ok: false, error: 'không thấy tab ' + CONFIG.SHEET_NAME });
      }

      const row = firstRowWithoutLink(sheet);
      sheet.getRange(row, CONFIG.LINK_COLUMN).setValue(payload.postUrl);
      sheet.getRange(row, CONFIG.POSTER_COLUMN).setValue(payload.poster || 'bot');

      const partners = payload.partners || [];
      if (partners.length > 0) {
        sheet
          .getRange(row, CONFIG.PARTNERS_START_COLUMN, 1, partners.length)
          .setValues([partners]);
      }

      seen.setProperty(key, String(row));
      return reply({ ok: true, row: row });
    } finally {
      lock.releaseLock();
    }
  } catch (error) {
    // Trả JSON kể cả khi hỏng, vì phía desktop phân biệt "script từ chối" với
    // "Google trả trang lỗi HTML" bằng đúng chuyện body có phải JSON hay không.
    return reply({ ok: false, error: String(error) });
  }
}

/**
 * Dòng trống đầu tiên xét theo CỘT LINK, không phải theo `getLastRow()`.
 *
 * `getLastRow()` trả dòng cuối có bất cứ thứ gì trong đó, nên nếu sheet đã có sẵn
 * danh sách quán ở cột K thì nó nhảy xuống dưới cả danh sách và ghi link vào chỗ
 * không có quán nào.
 */
function firstRowWithoutLink(sheet) {
  const lastRow = sheet.getLastRow();
  if (lastRow < CONFIG.FIRST_DATA_ROW) {
    return CONFIG.FIRST_DATA_ROW;
  }
  const height = lastRow - CONFIG.FIRST_DATA_ROW + 1;
  const links = sheet
    .getRange(CONFIG.FIRST_DATA_ROW, CONFIG.LINK_COLUMN, height, 1)
    .getValues();
  for (let index = 0; index < links.length; index += 1) {
    if (String(links[index][0]).trim() === '') {
      return CONFIG.FIRST_DATA_ROW + index;
    }
  }
  return lastRow + 1;
}

function reply(body) {
  return ContentService.createTextOutput(JSON.stringify(body)).setMimeType(
    ContentService.MimeType.JSON
  );
}

/**
 * Chạy tay trong trình soạn thảo để kiểm bố cục mà không cần app.
 *
 * Ghi một dòng thử rồi báo nó nằm ở dòng nào; xoá dòng đó đi sau khi xem.
 */
function thuGhiMotDong() {
  const out = doPost({
    postData: {
      contents: JSON.stringify({
        token: CONFIG.TOKEN,
        postUrl: 'https://www.tiktok.com/@thu/photo/0',
        poster: 'bot',
        partners: ['Quán thử A', 'Quán thử B'],
        assignmentId: 'thu-' + Date.now(),
      }),
    },
  });
  Logger.log(out.getContent());
}
