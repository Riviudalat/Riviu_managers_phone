/**
 * Riviu — nhận link bài đã đăng và ghi vào sheet đối tác.
 *
 * Dán toàn bộ file này vào Apps Script của chính spreadsheet đó
 * (Tiện ích mở rộng → Apps Script), sửa CONFIG bên dưới, rồi Triển khai → Ứng dụng web.
 *
 * Mẫu mới (LAYOUT_MODE auto/compact): A STT, B Người air, C Ngày, D Link,
 * E+ Đối tác, Đối tác 2...; không thêm cột kỹ thuật. Khoá assignmentId nằm trong
 * ghi chú ô Link. Bật dịch vụ nâng cao Google Sheets API trong Apps Script:
 * một batchUpdate ghi cả dữ liệu và ghi chú nguyên tử, không có khe chống trùng.
 * postedAt là thời điểm gửi bài đã lưu trong app, không phải thời điểm thử lại.
 * Ngày dùng múi giờ của spreadsheet. STT là max STT hiện có + 1, giữ khi gửi lại.
 * Nếu thiếu dịch vụ/postedAt/đúng header thì compact từ chối trước khi ghi.
 * Mẫu nội bộ thêm E Máy, F Tài khoản TikTok, G Trạng thái, H Lỗi hoặc ghi chú;
 * I+ giữ toàn bộ đối tác. Payload reportVersion=1 dùng revision và fingerprint
 * trong ghi chú D; bài chưa gửi được để trống Ngày/Link trên cùng dòng đã có khoá.
 *
 * Bố cục cũ dưới đây vẫn dùng khi LAYOUT_MODE=legacy hoặc auto nhận header cũ.
 * ─── Bố cục cũ ────────────────────────────────────────────────────────────────
 *
 *   cột D   link bài đăng
 *   cột B   người đăng — handle của máy khi app biết, `bot` khi chưa nhập  ← cột duy nhất tôi tự chọn, xem CONFIG
 *   cột K+  tên đối tác, trải ngang theo đúng thứ tự trong partners-setN.xlsx
 *   cột AH  assignmentId — KHOÁ CHỐNG TRÙNG, xem mục 2
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
 * 2. **Khoá chống trùng của mẫu legacy nằm TRONG SHEET, ở cột AH — không nằm trong Script
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
  /** auto: nhận header; internal/compact: bắt buộc mẫu tương ứng; legacy: K+/AH. */
  LAYOUT_MODE: 'auto',
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

  /**
   * Tab đích, chọn theo **gid** — con số trong URL sau `#gid=`.
   *
   * Đo 31/08/2026 trên sheet thật và đây là lý do trường này tồn tại: tab đầu tiên của
   * workbook KHÔNG phải tab đăng bài. Tab đầu có cột B trống, tab đăng bài có B là
   * `Nhân Viên` — hai bố cục khác nhau. Với `SHEET_NAME: ''` (dùng tab đầu) script sẽ ghi
   * vào nhầm tab, và ghi rất im lặng.
   *
   * gid thay vì tên vì gid nằm sẵn trong URL anh đang mở, và **đổi tên tab không đổi gid** —
   * còn một cái tên gõ sai thì `getSheetByName` trả null và script từ chối, một cái tên gõ
   * đúng-nhưng-của-tab-khác thì nó ghi vào đó.
   *
   * `0` là gid thật của tab đầu. `null` mới dùng SHEET_NAME/tab đầu tiên.
   */
  SHEET_GID: 0,

  /** Tên tab, chỉ dùng khi SHEET_GID = null. Để trống thì dùng tab đầu tiên. */
  SHEET_NAME: '',

  /** Cột link bài đăng. Anh chốt cột D. */
  LINK_COLUMN: 4,

  /**
   * Cột người đăng — **B, đã đối chiếu sheet thật 31/08/2026 và anh chốt.**
   *
   * B là cột `Nhân Viên`: 1892 dòng, 11 cái tên người (`Phúc`, `Lành`, `Quỳnh`, …). App ghi
   * `bot` vào đó, thành giá trị thứ 12 — nhìn một cái là biết dòng nào người đăng, dòng nào
   * máy đăng.
   *
   * Tài khoản nào đăng thì KHÔNG mất: link ở cột D mang sẵn `@handle` trong đường dẫn. Cột E
   * `Tên Kênh` có tiêu đề nhưng trống cả 1892 dòng — script không đụng tới, để anh quyết.
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
  /**
   * **23, đếm trên sheet thật ngày 31/08/2026** — tiêu đề chạy `Đối tác` … `Đối tác 23`
   * từ cột K tới AG, và đã có dòng dùng đủ cả 23.
   *
   * Con số cũ là **12**, một phỏng đoán, và nó sai hai đường cùng lúc: script chỉ ghi 12
   * tên đầu (mất 11 tên), và phép "xoá tên cũ" chỉ quét K..V nên tên đối tác cũ ở W..AG
   * của dòng đó **nằm lại** — 12 tên mới trộn với tên cũ, không ai nhìn ra.
   */
  PARTNERS_MAX: 23,

  /**
   * Cột giữ assignmentId. Xem mục 2 — đây là thứ làm cho một lần gửi lại không
   * thành hai hàng.
   *
   * **34 = AH, ô trống đầu tiên sau khối đối tác.** Con số cũ là 26 = **cột Z**, và trên
   * sheet thật Z là `Đối tác 16` — mỗi lần ghi khoá là **đè mất tên đối tác thứ 16**. Đó
   * là lý do `assertConfigIsSane` bên dưới tồn tại: ba con số này phụ thuộc nhau, và khi
   * chúng lệch thì hậu quả là dữ liệu sai chứ không phải một lỗi.
   *
   * Lưới sheet thật rộng tới AQ (43 cột), nên AH nằm trong lưới — script không phải chèn
   * cột, và không bao giờ nên tự chèn.
   */
  KEY_COLUMN: 34,

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
    if (payload.rowKind === 'check') return reply(checkTarget(payload));
    if (payload.rowKind === 'prepare') return reply(prepareTarget(payload));
    const postUrl = String(payload.postUrl || '').trim();
    const assignmentId = String(payload.assignmentId || '').trim();
    if (!postUrl && payload.rowKind !== 'internalReport') {
      return reply({ ok: false, error: 'thiếu postUrl' });
    }
    if (!assignmentId) {
      // Không có khoá thì không chống trùng được, và một lần gửi lại sẽ dán hai link.
      return reply({ ok: false, error: 'thiếu assignmentId' });
    }

    // Khoá theo script: hai request đồng thời của cùng một assignment nối đuôi nhau,
    // nên request thứ hai đọc lại cột khoá và thấy hàng đã ghi.
    const unsound = assertConfigIsSane();
    if (unsound) {
      return reply({ ok: false, error: unsound });
    }

    const lock = LockService.getScriptLock();
    lock.waitLock(30000);
    try {
      const sheet = targetSheet();
      if (!sheet) {
        return reply({ ok: false, error: 'không mở được sheet — kiểm tra SPREADSHEET_ID' });
      }

      const currentLayout = compactLayout(sheet);
      validateDeliveryTarget(sheet, payload);
      if (payload.deliveryVersion === 2 && !currentLayout) throw new Error('deliveryVersion 2 cần mẫu compact hoặc nội bộ');
      if (payload.deliveryVersion === 2 && currentLayout && !currentLayout.internal
        && (payload.reportVersion !== undefined || payload.rowRevision !== undefined)) throw new Error('Metadata báo cáo chỉ dùng mẫu nội bộ');
      const layout = currentLayout ? planPartnerColumns(currentLayout, payload.partners) : null;
      if (payload.rowKind === 'internalReport' && (!layout || !layout.internal)) {
        throw new Error('Báo cáo nội bộ cần đúng bốn cột Máy, Tài khoản TikTok, Trạng thái, Lỗi hoặc ghi chú');
      }
      if (layout && layout.internal) {
        return reply(finishDelivery(sheet, layout, payload, deliverInternal(sheet, layout, payload, postUrl, assignmentId)));
      }
      if (layout) {
        return reply(finishDelivery(sheet, layout, payload, deliverCompact(sheet, layout, payload, postUrl, assignmentId)));
      }

      if (!Array.isArray(payload.partners) || payload.partners.length > CONFIG.PARTNERS_MAX) {
        return reply({ ok: false, error: 'Danh sách đối tác không hợp lệ hoặc vượt số cột; không cắt bỏ tên' });
      }
      const existing = rowWithKey(sheet, assignmentId);
      if (existing > 0) {
        const currentLink = String(sheet.getRange(existing, CONFIG.LINK_COLUMN).getValues()[0][0]);
        const currentPoster = String(sheet.getRange(existing, CONFIG.POSTER_COLUMN).getValues()[0][0]);
        const currentPartners = sheet.getRange(existing, CONFIG.PARTNERS_START_COLUMN, 1, CONFIG.PARTNERS_MAX).getValues()[0];
        if (currentLink !== postUrl || currentPoster !== String(payload.poster || 'bot')
          || currentPartners.some((value, index) => String(value) !== String(payload.partners[index] || ''))) {
          return reply({ ok: false, error: 'Khoá đã ghi nhưng dữ liệu khác; kiểm tra trước khi gửi lại' });
        }
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
      const formulas = range.getFormulas()[0];
      // Keep formulas in untouched legacy columns; getValues alone freezes their result.
      for (let index = 0; index < width; index++) {
        if (formulas[index]) values[index] = formulas[index];
      }

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
    return reply({ ok: false, error: String(error), retryable: Boolean(error && error.sheetRetryable) });
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
  // gid trước, vì nó là thứ nằm trong URL và không đổi khi tab bị đổi tên. Không tìm thấy
  // thì trả null để `doPost` từ chối — KHÔNG lặng lẽ rơi về tab đầu tiên, vì "tab đầu tiên"
  // chính là cái tab sai mà trường này sinh ra để tránh.
  if (CONFIG.SHEET_GID !== null && CONFIG.SHEET_GID !== undefined && CONFIG.SHEET_GID !== '') {
    const sheets = book.getSheets();
    for (let index = 0; index < sheets.length; index += 1) {
      if (sheets[index].getSheetId() === CONFIG.SHEET_GID) {
        return sheets[index];
      }
    }
    return null;
  }
  return CONFIG.SHEET_NAME ? book.getSheetByName(CONFIG.SHEET_NAME) : book.getSheets()[0];
}

/** Read-only target proof. This path never creates rows, notes, locks or test values. */
function checkTarget(payload) {
  if (payload.checkVersion !== 1) throw new Error('checkVersion không được hỗ trợ');
  const unsound = assertConfigIsSane();
  if (unsound) throw new Error(unsound);
  const sheet = targetSheet();
  if (!sheet) throw new Error('Không mở được tab đã cấu hình');
  const spreadsheetId = sheet.getParent().getId();
  const sheetGid = sheet.getSheetId();
  if (String(payload.spreadsheetId || '') !== spreadsheetId || payload.sheetGid !== sheetGid) {
    throw new Error('Kết nối ghi đang trỏ tới bảng hoặc tab khác');
  }
  const columns = sheet.getRange(1, 1, 1, sheet.getMaxColumns()).getDisplayValues()[0].map(String);
  const compact = compactLayout(sheet);
  if (!compact && (!['Nhân Viên', 'Nhân viên', 'Người đăng'].includes((columns[1] || '').trim())
    || !String(columns[3] || '').toLowerCase().includes('link') || (columns[10] || '').trim() !== 'Đối tác')) {
    throw new Error('Header legacy chưa khớp mẫu Đăng bài');
  }
  return { ok: true, checkVersion: 1, deliveryVersion: compact ? 2 : 1, spreadsheetId, sheetGid,
    layout: compact ? (compact.internal ? 'internal' : 'compact') : 'legacy', columns };
}

/** Explicit initialization, serialized with all deliveries; never rewrites a populated tab. */
function prepareTarget(payload) {
  if (payload.checkVersion !== 1) throw new Error('checkVersion không được hỗ trợ');
  const unsound = assertConfigIsSane();
  if (unsound) throw new Error(unsound);
  const lock = LockService.getScriptLock();
  lock.waitLock(30000);
  try {
    const sheet = targetSheet();
    if (!sheet || String(payload.spreadsheetId || '') !== sheet.getParent().getId() || payload.sheetGid !== sheet.getSheetId()) throw new Error('Bảng hoặc tab không khớp kết nối');
    if (sheet.getLastRow() === 0 && sheet.getLastColumn() === 0) {
      if (!['auto', 'internal'].includes(CONFIG.LAYOUT_MODE) || CONFIG.LINK_COLUMN !== 4 || CONFIG.POSTER_COLUMN !== 2) throw new Error('Chuẩn bị bảng trống cần cấu hình mẫu nội bộ với cột B/D');
      const header = ['STT', 'Người air', 'Ngày', 'Link', 'Máy', 'Tài khoản TikTok', 'Trạng thái', 'Lỗi hoặc ghi chú', 'Đối tác'];
      if (typeof Sheets === 'undefined' || !Sheets.Spreadsheets?.batchUpdate) throw new Error('Bật dịch vụ Google Sheets API trước khi chuẩn bị bảng');
      const requests = [];
      if (sheet.getMaxColumns() < header.length) requests.push({ appendDimension: { sheetId: sheet.getSheetId(), dimension: 'COLUMNS', length: header.length - sheet.getMaxColumns() } });
      requests.push({ updateCells: { start: { sheetId: sheet.getSheetId(), rowIndex: 0, columnIndex: 0 }, rows: [{ values: header.map(value => ({ userEnteredValue: { stringValue: value } })) }], fields: 'userEnteredValue' } });
      requests.push({ updateSheetProperties: { properties: { sheetId: sheet.getSheetId(), gridProperties: { frozenRowCount: 1 } }, fields: 'gridProperties.frozenRowCount' } });
      commitSheetBatch(sheet, { extra: 0 }, 1, requests);
    }
    return checkTarget(payload);
  } finally { lock.releaseLock(); }
}

function planPartnerColumns(layout, partners) {
  if (!Array.isArray(partners) || partners.some(value => typeof value !== 'string')) throw new Error('partners phải là danh sách tên đối tác');
  const count = partners.length;
  if (!Number.isInteger(count) || count < 0 || count > 100) throw new Error('Số đối tác không hợp lệ');
  const extra = Math.max(0, count - layout.partners);
  return Object.assign({}, layout, { originalWidth: layout.width, originalPartners: layout.partners,
    width: layout.width + extra, partners: layout.partners + extra, extra: extra });
}

function validateDeliveryTarget(sheet, payload) {
  if (payload.deliveryVersion === undefined) return;
  if (payload.deliveryVersion !== 2 || !Number.isSafeInteger(payload.deliveryRevision) || payload.deliveryRevision < 0
    || !['canonical', 'internalReport'].includes(payload.rowKind)) throw new Error('deliveryVersion, deliveryRevision hoặc rowKind không hợp lệ');
  if (payload.spreadsheetId !== sheet.getParent().getId() || payload.sheetGid !== sheet.getSheetId()) throw new Error('Đích Sheet khác bảng hoặc tab đã chốt');
  if (typeof payload.assignmentId !== 'string' || !payload.assignmentId.trim() || payload.assignmentId !== payload.assignmentId.trim()
    || typeof payload.postUrl !== 'string' || payload.postUrl !== payload.postUrl.trim()
    || (payload.postUrl && !/^https:\/\/(?:www\.)?tiktok\.com\/@[^/?#]+\/(?:photo|video)\/\d+$/.test(payload.postUrl))
    || typeof payload.poster !== 'string' || !payload.poster.trim()) throw new Error('Danh tính hoặc canonical Link không hợp lệ');
  if (payload.rowKind === 'internalReport' && payload.deliveryRevision !== payload.rowRevision) throw new Error('Revision báo cáo không khớp deliveryRevision');
}

function ownedRows(sheet, layout, height, method) {
  if (!height) return [];
  const width = layout.originalWidth || layout.width;
  return sheet.getRange(CONFIG.FIRST_DATA_ROW, 1, height, width)[method]()
    .map(row => row.concat(Array(layout.width - width).fill('')));
}

function commitSheetBatch(sheet, layout, row, requests) {
  const prefix = [];
  if (layout.extra > 0) {
    prefix.push({ insertDimension: { range: { sheetId: sheet.getSheetId(), dimension: 'COLUMNS', startIndex: layout.originalWidth, endIndex: layout.width }, inheritFromBefore: true } });
    prefix.push({ updateCells: { start: { sheetId: sheet.getSheetId(), rowIndex: 0, columnIndex: layout.originalWidth },
      rows: [{ values: Array.from({ length: layout.extra }, (_, index) => ({ userEnteredValue: { stringValue: 'Đối tác ' + (layout.originalPartners + index + 1) } })) }], fields: 'userEnteredValue' } });
  }
  if (row > sheet.getMaxRows()) prefix.push({ appendDimension: { sheetId: sheet.getSheetId(), dimension: 'ROWS', length: row - sheet.getMaxRows() } });
  try {
    Sheets.Spreadsheets.batchUpdate({ requests: prefix.concat(requests) }, sheet.getParent().getId());
  } catch (error) {
    // A request can commit before its response is lost. Only this transport phase
    // is retryable; every validation and row conflict above it remains permanent.
    error.sheetRetryable = !/permission|forbidden|unauthori[sz]ed|invalid argument|not found|does not exist|access denied/i.test(String(error));
    throw error;
  }
}

function validateStoredDelivery(stored, payload, values, sheet) {
  if (!stored || stored.deliveryVersion !== 2) return;
  if (stored.spreadsheetId !== sheet.getParent().getId() || stored.sheetGid !== sheet.getSheetId()
    || !rowFingerprintMatches(values, stored.rowFingerprint)) throw new Error('Đích hoặc nội dung hàng đã ghi bị thay đổi');
  if (payload.deliveryVersion === 2 && payload.rowKind === 'canonical' && stored.canonicalDeliveryRevision !== undefined
    && stored.canonicalDeliveryRevision !== payload.deliveryRevision) throw new Error('deliveryRevision canonical khác hàng đã lưu');
  if (payload.deliveryVersion === 2 && payload.rowKind === 'canonical' && stored.canonicalPostedAt !== undefined
    && stored.canonicalPostedAt !== payload.postedAt) throw new Error('Thời điểm canonical khác hàng đã lưu');
}

function deliveryNote(payload, stored, values) {
  if (payload.deliveryVersion !== 2) return stored;
  const note = reportNote(stored) || {};
  delete note.legacy;
  note.kind = 'riviu-publish';
  note.deliveryVersion = 2;
  note.assignmentId = payload.assignmentId;
  note.spreadsheetId = payload.spreadsheetId;
  note.sheetGid = payload.sheetGid;
  if (payload.rowKind === 'canonical') {
    note.canonicalDeliveryRevision = payload.deliveryRevision;
    note.canonicalPostedAt = payload.postedAt;
  }
  note.rowFingerprint = reportFingerprint(values);
  return JSON.stringify(note);
}

function finishDelivery(sheet, layout, payload, result) {
  if (payload.deliveryVersion !== 2) return result;
  const row = result.row;
  const range = sheet.getRange(row, 1, 1, layout.width);
  let values = range.getDisplayValues()[0];
  const noteRange = sheet.getRange(row, 4);
  const rawNote = noteRange.getNotes()[0][0];
  let stored = reportNote(rawNote);
  if (!stored || stored.assignmentId !== payload.assignmentId || range.getFormulas()[0].some(Boolean)) throw new Error('Không đọc lại được danh tính hàng vừa ghi');
  if (payload.rowKind === 'canonical' && values[3] !== payload.postUrl) throw new Error('Canonical Link khác hàng đã lưu');
  // A retry can encounter a row from an older client. Upgrade only the verified
  // note; the delivery function already checked its complete visible identity.
  if (stored.deliveryVersion !== 2 || (payload.rowKind === 'canonical' && stored.canonicalDeliveryRevision === undefined)) {
    const upgraded = deliveryNote(payload, rawNote, values);
    commitSheetBatch(sheet, { extra: 0 }, row, [{ updateCells: { start: { sheetId: sheet.getSheetId(), rowIndex: row - 1, columnIndex: 3 }, rows: [{ values: [{ note: upgraded }] }], fields: 'note' } }]);
    stored = reportNote(noteRange.getNotes()[0][0]);
    values = range.getDisplayValues()[0];
  }
  if (!stored || stored.deliveryVersion !== 2 || stored.spreadsheetId !== sheet.getParent().getId()
    || stored.sheetGid !== sheet.getSheetId() || range.getFormulas()[0].some(Boolean)
    || !rowFingerprintMatches(values, stored.rowFingerprint)) throw new Error('Dữ liệu đọc lại không khớp hàng đã commit');
  const revision = payload.rowKind === 'canonical' ? stored.canonicalDeliveryRevision : stored.rowRevision;
  if (!Number.isSafeInteger(revision) || revision < 0) throw new Error('Hàng đã lưu thiếu deliveryRevision');
  return Object.assign({}, result, { deliveryVersion: stored.deliveryVersion, spreadsheetId: stored.spreadsheetId,
    sheetGid: stored.sheetGid, assignmentId: stored.assignmentId, deliveryRevision: revision, postUrl: values[3] });
}

/** Exact compact header recognition; a damaged compact header never falls back to K+. */
function compactLayout(sheet) {
  if (!['auto', 'compact', 'internal', 'legacy'].includes(CONFIG.LAYOUT_MODE)) {
    throw new Error('LAYOUT_MODE phải là auto, compact, internal hoặc legacy');
  }
  const headers = sheet.getRange(1, 1, 1, sheet.getMaxColumns()).getDisplayValues()[0].map(String);
  const expected = ['STT', 'Người air', 'Ngày', 'Link'];
  const metadata = ['Máy', 'Tài khoản TikTok', 'Trạng thái', 'Lỗi hoặc ghi chú'];
  const internal = metadata.every((value, index) => headers[index + 4] && headers[index + 4].trim() === value);
  const resemblesInternal = metadata.some((value, index) => headers[index + 4] && headers[index + 4].trim() === value);
  if (resemblesInternal && !internal) throw new Error('Bốn cột báo cáo nội bộ phải đúng tên và thứ tự');
  if ((internal && ['legacy', 'compact'].includes(CONFIG.LAYOUT_MODE))
    || (!internal && CONFIG.LAYOUT_MODE === 'internal')) {
    throw new Error('LAYOUT_MODE không khớp header báo cáo nội bộ');
  }
  if (CONFIG.LAYOUT_MODE === 'legacy') return null;
  const exact = expected.every((value, index) => headers[index].trim() === value);
  const resemblesCompact = (headers[1] && headers[1].trim() === 'Người air')
    || (headers[4] && headers[4].trim() === 'Đối tác');
  if (!exact) {
    if (CONFIG.LAYOUT_MODE === 'compact' || resemblesCompact || internal) {
      throw new Error('Header cần đúng STT, Người air, Ngày, Link');
    }
    return null;
  }
  if (internal && (CONFIG.LINK_COLUMN !== 4 || CONFIG.POSTER_COLUMN !== 2)) {
    throw new Error('Mẫu nội bộ cần LINK_COLUMN=4 và POSTER_COLUMN=2');
  }
  const partnerStart = internal ? 8 : 4;
  let partners = 0;
  while (headers[partnerStart + partners] !== undefined) {
    const expectedPartner = partners === 0 ? 'Đối tác' : 'Đối tác ' + (partners + 1);
    if (headers[partnerStart + partners].trim() !== expectedPartner) break;
    partners++;
  }
  if (!partners) {
    throw new Error('Sau Link cần Đối tác, Đối tác 2... liên tiếp, mỗi đối tác một cột');
  }
  // Các cột sau khối đối tác thuộc người dùng. Chỉ đọc/ghi tới đối tác cuối,
  // không sửa header, nội dung, ghi chú hoặc công thức ở phần còn lại.
  return { width: partnerStart + partners, partners: partners, internal: internal };
}

function compactDate(sheet, postedAt) {
  if (typeof postedAt !== 'string' || !/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2})$/.test(postedAt)) {
    throw new Error('Mẫu compact cần postedAt ISO từ thời điểm gửi bài đã lưu trong app');
  }
  const date = new Date(postedAt);
  if (!Number.isFinite(date.getTime())) throw new Error('postedAt không hợp lệ');
  const parts = postedAt.match(/^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2}):(\d{2})/).slice(1).map(Number);
  if (parts[1] < 1 || parts[1] > 12 || parts[2] < 1
    || parts[2] > new Date(Date.UTC(parts[0], parts[1], 0)).getUTCDate()
    || parts[3] > 23 || parts[4] > 59 || parts[5] > 59) {
    throw new Error('postedAt có ngày giờ không tồn tại');
  }
  return Utilities.formatDate(date, sheet.getParent().getSpreadsheetTimeZone(), 'd/M/yyyy');
}

function reportFingerprint(values) {
  return Utilities.computeDigest(Utilities.DigestAlgorithm.SHA_256,
    JSON.stringify(values), Utilities.Charset.UTF_8)
    .map(byte => ('0' + ((byte + 256) % 256).toString(16)).slice(-2)).join('');
}

function reportNote(text) {
  if (text.startsWith('riviu-publish:v1:')) {
    return { legacy: true, assignmentId: text.slice('riviu-publish:v1:'.length) };
  }
  try {
    const note = JSON.parse(text);
    return note && note.kind === 'riviu-publish' && (note.reportVersion === 1 || note.deliveryVersion === 2)
      && typeof note.assignmentId === 'string' ? note : null;
  } catch (_) {
    return null;
  }
}

function rowFingerprintMatches(values, expected) {
  const candidate = values.slice();
  while (candidate.length >= 5) {
    if (reportFingerprint(candidate) === expected) return true;
    if (candidate[candidate.length - 1] !== '') break;
    candidate.pop();
  }
  return false;
}

function payloadFingerprintMatches(poster, postedAt, postUrl, metadata, partners, expected) {
  const candidate = partners.slice();
  while (candidate.length >= 1) {
    if (reportFingerprint([poster, postedAt, postUrl, metadata, candidate]) === expected) return true;
    if (candidate[candidate.length - 1] !== '') break;
    candidate.pop();
  }
  return false;
}

function deliverInternal(sheet, layout, payload, postUrl, assignmentId) {
  if (typeof Sheets === 'undefined' || !Sheets.Spreadsheets || !Sheets.Spreadsheets.batchUpdate) {
    throw new Error('Bật dịch vụ Google Sheets API trước khi gửi báo cáo nội bộ');
  }
  if (!Array.isArray(payload.partners) || payload.partners.some(value => typeof value !== 'string')
    || payload.partners.length > layout.partners) {
    throw new Error('Danh sách đối tác không hợp lệ hoặc Sheet thiếu cột; không cắt bỏ tên');
  }
  const versioned = payload.rowKind === 'internalReport'
    || payload.reportVersion !== undefined || payload.rowRevision !== undefined;
  if (versioned && (!['internalReport', 'canonical'].includes(payload.rowKind)
    || payload.reportVersion !== 1 || !Number.isSafeInteger(payload.rowRevision) || payload.rowRevision < 0)) {
    throw new Error('Báo cáo cần rowKind, reportVersion=1 và rowRevision nguyên an toàn không âm');
  }
  const statuses = ['Chưa đăng', 'Đã gửi', 'Đã xác minh', 'Cần kiểm tra'];
  if (versioned && (!statuses.includes(payload.status)
    || ['machine', 'tiktokAccount', 'stateNotes'].some(key => typeof payload[key] !== 'string'))) {
    throw new Error('Thiếu thông tin máy, tài khoản, trạng thái hoặc ghi chú báo cáo');
  }
  if (versioned && payload.rowKind === 'canonical' && payload.status !== 'Đã xác minh') {
    throw new Error('Hàng canonical chỉ dành cho bài đã xác minh');
  }
  if (payload.rowKind === 'canonical' && payload.postedAt === null) throw new Error('Hàng canonical cần thời điểm gửi bài đã lưu');
  if (postUrl && !/^https:\/\/(?:www\.)?tiktok\.com\/@[^/?#]+\/(?:photo|video)\/\d+$/.test(postUrl)) {
    throw new Error('Link báo cáo phải là canonical TikTok HTTPS');
  }
  if (versioned && ((payload.status === 'Đã xác minh') !== Boolean(postUrl))) {
    throw new Error('Chỉ trạng thái Đã xác minh được có canonical link');
  }
  const date = versioned && payload.postedAt === null ? '' : compactDate(sheet, payload.postedAt);
  const poster = String(payload.poster || 'bot');
  const partners = Array.from({ length: layout.partners }, (_, index) => payload.partners[index] || '');
  const metadata = versioned ? [payload.machine, payload.tiktokAccount, payload.status, payload.stateNotes] : ['', '', '', ''];
  const expected = [poster, date, postUrl].concat(metadata, partners);
  const payloadFingerprint = versioned ? reportFingerprint([
    poster, payload.postedAt, postUrl, metadata, partners,
  ]) : null;
  const height = Math.max(0, sheet.getLastRow() - CONFIG.FIRST_DATA_ROW + 1);
  const values = ownedRows(sheet, layout, height, 'getDisplayValues');
  const formulas = ownedRows(sheet, layout, height, 'getFormulas');
  const rawNotes = height ? sheet.getRange(CONFIG.FIRST_DATA_ROW, 4, height, 1).getNotes().map(row => row[0]) : [];
  const notes = rawNotes.map(reportNote);
  const keyed = notes.map((note, index) => note && note.assignmentId === assignmentId ? index : -1).filter(index => index >= 0);
  if (keyed.length > 1) throw new Error('Nhiều dòng cùng assignmentId; kiểm tra trước khi gửi');
  let index = keyed.length ? keyed[0] : -1;
  let stored = index >= 0 ? notes[index] : null;
  if (stored) validateStoredDelivery(stored, payload, values[index], sheet);
  const validRow = at => /^[1-9]\d*$/.test(values[at][0]) && Number.isSafeInteger(Number(values[at][0]))
    && formulas[at].every(value => !value);
  const sameIdentity = (at, maySetDate) => values[at][1] === poster
    && (values[at][2] === date || (maySetDate && values[at][2] === ''))
    && partners.every((partner, offset) => values[at][8 + offset] === partner);
  const ack = (revision, duplicate) => {
    const result = { ok: true, reportVersion: 1, assignmentId: assignmentId, rowRevision: revision, row: CONFIG.FIRST_DATA_ROW + index };
    if (duplicate) result.duplicate = true;
    return result;
  };
  if (stored && !stored.legacy && stored.reportVersion === 1) {
    if (!validRow(index) || !Number.isSafeInteger(stored.rowRevision) || stored.rowRevision < 0
      || !rowFingerprintMatches(values[index], stored.rowFingerprint)) {
      throw new Error('Dòng báo cáo đã bị sửa; kiểm tra trước khi ghi đè');
    }
    if (versioned && payload.rowRevision < stored.rowRevision) return ack(stored.rowRevision, true);
    if (versioned && payload.rowRevision === stored.rowRevision) {
      if (!payloadFingerprintMatches(poster, payload.postedAt, postUrl, metadata, partners, stored.payloadFingerprint)) throw new Error('Cùng revision nhưng nội dung khác');
      return ack(stored.rowRevision, true);
    }
    if (versioned && stored.postedAt !== null && stored.postedAt !== payload.postedAt) {
      throw new Error('Thời điểm gửi bài đã lưu không được thay đổi khi cập nhật');
    }
  }
  if (index < 0 && postUrl) {
    const linked = values.map((row, at) => row[3] === postUrl ? at : -1).filter(at => at >= 0);
    if (linked.length > 1) throw new Error('Link đã có ở nhiều dòng');
    if (linked.length) {
      index = linked[0];
      if (rawNotes[index] || !validRow(index) || !sameIdentity(index, false)
        || values[index].slice(4, 8).some((value, offset) => value !== '' && value !== metadata[offset])) {
        throw new Error('Dòng có link chưa khớp danh tính hoặc đang có ghi chú khác');
      }
    }
  }
  if (index >= 0) {
    if (!validRow(index) || !sameIdentity(index, Boolean(stored && !stored.legacy))
      || (values[index][3] && values[index][3] !== postUrl)) {
      throw new Error('Người đăng, ngày, link hoặc đối tác khác dòng đã ghi');
    }
    if (stored && stored.legacy && versioned && values[index].slice(4, 8).some(value => value !== '')) {
      throw new Error('Dòng v1 đã có thông tin nội bộ; cần kiểm tra trước khi nhận quản lý');
    }
    if (!versioned) {
      // A delayed classic outbox request may complete its known Link, but cannot
      // erase metadata, downgrade status, or invent a versioned acknowledgement.
      expected.splice(3, 4, ...values[index].slice(4, 8));
      if (values[index][3] === postUrl && stored) {
        return { ok: true, duplicate: true, row: CONFIG.FIRST_DATA_ROW + index };
      }
    }
  } else {
    // Blank pending links have no identity: never adopt a partly filled unkeyed row.
    index = values.findIndex((row, at) => row.every(value => value === '')
      && !rawNotes[at] && formulas[at].every(value => !value));
    if (index < 0) index = height;
  }
  const existingNumber = values[index] && values[index][0];
  const number = existingNumber ? Number(existingNumber) : values.reduce((max, row) =>
    /^[1-9]\d*$/.test(row[0]) ? Math.max(max, Number(row[0])) : max, 0) + 1;
  if (!Number.isSafeInteger(number)) throw new Error('STT vượt giới hạn số nguyên');
  const display = [String(number)].concat(expected);
  let note;
  if (versioned) {
    note = JSON.stringify(Object.assign({}, stored && !stored.legacy ? stored : {}, { kind: 'riviu-publish', reportVersion: 1, assignmentId: assignmentId,
      rowRevision: payload.rowRevision, rowFingerprint: reportFingerprint(display),
      payloadFingerprint: payloadFingerprint, postedAt: payload.postedAt }));
  } else if (stored && !stored.legacy) {
    note = JSON.stringify(Object.assign({}, stored, { rowFingerprint: reportFingerprint(display) }));
  } else {
    note = 'riviu-publish:v1:' + assignmentId;
  }
  note = deliveryNote(payload, note, display);
  const row = CONFIG.FIRST_DATA_ROW + index;
  commitSheetBatch(sheet, layout, row, [
    { updateCells: { start: { sheetId: sheet.getSheetId(), rowIndex: row - 1, columnIndex: 0 },
      rows: [{ values: [{ userEnteredValue: { numberValue: number } }].concat(
        expected.map(value => ({ userEnteredValue: { stringValue: value } }))
      ) }], fields: 'userEnteredValue' } },
    { updateCells: { start: { sheetId: sheet.getSheetId(), rowIndex: row - 1, columnIndex: 3 },
      rows: [{ values: [{ note: note }] }], fields: 'note' } },
  ]);
  return versioned ? ack(payload.rowRevision, false) : { ok: true, row: row };
}

function deliverCompact(sheet, layout, payload, postUrl, assignmentId) {
  if (typeof Sheets === 'undefined' || !Sheets.Spreadsheets || !Sheets.Spreadsheets.batchUpdate) {
    throw new Error('Bật dịch vụ Google Sheets API trong Apps Script trước khi dùng mẫu compact');
  }
  if (!Array.isArray(payload.partners) || payload.partners.some(value => typeof value !== 'string')) {
    throw new Error('partners phải là danh sách tên đối tác');
  }
  if (payload.partners.length > layout.partners) {
    throw new Error('Sheet thiếu cột đối tác; thêm đúng header trước khi thử lại, không cắt bỏ tên');
  }
  const date = compactDate(sheet, payload.postedAt);
  const poster = String(payload.poster || 'bot');
  const partners = Array.from({ length: layout.partners }, (_, index) => payload.partners[index] || '');
  const expected = [poster, date, postUrl].concat(partners);
  const note = 'riviu-publish:v1:' + assignmentId;
  const height = Math.max(0, sheet.getLastRow() - CONFIG.FIRST_DATA_ROW + 1);
  const values = ownedRows(sheet, layout, height, 'getDisplayValues');
  const formulas = ownedRows(sheet, layout, height, 'getFormulas');
  const notes = height ? sheet.getRange(CONFIG.FIRST_DATA_ROW, 4, height, 1).getNotes() : [];
  const same = index => expected.every((value, column) => String(values[index][column + 1]) === value)
    && /^[1-9]\d*$/.test(String(values[index][0]))
    && formulas[index].every(value => !value);
  const keyed = notes.map((value, index) => reportNote(value[0])?.assignmentId === assignmentId ? index : -1).filter(index => index >= 0);
  if (keyed.length) {
    if (keyed.length !== 1 || !same(keyed[0])) {
      throw new Error('Dòng đã ghi bị thay đổi hoặc khoá trùng; kiểm tra trước khi gửi lại');
    }
    validateStoredDelivery(reportNote(notes[keyed[0]][0]), payload, values[keyed[0]], sheet);
    return { ok: true, duplicate: true, row: CONFIG.FIRST_DATA_ROW + keyed[0] };
  }
  const linked = values.map((value, index) => value[3] === postUrl ? index : -1).filter(index => index >= 0);
  let index = -1;
  if (linked.length) {
    if (linked.length !== 1 || !same(linked[0]) || notes[linked[0]][0]) {
      throw new Error('Link đã có nhưng dữ liệu hoặc ghi chú khác; không thêm dòng trùng');
    }
    index = linked[0];
  } else {
    // Match an existing pending row only when all visible identity fields agree.
    const pending = values.map((value, at) => value[3] === '' && value[1] === poster && value[2] === date
      && /^[1-9]\d*$/.test(String(value[0])) && partners.every((partner, p) => value[p + 4] === partner)
      && !notes[at][0] && formulas[at].every(value => !value) ? at : -1).filter(at => at >= 0);
    if (pending.length > 1) throw new Error('Nhiều dòng chờ giống nhau; cần xác định đúng dòng trước');
    if (pending.length === 1) index = pending[0];
  }
  let number;
  if (index >= 0) {
    number = Number(values[index][0]);
  } else {
    number = values.reduce((max, row) => /^[1-9]\d*$/.test(String(row[0])) ? Math.max(max, Number(row[0])) : max, 0) + 1;
    if (!Number.isSafeInteger(number)) throw new Error('STT vượt giới hạn số nguyên');
    index = values.findIndex((row, at) => row.every(value => value === '')
      && !notes[at][0] && formulas[at].every(value => !value));
    if (index < 0) index = height;
  }
  const row = CONFIG.FIRST_DATA_ROW + index;
  const start = { sheetId: sheet.getSheetId(), rowIndex: row - 1, columnIndex: 0 };
  // Explicit stringValue prevents formulas; both requests commit together, including
  // the note key. No PropertiesService/hidden-sheet second-write window exists.
  commitSheetBatch(sheet, layout, row, [
    { updateCells: { start: start, rows: [{ values: [{ userEnteredValue: { numberValue: number } }].concat(
      expected.map(value => ({ userEnteredValue: { stringValue: value } }))
    ) }], fields: 'userEnteredValue' } },
    { updateCells: { start: { sheetId: start.sheetId, rowIndex: start.rowIndex, columnIndex: 3 },
      rows: [{ values: [{ note: deliveryNote(payload, note, [String(number)].concat(expected)) }] }], fields: 'note' } },
  ]);
  return { ok: true, row: row };
}

/**
 * Ba con số cột phụ thuộc nhau, và khi chúng lệch thì hậu quả là **dữ liệu sai**, không
 * phải một lỗi ai đó nhìn thấy.
 *
 * Đo được 31/08/2026: `KEY_COLUMN: 26` đặt khoá vào cột Z, và trên sheet thật Z là
 * `Đối tác 16` — mỗi hàng ghi ra là một tên đối tác bị đè. Không có gì trong script cũ
 * nhận ra, vì mỗi con số một mình đều hợp lệ.
 *
 * Trả chuỗi lý do khi cấu hình không dùng được, `''` khi dùng được. Chạy trước cả khoá, để
 * một cấu hình hỏng không kịp chạm vào sheet.
 */
function assertConfigIsSane() {
  const first = CONFIG.PARTNERS_START_COLUMN;
  const last = CONFIG.PARTNERS_START_COLUMN + CONFIG.PARTNERS_MAX - 1;
  const inside = (column) => column >= first && column <= last;

  if (CONFIG.PARTNERS_MAX < 1) {
    return 'PARTNERS_MAX phải >= 1';
  }
  if (inside(CONFIG.KEY_COLUMN)) {
    return (
      'KEY_COLUMN ' + CONFIG.KEY_COLUMN + ' nằm TRONG khối đối tác (' + first + '..' + last +
      ') — ghi khoá vào đó là đè mất một tên đối tác. Đổi KEY_COLUMN sang cột trống sau ' + last
    );
  }
  if (inside(CONFIG.LINK_COLUMN)) {
    return 'LINK_COLUMN ' + CONFIG.LINK_COLUMN + ' nằm trong khối đối tác';
  }
  if (inside(CONFIG.POSTER_COLUMN)) {
    return 'POSTER_COLUMN ' + CONFIG.POSTER_COLUMN + ' nằm trong khối đối tác';
  }
  if (CONFIG.KEY_COLUMN === CONFIG.LINK_COLUMN || CONFIG.KEY_COLUMN === CONFIG.POSTER_COLUMN) {
    return 'KEY_COLUMN trùng với LINK_COLUMN hoặc POSTER_COLUMN';
  }
  return '';
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
        postedAt: new Date().toISOString(),
      }),
    },
  });
  Logger.log(out.getContent());
}
