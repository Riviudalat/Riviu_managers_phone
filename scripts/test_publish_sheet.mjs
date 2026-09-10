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
  const cells = [headers, ...rows].map(row => [...row]);
  const notes = new Map();
  const formulas = new Map();
  const state = { writes: 0, batches: [], lostResponse: false, rejectBatch: false, locks: 0 };
  let maxRows = 20;
  const maxColumns = Math.max(headers.length, ...rows.map(row => row.length));
  const read = (r, c) => cells[r - 1]?.[c - 1] ?? "";
  const sheet = {
    getSheetId: () => gid,
    getParent: () => book,
    getLastRow: () => cells.length,
    getMaxRows: () => maxRows,
    getMaxColumns: () => maxColumns,
    getLastColumn: () => maxColumns,
    insertRowsAfter: (_after, count) => { maxRows += count; },
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
      state.batches.push(batch);
      if (state.rejectBatch) throw new Error("fixture batch rejected before commit");
      for (const request of batch.requests) {
        const update = request.updateCells;
        assert.ok(update, "only cell writes expected");
        assert.equal(update.start.sheetId, gid);
        const r = update.start.rowIndex + 1;
        const c = update.start.columnIndex + 1;
        assert.ok(r <= maxRows && c + update.rows[0].values.length - 1 <= maxColumns);
      }
      for (const { updateCells: update } of batch.requests) {
        update.rows.forEach((row, y) => row.values.forEach((cell, x) => {
          const r = update.start.rowIndex + y + 1;
          const c = update.start.columnIndex + x + 1;
          cells[r - 1] ??= [];
          if (update.fields.includes("userEnteredValue")) cells[r - 1][c - 1] = cell.userEnteredValue?.stringValue ?? cell.userEnteredValue?.numberValue ?? "";
          if (update.fields.includes("note")) notes.set(`${r}:${c}`, cell.note ?? "");
        }));
      }
      state.writes++;
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

test("compact partner overflow is rejected without dropping names", () => {
  const h = harness();
  assert.equal(h.deliver({ partners: ["A", "B", "C"] }).ok, false);
  assert.equal(h.state.writes, 0);
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
    assert.deepEqual(result, { ok: true, checkVersion: 1, spreadsheetId: "fixture-book", sheetGid: 37, layout, columns: headers });
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
