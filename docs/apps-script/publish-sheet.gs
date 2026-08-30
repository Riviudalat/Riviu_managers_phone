/**
 * Riviu — nhận link bài đã đăng và ghi vào sheet đối tác.
 *
 * Dán toàn bộ file này vào Apps Script của chính spreadsheet đó
 * (Tiện ích mở rộng → Apps Script), sửa CONFIG bên dưới, rồi Triển khai → Ứng dụng web.
 *
 * ─── Bố cục nó ghi ────────────────────────────────────────────────────────────
 *
 *   cột D   link bài đăng
 *   cột B   người đăng — handle của máy khi app biết, `bot` khi chưa nhập  ← cột duy nhất tôi tự chọn, xem CONFIG
 *   cột K+  tên đối tác, trải ngang theo đúng thứ tự trong partners-setN.xlsx
 *   cột Z   assignmentId — KHOÁ CHỐNG TRÙNG, xem mục 2
 *
 * ─── Bốn điều đáng đọc trước khi triển khai ──────────────────────────────────
 *
 * 1. **URL của ứng dụng web KHÔNG phải là mật khẩu.** Deploy với quyền "Bất kỳ ai"
 *    nghĩa là bất kỳ ai có URL đều gọi được, và Google không xác thực người gọi.
 *    Nên script so TOKEN trước khi ghi bất cứ thứ gì.
 *
 *    Hai giới hạn của cách đó, nói thẳng: token vẫn KHÔNG chặn được người ta đốt
 *    hạn mức chạy của anh bằng request sai token, vì mỗi request vẫn khởi động một
 *    lượt chạy. Và đổi token trong một bản deploy MỚI không thu hồi token nằm
 *    trong bản deploy CŨ — muốn xoay token thì phải **vô hiệu hoá mọi bản deploy
 *    cũ**, không chỉ đổi URL bên app.
 *
 * 2. **Khoá chống trùng nằm TRONG SHEET, ở cột Z — không nằm trong Script
 *    Properties.** Bản đầu tiên của file này ghi hàng trước rồi mới ghi khoá vào
 *    PropertiesService, và giữa hai thao tác đó có một khe: bất cứ lỗi nào rơi vào
 *    đấy — kể cả hết hạn mức 500 KB của chính kho property, thứ không bao giờ được
 *    dọn — để lại một hàng có link mà phép chống trùng không nhận ra, nên lần thử
 *    lại dán đúng link đó xuống hàng kế tiếp. Vĩnh viễn.
 *
 *    Hàng tự nó là bản ghi. Cùng một `setValues` viết ra link, người đăng, tên quán
 *    và khoá; không có khe nào. Và nó sống sót qua việc thay project, copy
 *    spreadsheet hay xoá property, vì nó ở trong đúng cái sheet mà nó nói về.
 *
 * 3. **Không dùng getActiveSpreadsheet().** Một web app không có "spreadsheet đang
 *    hoạt động" — hàm đó trả null khi chạy qua /exec, dù nó chạy tốt trong trình
 *    soạn thảo. Đó là lý do bản trước "thử trong editor thì được, deploy thì
 *    không". Điền SPREADSHEET_ID và dùng openById.
 *
 * 4. **Triển khai lại thì URL đổi.** Dùng "Quản lý bản triển khai → sửa → phiên bản
 *    Mới nhất" để giữ nguyên URL. Nếu app báo "webhook Sheet trả thứ không phải
 *    JSON" thì gần như chắc chắn URL đang trỏ vào bản deploy cũ.
 */

const CONFIG = {
  /**
   * Id của spreadsheet — lấy trong URL của nó, đoạn giữa /d/ và /edit.
   * Bắt buộc: xem mục 3 ở trên.
   */
  SPREADSHEET_ID: 'DIEN-ID-SPREADSHEET-VAO-DAY',

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
   * **Con số duy nhất trong file này tôi phải tự chọn** — anh nói người đăng là
   * `bot` nhưng không nói cột nào, nên tôi để B và tách nó ra đây để anh sửa một chỗ.
   */
  POSTER_COLUMN: 2,

  /** Cột đầu tiên của tên đối tác. Anh chốt cột K. */
  PARTNERS_START_COLUMN: 11,

  /**
   * Số cột tối đa dành cho tên đối tác, tính từ cột K.
   *
   * Có hai việc: chặn một danh sách dài tràn ra ngoài lưới (17 tên từ cột K là tới
   * AA, và trên một sheet 26 cột thì getRange ném lỗi), và **xoá sạch tên cũ**.
   * Bản trước chỉ ghi đè đúng số ô mới cần, nên một hàng đang có A,B,C mà nhận đúng
   * một tên X sẽ thành X,B,C — ba đối tác cho một bài chỉ có một.
   */
  PARTNERS_MAX: 12,

  /**
   * Cột giữ assignmentId. Xem mục 2 — đây là thứ làm cho một lần gửi lại không
   * thành hai hàng.
   */
  KEY_COLUMN: 26,

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
    const postUrl = String(payload.postUrl || '').trim();
    const assignmentId = String(payload.assignmentId || '').trim();
    if (!postUrl) {
      return reply({ ok: false, error: 'thiếu postUrl' });
    }
    if (!assignmentId) {
      // Không có khoá thì không chống trùng được, và một lần gửi lại sẽ dán hai link.
      return reply({ ok: false, error: 'thiếu assignmentId' });
    }

    // Khoá theo script: hai request đồng thời của cùng một assignment nối đuôi nhau,
    // nên request thứ hai đọc lại cột khoá và thấy hàng đã ghi.
    const lock = LockService.getScriptLock();
    lock.waitLock(30000);
    try {
      const sheet = targetSheet();
      if (!sheet) {
        return reply({ ok: false, error: 'không mở được sheet — kiểm tra SPREADSHEET_ID' });
      }

      const existing = rowWithKey(sheet, assignmentId);
      if (existing > 0) {
        return reply({ ok: true, duplicate: true, row: existing });
      }

      const row = firstRowWithoutLink(sheet);
      const partners = (payload.partners || []).slice(0, CONFIG.PARTNERS_MAX);

      // **Một lần ghi cho cả hàng.** Link, người đăng, tên quán và khoá đi cùng nhau,
      // nên không có trạng thái nào ở giữa mà phép chống trùng không nhận ra. Cột nào
      // không thuộc về script thì giữ nguyên giá trị đang có.
      const left = Math.min(CONFIG.POSTER_COLUMN, CONFIG.LINK_COLUMN, CONFIG.PARTNERS_START_COLUMN);
      const right = Math.max(
        CONFIG.POSTER_COLUMN,
        CONFIG.LINK_COLUMN,
        CONFIG.PARTNERS_START_COLUMN + CONFIG.PARTNERS_MAX - 1,
        CONFIG.KEY_COLUMN
      );
      const width = right - left + 1;
      const range = sheet.getRange(row, left, 1, width);
      const values = range.getValues()[0];

      values[CONFIG.LINK_COLUMN - left] = asText(postUrl);
      values[CONFIG.POSTER_COLUMN - left] = asText(payload.poster || 'bot');
      values[CONFIG.KEY_COLUMN - left] = asText(assignmentId);
      for (let offset = 0; offset < CONFIG.PARTNERS_MAX; offset += 1) {
        const at = CONFIG.PARTNERS_START_COLUMN - left + offset;
        // Ô nào không có tên mới thì XOÁ, chứ không để tên của bài trước nằm lại.
        values[at] = offset < partners.length ? asText(partners[offset]) : '';
      }
      range.setValues([values]);

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
 * Sheet đích, mở bằng id.
 *
 * `getActiveSpreadsheet()` là accessor của ngữ cảnh container/giao diện; một web app
 * chạy qua /exec không có ngữ cảnh đó và nó trả null. Xem mục 3 ở đầu file.
 */
function targetSheet() {
  const book = SpreadsheetApp.openById(CONFIG.SPREADSHEET_ID);
  if (!book) {
    return null;
  }
  return CONFIG.SHEET_NAME ? book.getSheetByName(CONFIG.SHEET_NAME) : book.getSheets()[0];
}

/**
 * Hàng đã mang assignmentId này, hoặc 0.
 *
 * Đọc từ chính sheet, nên nó đúng dù project bị thay, spreadsheet bị copy, hay kho
 * property bị xoá — và nó sai đi cùng chiều với sheet, chứ không nói "đã ghi rồi" về
 * một hàng mà anh vừa xoá.
 */
function rowWithKey(sheet, assignmentId) {
  const lastRow = sheet.getLastRow();
  if (lastRow < CONFIG.FIRST_DATA_ROW) {
    return 0;
  }
  const height = lastRow - CONFIG.FIRST_DATA_ROW + 1;
  const keys = sheet.getRange(CONFIG.FIRST_DATA_ROW, CONFIG.KEY_COLUMN, height, 1).getValues();
  for (let index = 0; index < keys.length; index += 1) {
    if (String(keys[index][0]).trim() === assignmentId) {
      return CONFIG.FIRST_DATA_ROW + index;
    }
  }
  return 0;
}

/**
 * Dòng trống đầu tiên xét theo CỘT LINK, không phải theo `getLastRow()`.
 *
 * `getLastRow()` trả dòng cuối có bất cứ thứ gì trong đó, nên nếu sheet đã có sẵn
 * danh sách quán ở cột K thì nó nhảy xuống dưới cả danh sách và ghi link vào chỗ
 * không có quán nào.
 *
 * **Một ô có công thức là ô ĐÃ CÓ CHỦ**, kể cả khi công thức đang trả về chuỗi rỗng.
 * `getValues()` trả giá trị đã tính, nên một công thức chờ dữ liệu trông y hệt một ô
 * trống — và ghi đè lên nó là xoá công thức của anh.
 *
 * Và khi cả lưới đã đầy thì **chèn thêm hàng**, chứ không trả `lastRow + 1` rồi để
 * `getRange` ném lỗi trên một hàng không tồn tại.
 */
function firstRowWithoutLink(sheet) {
  const lastRow = sheet.getLastRow();
  if (lastRow >= CONFIG.FIRST_DATA_ROW) {
    const height = lastRow - CONFIG.FIRST_DATA_ROW + 1;
    const target = sheet.getRange(CONFIG.FIRST_DATA_ROW, CONFIG.LINK_COLUMN, height, 1);
    const values = target.getValues();
    const formulas = target.getFormulas();
    for (let index = 0; index < values.length; index += 1) {
      const blank = String(values[index][0]).trim() === '';
      const claimed = String(formulas[index][0]).trim() !== '';
      if (blank && !claimed) {
        return CONFIG.FIRST_DATA_ROW + index;
      }
    }
  }
  const next = Math.max(lastRow + 1, CONFIG.FIRST_DATA_ROW);
  const needed = Math.max(next, CONFIG.KEY_COLUMN > 0 ? next : next);
  if (needed > sheet.getMaxRows()) {
    sheet.insertRowsAfter(sheet.getMaxRows(), needed - sheet.getMaxRows());
  }
  return next;
}

/**
 * Ép một chuỗi thành CHỮ, không phải công thức.
 *
 * `setValues` diễn giải chuỗi bắt đầu bằng `=` như một công thức, nên một tên quán
 * `=IMAGE("https://...")` — dù đến từ file xlsx của anh hay từ ai đó cầm token — sẽ
 * chạy thật chứ không nằm im. Dấu nháy đầu là cách Sheets đánh dấu "đây là chữ"; nó
 * không hiện ra trong ô.
 */
function asText(value) {
  const text = String(value === null || value === undefined ? '' : value);
  return /^[=+\-@]/.test(text) ? "'" + text : text;
}

function reply(body) {
  return ContentService.createTextOutput(JSON.stringify(body)).setMimeType(
    ContentService.MimeType.JSON
  );
}

/**
 * Chạy tay trong trình soạn thảo để kiểm bố cục.
 *
 * **Không thay thế được một lần thử qua URL /exec đã deploy** — mục 3 ở đầu file là
 * đúng cái bẫy này: bản chạy trong editor có ngữ cảnh container, bản deploy thì không.
 * Cái này chỉ nói với anh rằng bố cục cột đúng.
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
