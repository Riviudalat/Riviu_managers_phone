import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import test from "node:test";
import vm from "node:vm";
import { createHash } from "node:crypto";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const source = fs.readFileSync(process.env.RIVIU_SHEET_FIXTURE_PATH || path.join(root, "docs/apps-script/publish-sheet.gs"), "utf8");

// Stateful Google Sheets fixture. A batch is validated before applying any write;
// an optional lost response is raised after the complete write to model retry.
function harness({ config = {}, headers = ["STT", "Người air", "Ngày", "Link", "Đối tác", "Đối tác 2"], rows = [], gid = 0 } = {}) {
  const cells = (headers.length ? [headers, ...rows] : rows).map(row => [...row]);
  const notes = new Map();
  const formulas = new Map();
  const state = { writes: 0, batches: [], lostResponse: false, rejectBatch: false, locks: 0, afterCommit: null, frozenRows: 0 };
  let maxRows = 20;
  let maxColumns = Math.max(1, headers.length, ...rows.map(row => row.length));
  const read = (r, c) => cells[r - 1]?.[c - 1] ?? "";
  const sheet = {
    getSheetId: () => gid,
    getParent: () => book,
    getLastRow: () => cells.length,
    getMaxRows: () => maxRows,
    getMaxColumns: () => maxColumns,
    getLastColumn: () => Math.max(0, ...cells.map(row => row.length)),
    insertRowsAfter: () => { throw new Error("row expansion must be inside the atomic batch"); },
    insertColumnsAfter: () => { throw new Error("column expansion must be inside the atomic batch"); },
    getRange(row, column, height = 1, width = 1) {
      assert.ok(row >= 1 && column >= 1 && column + width - 1 <= maxColumns);
      const matrix = fn => Array.from({ length: height }, (_, y) => Array.from({ length: width }, (_, x) => fn(row + y, column + x)));
      return {
        getValues: () => matrix(read),
        getDisplayValues: () => matrix((r, c) => String(read(r, c))),
        getFormulas: () => matrix((r, c) => formulas.get(`${r}:${c}`) ?? ""),
        getNotes: () => matrix((r, c) => notes.get(`${r}:${c}`) ?? ""),
        setValues(values) {
          for (let y = 0; y < height; y++) {
            cells[row + y - 1] ??= [];
            for (let x = 0; x < width; x++) cells[row + y - 1][column + x - 1] = values[y][x];
          }
          state.writes++;
        },
      };
    },
  };
  const book = { getId: () => "fixture-book", getSheets: () => [sheet], getSheetByName: () => sheet, getSpreadsheetTimeZone: () => "Asia/Ho_Chi_Minh" };
  const context = vm.createContext({
    SpreadsheetApp: { openById: () => book },
    LockService: { getScriptLock: () => ({ waitLock: () => { state.locks++; }, releaseLock: () => { state.locks--; } }) },
    ContentService: { MimeType: { JSON: "json" }, createTextOutput: text => ({ setMimeType: () => ({ getContent: () => text }) }) },
    Utilities: {
      formatDate: date => new Intl.DateTimeFormat("en-GB", { timeZone: "Asia/Ho_Chi_Minh", day: "numeric", month: "numeric", year: "numeric" }).format(date).split("/").map(Number).join("/"),
      DigestAlgorithm: { SHA_256: "sha256" }, Charset: { UTF_8: "utf8" },
      computeDigest: (_algorithm, value) => [...createHash("sha256").update(value, "utf8").digest()],
    },
    Sheets: { Spreadsheets: { batchUpdate(batch) {
      assert.equal(state.locks, 1, "each atomic row write must hold the shared script lock");
      state.batches.push(batch);
      if (state.rejectBatch) throw new Error("fixture batch rejected before commit");
      let nextColumns = maxColumns, nextRows = maxRows;
      for (const request of batch.requests) {
        if (request.insertDimension) {
          const range = request.insertDimension.range;
          assert.equal(range.sheetId, gid); assert.equal(range.dimension, "COLUMNS");
          assert.ok(range.startIndex <= nextColumns && range.endIndex > range.startIndex);
          nextColumns += range.endIndex - range.startIndex;
          continue;
        }
        if (request.appendDimension) {
          assert.equal(request.appendDimension.sheetId, gid);
          if (request.appendDimension.dimension === "ROWS") nextRows += request.appendDimension.length;
          else { assert.equal(request.appendDimension.dimension, "COLUMNS"); nextColumns += request.appendDimension.length; }
          continue;
        }
        if (request.updateSheetProperties) { assert.equal(request.updateSheetProperties.properties.sheetId, gid); continue; }
        const update = request.updateCells;
        assert.ok(update, "only validated grid or cell writes expected");
        assert.equal(update.start.sheetId, gid);
        const r = update.start.rowIndex + 1;
        const c = update.start.columnIndex + 1;
        assert.ok(r <= nextRows && c + update.rows[0].values.length - 1 <= nextColumns);
      }
      for (const request of batch.requests) {
        if (request.insertDimension) {
          const { startIndex, endIndex } = request.insertDimension.range;
          const extra = endIndex - startIndex;
          for (const row of cells) row.splice(startIndex, 0, ...Array(extra).fill(""));
          for (const map of [notes, formulas]) {
            const entries = [...map]; map.clear();
            for (const [key, value] of entries) {
              const [r, c] = key.split(":").map(Number);
              map.set(`${r}:${c > startIndex ? c + extra : c}`, value);
            }
          }
          maxColumns += extra;
          continue;
        }
        if (request.appendDimension) {
          if (request.appendDimension.dimension === "ROWS") maxRows += request.appendDimension.length;
          else maxColumns += request.appendDimension.length;
          continue;
        }
        if (request.updateSheetProperties) { state.frozenRows = request.updateSheetProperties.properties.gridProperties.frozenRowCount; continue; }
        const update = request.updateCells;
        update.rows.forEach((row, y) => row.values.forEach((cell, x) => {
          const r = update.start.rowIndex + y + 1;
          const c = update.start.columnIndex + x + 1;
          cells[r - 1] ??= [];
          if (update.fields.includes("userEnteredValue")) cells[r - 1][c - 1] = cell.userEnteredValue?.stringValue ?? cell.userEnteredValue?.numberValue ?? "";
          if (update.fields.includes("note")) notes.set(`${r}:${c}`, cell.note ?? "");
        }));
      }
      state.writes++;
      state.afterCommit?.();
      if (state.lostResponse) { state.lostResponse = false; throw new Error("fixture response lost after commit"); }
      return {};
    } } },
    Logger: { log() {} },
  });
  vm.runInContext(source + "\nObject.assign(CONFIG, " + JSON.stringify({ SPREADSHEET_ID: "fixture-book", TOKEN: "fixture-token", SHEET_GID: gid, ...config }) + ");", context);
  const deliver = overrides => {
    const payload = { token: "fixture-token", assignmentId: "assignment-1", postUrl: "https://www.tiktok.com/@test/photo/123456", poster: "bot", partners: ["Quán A", "Quán B"], postedAt: "2026-09-08T18:30:00Z", ...overrides };
    return JSON.parse(context.doPost({ postData: { contents: JSON.stringify(payload) } }).getContent());
  };
  return { cells, notes, formulas, state, deliver, context };
}

test("compact headers write exactly requested visible values and no extra key column", () => {
  const h = harness();
  assert.equal(h.deliver().ok, true);
  assert.deepEqual(h.cells[1], [1, "bot", "9/9/2026", "https://www.tiktok.com/@test/photo/123456", "Quán A", "Quán B"]);
  assert.equal(h.state.batches.length, 1);
  assert.match(h.notes.get("2:4"), /assignment-1/);
  assert.equal(h.state.locks, 0);
});

test("lost response after atomic write retries as one row", () => {
  const h = harness();
  h.state.lostResponse = true;
  assert.equal(h.deliver().ok, false);
  assert.equal(h.cells.length, 2);
  assert.deepEqual(h.deliver(), { ok: true, duplicate: true, row: 2 });
  assert.equal(h.state.writes, 1);
  assert.equal(h.state.locks, 0);
});

test("rejected batch has neither visible row nor idempotency key", () => {
  const h = harness();
  h.state.rejectBatch = true;
  assert.equal(h.deliver().ok, false);
  assert.equal(h.cells.length, 1);
  assert.equal(h.notes.size, 0);
});

test("compact partner overflow expands headers and data in one commit", () => {
  const h = harness();
  assert.equal(h.deliver({ partners: ["A", "B", "C"] }).ok, true);
  assert.deepEqual(h.cells[0].slice(4), ["Đối tác", "Đối tác 2", "Đối tác 3"]);
  assert.deepEqual(h.cells[1].slice(4), ["A", "B", "C"]);
  assert.equal(h.state.writes, 1);
});

test("same assignment with changed content is not acknowledged as delivered", () => {
  const h = harness();
  assert.equal(h.deliver().ok, true);
  assert.equal(h.deliver({ postUrl: "https://www.tiktok.com/@test/photo/654321" }).ok, false);
  assert.equal(h.state.writes, 1);
});

test("compact posting date is a required immutable value", () => {
  for (const invalid of [{ postedAt: undefined }, { postedAt: "bad-date" }, { postedAt: "2026-02-30T00:00:00Z" }]) {
    const h = harness();
    assert.equal(h.deliver(invalid).ok, false);
    assert.equal(h.state.writes, 0);
  }
});

test("partner formula-looking text is written as literal stringValue", () => {
  const h = harness();
  assert.equal(h.deliver({ partners: ["=1+1", "@shop"] }).ok, true);
  assert.deepEqual(h.cells[1].slice(4), ["=1+1", "@shop"]);
});

test("compact writes preserve user trailing note headers, values and formulas", () => {
  const h = harness({
    headers: ["STT", "Người air", "Ngày", "Link", "Đối tác", "Đối tác 2", "Post is being processed", "Tổng"],
    rows: [["", "", "", "", "", "", "Ghi chú của tôi", 42]],
  });
  h.formulas.set("2:8", "=40+2");
  h.notes.set("2:7", "Ghi chú riêng");
  assert.equal(h.deliver().ok, true);
  assert.deepEqual(h.cells[1], [1, "bot", "9/9/2026", "https://www.tiktok.com/@test/photo/123456", "Quán A", "Quán B", "Ghi chú của tôi", 42]);
  assert.equal(h.cells[0][6], "Post is being processed");
  assert.equal(h.formulas.get("2:8"), "=40+2");
  assert.equal(h.notes.get("2:7"), "Ghi chú riêng");
  for (const batch of h.state.batches) {
    for (const { updateCells } of batch.requests) {
      assert.ok(updateCells.start.columnIndex + updateCells.rows[0].values.length <= 6);
    }
  }
  assert.equal(h.deliver().duplicate, true);
  assert.equal(h.state.writes, 1);
});

test("header and existing row changes fail without overwriting", () => {
  const h = harness();
  assert.equal(h.deliver().ok, true);
  h.cells[1][4] = "Người sửa";
  assert.equal(h.deliver().ok, false);
  assert.equal(h.state.writes, 1);
  const changed = harness({ headers: ["STT", "Người air", "Ngày sai", "Link", "Đối tác", "Đối tác 2"] });
  assert.equal(changed.deliver().ok, false);
  assert.equal(changed.state.writes, 0);
});

test("manual matching row adopts key and pending row fills exact blank Link", () => {
  for (const link of ["", "https://www.tiktok.com/@test/photo/123456"]) {
    const h = harness({ rows: [[8, "bot", "9/9/2026", link, "Quán A", "Quán B"]] });
    assert.equal(h.deliver().ok, true);
    assert.equal(h.cells.length, 2);
    assert.equal(h.cells[1][0], 8);
    assert.equal(h.cells[1][3], "https://www.tiktok.com/@test/photo/123456");
    assert.equal(h.deliver().duplicate, true);
  }
});

test("new STT follows maximum and does not claim an unrelated pending row", () => {
  const h = harness({ rows: [[9, "Phát", "3/9/2026", "", "Khác", ""]] });
  assert.equal(h.deliver().ok, true);
  assert.equal(h.cells[1][3], "");
  assert.equal(h.cells[2][0], 10);
  assert.equal(h.cells[2][2], "9/9/2026");
});

test("formula-owned blank Link and foreign note are not overwritten", () => {
  const h = harness({ rows: [[1, "bot", "9/9/2026", "", "Quán A", "Quán B"]] });
  h.formulas.set("2:4", '=IF(A2="","","")');
  assert.equal(h.deliver().ok, true);
  assert.equal(h.cells[1][3], "");
  assert.equal(h.cells[2][0], 2);
  const other = harness({ rows: [[1, "bot", "9/9/2026", "https://www.tiktok.com/@test/photo/123456", "Quán A", "Quán B"]] });
  other.notes.set("2:4", "manual note");
  assert.equal(other.deliver().ok, false);
  assert.equal(other.state.writes, 0);
});

test("missing advanced Sheets service rejects before any writes", () => {
  const h = harness();
  vm.runInContext("Sheets = undefined", h.context);
  assert.equal(h.deliver().ok, false);
  assert.equal(h.state.writes, 0);
});

test("legacy K through AG and AH key layout still works without postedAt", () => {
  const headers = Array(34).fill("");
  headers[1] = "Nhân Viên";
  headers[3] = "Link";
  for (let i = 0; i < 23; i++) headers[10 + i] = "Đối tác " + (i + 1);
  const h = harness({ headers });
  assert.equal(h.deliver({ postedAt: undefined }).ok, true);
  assert.equal(h.cells[1][1], "bot");
  assert.equal(h.cells[1][10], "Quán A");
  assert.equal(h.cells[1][11], "Quán B");
  assert.equal(h.cells[1][33], "assignment-1");
  assert.equal(h.state.batches.length, 0);
  assert.equal(h.deliver({ postedAt: undefined }).duplicate, true);
  assert.equal(h.deliver({ postedAt: undefined, partners: ["changed"] }).ok, false);
});

test("legacy non-owned column formulas survive the row write", () => {
  const h = harness({ headers: Array(34).fill(""), rows: [Array(34).fill("")], config: { LAYOUT_MODE: "legacy" } });
  h.formulas.set("2:5", "=1+1");
  h.cells[1][4] = 2;
  assert.equal(h.deliver({ postedAt: undefined }).ok, true);
  assert.equal(h.cells[1][4], "=1+1");
});

test("gid zero does not fall back to the first mismatched tab", () => {
  const h = harness({ gid: 123, config: { SHEET_GID: 0 } });
  assert.equal(h.deliver().ok, false);
  assert.equal(h.state.writes, 0);
});

const internalHeaders = ["STT", "Người air", "Ngày", "Link", "Máy", "Tài khoản TikTok", "Trạng thái", "Lỗi hoặc ghi chú", "Đối tác", "Đối tác 2"];
const internalRow = { rowKind: "internalReport", reportVersion: 1, rowRevision: 1, postUrl: "", postedAt: null, machine: "Máy 18", tiktokAccount: "@test", status: "Chưa đăng", stateNotes: "Đang chuẩn bị" };

test("internal report writes four requested columns before every partner and accepts a blank date/link", () => {
  const h = harness({ headers: internalHeaders });
  assert.deepEqual(h.deliver(internalRow), { ok: true, reportVersion: 1, assignmentId: "assignment-1", rowRevision: 1, row: 2 });
  assert.deepEqual(h.cells[1], [1, "bot", "", "", "Máy 18", "@test", "Chưa đăng", "Đang chuẩn bị", "Quán A", "Quán B"]);
  const note = JSON.parse(h.notes.get("2:4"));
  assert.equal(note.assignmentId, "assignment-1");
  assert.equal(note.rowRevision, 1);
  assert.equal(h.state.writes, 1);
});

test("internal pending submitted and verified revisions update the same STT and row", () => {
  const h = harness({ headers: internalHeaders });
  assert.equal(h.deliver(internalRow).ok, true);
  assert.equal(h.deliver({ ...internalRow, rowRevision: 2, postedAt: "2026-09-08T18:30:00Z", status: "Đã gửi", stateNotes: "TikTok đang xử lý" }).ok, true);
  assert.equal(h.deliver({ ...internalRow, rowRevision: 3, postedAt: "2026-09-08T18:30:00Z", postUrl: "https://www.tiktok.com/@test/photo/123456", status: "Đã xác minh", stateNotes: "" }).ok, true);
  assert.equal(h.cells.length, 2);
  assert.equal(h.cells[1][0], 1);
  assert.equal(h.cells[1][2], "9/9/2026");
  assert.equal(h.cells[1][6], "Đã xác minh");
  assert.deepEqual(h.cells[1].slice(8), ["Quán A", "Quán B"]);
  assert.equal(h.deliver({ ...internalRow, rowRevision: 4, postedAt: "2026-09-08T19:30:00Z", postUrl: h.cells[1][3], status: "Đã xác minh" }).ok, false);
});

test("internal lost response and older revision acknowledge stored revision without another write", () => {
  const h = harness({ headers: internalHeaders });
  h.state.lostResponse = true;
  assert.equal(h.deliver({ ...internalRow, rowRevision: 5 }).ok, false);
  const duplicate = h.deliver({ ...internalRow, rowRevision: 5 });
  assert.equal(duplicate.ok, true);
  assert.equal(duplicate.rowRevision, 5);
  assert.equal(duplicate.duplicate, true);
  const older = h.deliver({ ...internalRow, rowRevision: 2, stateNotes: "Cũ" });
  assert.equal(older.rowRevision, 5);
  assert.equal(h.state.writes, 1);
  assert.equal(h.deliver({ ...internalRow, rowRevision: 5, stateNotes: "Xung đột" }).ok, false);
});

test("internal human cell edits and formulas block even an older acknowledgement", () => {
  for (const edit of [h => { h.cells[1][7] = "Người sửa"; }, h => h.formulas.set("2:5", '=IF(TRUE,"Máy 18","")')]) {
    const h = harness({ headers: internalHeaders });
    h.deliver({ ...internalRow, rowRevision: 4 });
    edit(h);
    assert.equal(h.deliver({ ...internalRow, rowRevision: 3 }).ok, false);
    assert.equal(h.deliver({ ...internalRow, rowRevision: 5 }).ok, false);
    assert.equal(h.state.writes, 1);
  }
});

test("internal input status revision date layout and link must match the declared contract", () => {
  for (const change of [
    { status: "Đã gửi, chờ xác minh" }, { status: "Đã xác minh" },
    { postUrl: "https://www.tiktok.com/@test/photo/123456" },
    { rowRevision: -1 }, { rowRevision: 1.1 }, { rowRevision: Number.MAX_SAFE_INTEGER + 1 },
    { reportVersion: 2 }, { stateNotes: null }, { postedAt: undefined }, { postedAt: "2026-02-30T00:00:00Z" },
  ]) {
    const h = harness({ headers: internalHeaders });
    assert.equal(h.deliver({ ...internalRow, ...change }).ok, false, JSON.stringify(change));
    assert.equal(h.state.writes, 0);
  }
  assert.equal(harness().deliver(internalRow).ok, false);
  for (const mode of ["compact", "legacy"]) {
    const h = harness({ headers: internalHeaders, config: { LAYOUT_MODE: mode } });
    assert.equal(h.deliver(internalRow).ok, false);
    assert.equal(h.deliver().ok, false);
    assert.equal(h.state.writes, 0);
  }
  const wrong = [...internalHeaders]; [wrong[4], wrong[5]] = [wrong[5], wrong[4]];
  assert.equal(harness({ headers: wrong }).deliver(internalRow).ok, false);
});

test("internal verified link is immutable even under a newer revision", () => {
  const h = harness({ headers: internalHeaders });
  const verified = { ...internalRow, postedAt: "2026-09-08T18:30:00Z", postUrl: "https://www.tiktok.com/@test/photo/123456", status: "Đã xác minh" };
  assert.equal(h.deliver(verified).ok, true);
  assert.equal(h.deliver({ ...verified, rowRevision: 2, postUrl: "https://www.tiktok.com/@test/photo/654321" }).ok, false);
  assert.equal(h.deliver({ ...internalRow, rowRevision: 2 }).ok, false);
  assert.equal(h.state.writes, 1);
});

test("internal literal fields full partner tail and trailing user formulas survive", () => {
  const headers = internalHeaders.concat(["Đối tác 3", "Tổng"]);
  const h = harness({ headers, rows: [["", "", "", "", "", "", "", "", "", "", "", 42]] });
  h.formulas.set("2:12", "=40+2");
  const row = { ...internalRow, machine: "=1+1", tiktokAccount: "@test", stateNotes: "+ghi chú", partners: ["=shop", "@shop", "Cuối"] };
  assert.equal(h.deliver(row).ok, true);
  assert.deepEqual(h.cells[1].slice(4), ["=1+1", "@test", "Chưa đăng", "+ghi chú", "=shop", "@shop", "Cuối", 42]);
  assert.equal(h.formulas.get("2:12"), "=40+2");
  assert.equal(h.deliver({ ...row, rowRevision: 2, partners: ["1", "2", "3", "4"] }).ok, false);
  assert.equal(h.state.writes, 1);
});

test("internal adopts a v1 key after metadata columns were inserted only if old identity still matches", () => {
  const values = [8, "bot", "9/9/2026", "https://www.tiktok.com/@test/photo/123456", "", "", "", "", "Quán A", "Quán B"];
  const h = harness({ headers: internalHeaders, rows: [values] });
  h.notes.set("2:4", "riviu-publish:v1:assignment-1");
  const row = { ...internalRow, rowRevision: 8, postedAt: "2026-09-08T18:30:00Z", postUrl: values[3], status: "Đã xác minh" };
  assert.equal(h.deliver(row).ok, true);
  assert.equal(h.cells[1][0], 8);
  assert.equal(JSON.parse(h.notes.get("2:4")).rowRevision, 8);
  const edited = harness({ headers: internalHeaders, rows: [values] });
  edited.notes.set("2:4", "riviu-publish:v1:assignment-1");
  edited.cells[1][4] = "Máy người nhập";
  assert.equal(edited.deliver(row).ok, false);
  assert.equal(edited.state.writes, 0);
});

test("internal never adopts an unkeyed blank pending link and only adopts an exact same-URL identity", () => {
  const h = harness({ headers: internalHeaders, rows: [[3, "bot", "", "", "Máy 18", "@test", "Chưa đăng", "Đang chuẩn bị", "Quán A", "Quán B"]] });
  assert.equal(h.deliver(internalRow).ok, true);
  assert.equal(h.cells.length, 3);
  assert.equal(h.cells[2][0], 4);
  const row = { ...internalRow, postedAt: "2026-09-08T18:30:00Z", postUrl: "https://www.tiktok.com/@test/photo/123456", status: "Đã xác minh" };
  const exact = harness({ headers: internalHeaders, rows: [[8, "bot", "9/9/2026", row.postUrl, "", "", "", "", "Quán A", "Quán B"]] });
  assert.equal(exact.deliver(row).ok, true);
  assert.equal(exact.cells.length, 2);
  const wrong = harness({ headers: internalHeaders, rows: [[8, "Người khác", "9/9/2026", row.postUrl, "", "", "", "", "Quán A", "Quán B"]] });
  assert.equal(wrong.deliver(row).ok, false);
});

test("classic outbox keeps internal metadata and versioned notes while canonical metadata updates normally", () => {
  const h = harness({ headers: internalHeaders });
  const posted = { ...internalRow, rowRevision: 7, status: "Đã gửi", postedAt: "2026-09-08T18:30:00Z" };
  assert.equal(h.deliver(posted).ok, true);
  const beforeMetadata = h.cells[1].slice(4, 8);
  const classic = h.deliver();
  assert.equal(classic.ok, true);
  assert.equal(classic.reportVersion, undefined);
  assert.deepEqual(h.cells[1].slice(4, 8), beforeMetadata);
  assert.equal(JSON.parse(h.notes.get("2:4")).rowRevision, 7);
  assert.equal(h.deliver({ ...posted, rowKind: "canonical", rowRevision: 8, postUrl: h.cells[1][3], status: "Đã xác minh", stateNotes: "" }).ok, true);
  assert.equal(h.cells[1][6], "Đã xác minh");
  assert.equal(h.deliver().duplicate, true);
  assert.equal(JSON.parse(h.notes.get("2:4")).rowRevision, 8);
});

test("classic delivery on internal headers leaves metadata blank and partners start at I", () => {
  const h = harness({ headers: internalHeaders });
  assert.equal(h.deliver().ok, true);
  assert.deepEqual(h.cells[1].slice(4), ["", "", "", "", "Quán A", "Quán B"]);
  assert.equal(h.notes.get("2:4"), "riviu-publish:v1:assignment-1");
  assert.equal(h.deliver().duplicate, true);
});

test("internal duplicate keys and precommit failures never produce partial status or notes", () => {
  const h = harness({ headers: internalHeaders });
  h.state.rejectBatch = true;
  assert.equal(h.deliver(internalRow).ok, false);
  assert.equal(h.cells.length, 1);
  assert.equal(h.notes.size, 0);
  h.state.rejectBatch = false;
  h.deliver(internalRow);
  h.cells.push([...h.cells[1]]);
  h.notes.set("3:4", h.notes.get("2:4"));
  assert.equal(h.deliver({ ...internalRow, rowRevision: 2 }).ok, false);
  assert.equal(h.state.writes, 1);
});

test("check returns the authenticated exact target and layout without any writes", () => {
  for (const [headers, layout] of [[internalHeaders, "internal"], [["STT", "Người air", "Ngày", "Link", "Đối tác"], "compact"]]) {
    const h = harness({ headers, gid: 37 });
    const before = JSON.stringify(h.cells);
    const result = h.deliver({ rowKind: "check", checkVersion: 1, spreadsheetId: "fixture-book", sheetGid: 37, assignmentId: undefined, postUrl: undefined });
    assert.deepEqual(result, { ok: true, checkVersion: 1, deliveryVersion: 2, spreadsheetId: "fixture-book", sheetGid: 37, layout, columns: headers });
    assert.equal(h.state.writes, 0); assert.equal(h.state.locks, 0); assert.equal(h.notes.size, 0);
    assert.equal(JSON.stringify(h.cells), before);
  }
});

test("check rejects wrong token, target, gid, version and damaged header without effects", () => {
  for (const patch of [{ token: "wrong" }, { spreadsheetId: "wrong" }, { sheetGid: 99 }, { checkVersion: 2 }]) {
    const h = harness();
    assert.equal(h.deliver({ rowKind: "check", checkVersion: 1, spreadsheetId: "fixture-book", sheetGid: 0, ...patch }).ok, false);
    assert.equal(h.state.writes, 0); assert.equal(h.state.locks, 0); assert.equal(h.notes.size, 0);
  }
  const h = harness({ headers: ["STT", "broken", "Ngày", "Link", "Đối tác"] });
  assert.equal(h.deliver({ rowKind: "check", checkVersion: 1, spreadsheetId: "fixture-book", sheetGid: 0 }).ok, false);
  assert.equal(h.state.writes, 0);
});


test("two scheduled machines finish in reverse order without mixing or duplicating their internal rows", () => {
  const h = harness({ headers: internalHeaders });
  const a = { ...internalRow, assignmentId: "phone-a-post", machine: "Máy A", tiktokAccount: "@a", partners: ["Đối tác A"], postedAt: "2026-09-10T13:00:00Z", status: "Đã gửi" };
  const b = { ...a, assignmentId: "phone-b-post", machine: "Máy B", tiktokAccount: "@b", partners: ["Đối tác B"] };
  assert.equal(h.deliver(a).row, 2);
  assert.equal(h.deliver(b).row, 3);
  const done = row => ({ ...row, rowRevision: 2, status: "Đã xác minh", stateNotes: "", postUrl: `https://www.tiktok.com/${row.tiktokAccount}/photo/${row === a ? "111111" : "222222"}` });
  h.state.lostResponse = true;
  assert.equal(h.deliver(done(b)).ok, false, "write completed but response was lost");
  assert.equal(h.deliver(done(a)).row, 2);
  assert.equal(h.deliver(done(b)).duplicate, true);
  assert.equal(h.deliver(b).duplicate, true, "old pending report cannot erase verified B");
  assert.equal(h.cells.length, 3);
  for (const [index, row] of [[1, a], [2, b]]) {
    assert.equal(h.cells[index][0], index);
    assert.equal(h.cells[index][2], "10/9/2026");
    assert.equal(h.cells[index][3], done(row).postUrl);
    assert.equal(h.cells[index][4], row.machine);
    assert.equal(h.cells[index][5], row.tiktokAccount);
    assert.equal(h.cells[index][6], "Đã xác minh");
    assert.equal(h.cells[index][8], row.partners[0]);
    assert.equal(JSON.parse(h.notes.get(`${index + 1}:4`)).assignmentId, row.assignmentId);
  }
  assert.equal(h.state.writes, 4);
  assert.equal(h.state.locks, 0);
});

test("compact sheet appends by received link order while retaining each machine post identity", () => {
  const h = harness();
  const a = { assignmentId: "a", poster: "Máy A", postUrl: "https://www.tiktok.com/@a/photo/111111", partners: ["Đối tác A"], postedAt: "2026-09-10T13:00:00Z" };
  const b = { assignmentId: "b", poster: "Máy B", postUrl: "https://www.tiktok.com/@b/photo/222222", partners: ["Đối tác B"], postedAt: "2026-09-10T13:00:00Z" };
  assert.equal(h.deliver(b).row, 2);
  assert.equal(h.deliver(a).row, 3);
  assert.equal(h.deliver(b).duplicate, true);
  assert.equal(h.deliver(a).duplicate, true);
  assert.deepEqual(h.cells.slice(1).map(r => r.slice(0,5)), [
    [1, "Máy B", "10/9/2026", b.postUrl, "Đối tác B"],
    [2, "Máy A", "10/9/2026", a.postUrl, "Đối tác A"],
  ]);
  assert.equal(h.state.writes, 2);
  assert.equal(h.state.locks, 0);
});

const deliveryV2 = { deliveryVersion: 2, spreadsheetId: "fixture-book", sheetGid: 0, deliveryRevision: 1, rowKind: "canonical" };

test("v2 canonical ACK contains the committed target identity revision and actual Link", () => {
  const h = harness();
  const ack = h.deliver(deliveryV2);
  assert.deepEqual(ack, { ok: true, row: 2, deliveryVersion: 2, spreadsheetId: "fixture-book", sheetGid: 0,
    assignmentId: "assignment-1", deliveryRevision: 1, postUrl: h.cells[1][3] });
  assert.equal(h.state.batches.length, 1);
  const note = JSON.parse(h.notes.get("2:4"));
  assert.equal(note.canonicalDeliveryRevision, 1);
  assert.equal(note.spreadsheetId, "fixture-book");
  assert.equal(h.deliver(deliveryV2).duplicate, true);
  assert.equal(h.state.writes, 1);
});

test("v2 malformed target identity metadata and dates fail before column expansion", () => {
  for (const patch of [{ spreadsheetId: "another-book" }, { sheetGid: 1 }, { deliveryVersion: 3 },
    { deliveryRevision: -1 }, { deliveryRevision: 1.2 }, { assignmentId: " padded " },
    { postUrl: "https://vt.tiktok.com/example" }, { postedAt: "not-a-date" }, { poster: null },
    { rowKind: "internalReport", reportVersion: 2, rowRevision: 1 }]) {
    const h = harness();
    const before = JSON.stringify(h.cells);
    assert.equal(h.deliver({ ...deliveryV2, partners: ["A", "B", "C"], ...patch }).ok, false, JSON.stringify(patch));
    assert.equal(JSON.stringify(h.cells), before);
    assert.equal(h.state.writes, 0);
    assert.equal(h.state.batches.length, 0);
  }
});

test("v2 header extension preserves trailing formulas notes and values in one atomic batch", () => {
  const h = harness({ headers: ["STT", "Người air", "Ngày", "Link", "Đối tác", "Đối tác 2", "Tổng"],
    rows: [["", "", "", "", "", "", 42]] });
  h.formulas.set("2:7", "=40+2"); h.notes.set("2:7", "user note");
  assert.equal(h.deliver({ ...deliveryV2, partners: ["A", "B", "C", "D"] }).ok, true);
  assert.deepEqual(h.cells[0].slice(4), ["Đối tác", "Đối tác 2", "Đối tác 3", "Đối tác 4", "Tổng"]);
  assert.deepEqual(h.cells[1].slice(4), ["A", "B", "C", "D", 42]);
  assert.equal(h.formulas.get("2:9"), "=40+2"); assert.equal(h.notes.get("2:9"), "user note");
  assert.equal(h.state.writes, 1);
  assert.equal(h.state.batches[0].requests[0].insertDimension.range.startIndex, 6);
});

test("v2 rejected expansion commits neither columns headers values nor key", () => {
  const h = harness(); h.state.rejectBatch = true;
  const before = JSON.stringify(h.cells);
  const ack = h.deliver({ ...deliveryV2, partners: ["A", "B", "C"] });
  assert.equal(ack.ok, false); assert.equal(ack.retryable, true);
  assert.equal(JSON.stringify(h.cells), before); assert.equal(h.notes.size, 0); assert.equal(h.state.writes, 0);
});

test("v2 ambiguous committed expansion retries from stored values without adding another row", () => {
  const h = harness(); h.state.lostResponse = true;
  const request = { ...deliveryV2, partners: ["A", "B", "C"] };
  assert.equal(h.deliver(request).ok, false);
  const ack = h.deliver(request);
  assert.equal(ack.ok, true); assert.equal(ack.duplicate, true); assert.equal(ack.deliveryRevision, 1);
  assert.equal(h.cells.length, 2); assert.equal(h.cells[0].length, 7); assert.equal(h.state.writes, 1);
});

test("v2 post-commit readback rejects a changed Link instead of echoing the request", () => {
  const h = harness();
  h.state.afterCommit = () => { h.cells[1][3] = "https://www.tiktok.com/@other/photo/999"; };
  const ack = h.deliver(deliveryV2);
  assert.equal(ack.ok, false); assert.equal(ack.postUrl, undefined);
});

test("v2 canonical refuses a different revision or immutable Link without writing", () => {
  const h = harness(); h.deliver(deliveryV2);
  assert.equal(h.deliver({ ...deliveryV2, deliveryRevision: 2 }).ok, false);
  assert.equal(h.deliver({ ...deliveryV2, postUrl: "https://www.tiktok.com/@test/photo/999" }).ok, false);
  assert.equal(h.deliver({ ...deliveryV2, postedAt: "2026-09-08T19:30:00Z" }).ok, false);
  assert.equal(h.state.writes, 1);
});

test("v2 stale canonical mismatch cannot acquire a canonical note on another verified Link", () => {
  const h = harness({ headers: internalHeaders });
  const verified = { ...deliveryV2, ...internalRow, deliveryRevision: 4, rowRevision: 4,
    postedAt: "2026-09-08T18:30:00Z", postUrl: "https://www.tiktok.com/@test/photo/123456", status: "Đã xác minh" };
  h.deliver(verified);
  const before = h.notes.get("2:4");
  assert.equal(h.deliver({ ...verified, rowKind: "canonical", deliveryRevision: 1, rowRevision: 3, postUrl: "https://www.tiktok.com/@test/photo/999" }).ok, false);
  assert.equal(h.notes.get("2:4"), before); assert.equal(h.state.writes, 1);
});

test("v2 canonical old-client duplicate upgrades only its already verified note", () => {
  const h = harness(); h.deliver();
  const row = [...h.cells[1]];
  assert.equal(h.deliver(deliveryV2).deliveryRevision, 1);
  assert.deepEqual(h.cells[1], row);
  assert.equal(h.state.writes, 2);
  assert.equal(h.state.batches[1].requests.length, 1);
  assert.equal(h.state.batches[1].requests[0].updateCells.fields, "note");
  assert.equal(h.deliver().duplicate, true);
  assert.equal(h.state.writes, 2);
});

test("v2 note upgrade reads the stored row again after its commit", () => {
  const h = harness(); h.deliver();
  h.state.afterCommit = () => h.formulas.set("2:4", '=HYPERLINK("https://example.org")');
  const ack = h.deliver(deliveryV2);
  assert.equal(ack.ok, false); assert.equal(ack.postUrl, undefined);
});

test("v2 internal stale report returns stored revision and canonical Link without erasing it", () => {
  const h = harness({ headers: internalHeaders });
  const pending = { ...deliveryV2, ...internalRow };
  assert.equal(h.deliver(pending).deliveryRevision, 1);
  const verified = { ...pending, deliveryRevision: 3, rowRevision: 3, postedAt: "2026-09-08T18:30:00Z",
    postUrl: "https://www.tiktok.com/@test/photo/123456", status: "Đã xác minh", stateNotes: "" };
  assert.equal(h.deliver(verified).deliveryRevision, 3);
  const ack = h.deliver(pending);
  assert.equal(ack.deliveryRevision, 3); assert.equal(ack.rowRevision, 3);
  assert.equal(ack.postUrl, verified.postUrl); assert.equal(ack.duplicate, true);
  assert.equal(h.state.writes, 2);
  assert.equal(h.deliver({ ...pending, deliveryRevision: 4, rowRevision: 4 }).ok, false);
});

test("v2 canonical metadata and subsequent reports retain independent canonical revision", () => {
  const h = harness({ headers: internalHeaders });
  const canonical = { ...deliveryV2, ...internalRow, rowKind: "canonical", rowRevision: 7,
    postedAt: "2026-09-08T18:30:00Z", postUrl: "https://www.tiktok.com/@test/photo/123456", status: "Đã xác minh", stateNotes: "" };
  assert.equal(h.deliver(canonical).deliveryRevision, 1);
  const report = { ...canonical, rowKind: "internalReport", rowRevision: 8, deliveryRevision: 8, stateNotes: "Đã ghi Sheet" };
  assert.equal(h.deliver(report).deliveryRevision, 8);
  const ack = h.deliver(canonical);
  assert.equal(ack.deliveryRevision, 1); assert.equal(ack.rowRevision, 8); assert.equal(ack.postUrl, canonical.postUrl);
  assert.equal(JSON.parse(h.notes.get("2:4")).canonicalDeliveryRevision, 1);
  assert.equal(h.state.writes, 2);
});

test("v2 report validation runs before partner expansion and stale rows never expand", () => {
  const h = harness({ headers: internalHeaders });
  const pending = { ...deliveryV2, ...internalRow, rowRevision: 2, deliveryRevision: 2 };
  assert.equal(h.deliver(pending).ok, true);
  const before = JSON.stringify(h.cells);
  assert.equal(h.deliver({ ...pending, partners: ["A", "B", "C"], rowRevision: 3, deliveryRevision: 3, status: "bad" }).ok, false);
  assert.equal(JSON.stringify(h.cells), before);
  assert.equal(h.deliver({ ...pending, partners: ["A", "B", "C"], rowRevision: 1, deliveryRevision: 1 }).ok, false);
  assert.equal(JSON.stringify(h.cells), before); assert.equal(h.state.writes, 1);
});

test("v2 expands rows only inside its commit and preserves exactly one assignment", () => {
  const h = harness({ rows: Array.from({ length: 19 }, (_, i) => [i + 1, "person", "date", `old-${i}`, "", ""]) });
  assert.equal(h.deliver(deliveryV2).ok, true);
  assert.equal(h.state.batches[0].requests[0].appendDimension.length, 1);
  assert.equal(h.deliver(deliveryV2).duplicate, true);
  assert.equal(h.state.writes, 1);
});

test("prepare blank target commits grid header and frozen row together and never writes on read-only check", () => {
  const h = harness({ headers: [] });
  const request = { rowKind: "prepare", checkVersion: 1, spreadsheetId: "fixture-book", sheetGid: 0 };
  const ack = h.deliver(request);
  assert.equal(ack.ok, true); assert.equal(ack.deliveryVersion, 2);
  assert.deepEqual(h.cells[0], internalHeaders.slice(0, 9));
  assert.equal(h.state.frozenRows, 1); assert.equal(h.state.writes, 1);
  assert.equal(h.deliver({ ...request, rowKind: "check" }).ok, true);
  assert.equal(h.state.writes, 1);
  const rejected = harness({ headers: [] }); rejected.state.rejectBatch = true;
  assert.equal(rejected.deliver(request).ok, false);
  assert.equal(rejected.cells.length, 0); assert.equal(rejected.state.frozenRows, 0); assert.equal(rejected.state.writes, 0);
});

test("legacy check remains available but never advertises v2 delivery support", () => {
  const headers = Array(34).fill(""); headers[1] = "Nhân Viên"; headers[3] = "Link"; headers[10] = "Đối tác";
  const h = harness({ headers, config: { LAYOUT_MODE: "legacy" } });
  const ack = h.deliver({ rowKind: "check", checkVersion: 1, spreadsheetId: "fixture-book", sheetGid: 0 });
  assert.equal(ack.ok, true); assert.equal(ack.deliveryVersion, 1);
  assert.equal(h.deliver(deliveryV2).ok, false); assert.equal(h.state.writes, 0);
});

test("prepare validates its target header configuration before any grid mutation", () => {
  for (const config of [{ LAYOUT_MODE: "compact" }, { LAYOUT_MODE: "legacy" }, { LINK_COLUMN: 3 }, { POSTER_COLUMN: 1 }]) {
    const h = harness({ headers: [], config });
    assert.equal(h.deliver({ rowKind: "prepare", checkVersion: 1, spreadsheetId: "fixture-book", sheetGid: 0 }).ok, false);
    assert.equal(h.cells.length, 0); assert.equal(h.state.writes, 0); assert.equal(h.state.batches.length, 0);
  }
});

test("v2 canonical metadata still requires the immutable send timestamp", () => {
  const h = harness({ headers: internalHeaders });
  assert.equal(h.deliver({ ...deliveryV2, ...internalRow, rowKind: "canonical", status: "Đã xác minh", postUrl: "https://www.tiktok.com/@test/photo/123456" }).ok, false);
  assert.equal(h.state.writes, 0);
});
