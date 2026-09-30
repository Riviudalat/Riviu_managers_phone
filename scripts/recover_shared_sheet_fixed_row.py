"""Explicit operator recovery of one frozen existing-row Sheets operation.

Preview is GET/read-only. Execution requires the reviewed preview hash and drained
writers. It writes the same fixed cells/note under the retained exact lock, once.
It never changes the local journal, appends a row, deletes a lock, or retries POST.
"""
import argparse
import ctypes
import ctypes.wintypes as w
import datetime as dt
import hashlib
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import urllib.parse
import urllib.request


TOKEN = "b9a27cba-995d-4e9f-86f4-e92bd47095d0"
PUBLICATION = "35d9f8ee-c8cb-439c-aabd-63cda9139ddb"
PAYLOAD_HASH = "63bbb96334066669835bf41cabb7111b7532358098140c95dbb26df87886fbff"
LOCK_KEY = "riviu.direct.shared-lock.v2"


def compact(value):
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":"))


def digest(value):
    return hashlib.sha256(compact(value).encode()).hexdigest()


def save(path, value, exclusive=False):
    with path.open("x" if exclusive else "w", encoding="utf-8", newline="\n") as f:
        json.dump(value, f, ensure_ascii=False, indent=2)
        f.flush()
        os.fsync(f.fileno())


def credential():
    class Cred(ctypes.Structure):
        _fields_ = [("Flags", w.DWORD), ("Type", w.DWORD), ("TargetName", w.LPWSTR),
                    ("Comment", w.LPWSTR), ("LastWritten", w.FILETIME),
                    ("CredentialBlobSize", w.DWORD),
                    ("CredentialBlob", ctypes.POINTER(ctypes.c_ubyte)),
                    ("Persist", w.DWORD), ("AttributeCount", w.DWORD),
                    ("Attributes", ctypes.c_void_p), ("TargetAlias", w.LPWSTR),
                    ("UserName", w.LPWSTR)]
    api = ctypes.WinDLL("advapi32", use_last_error=True)
    api.CredReadW.argtypes = [w.LPCWSTR, w.DWORD, w.DWORD,
                             ctypes.POINTER(ctypes.POINTER(Cred))]
    api.CredReadW.restype = w.BOOL
    api.CredFree.argtypes = [ctypes.c_void_p]
    ptr = ctypes.POINTER(Cred)()
    if not api.CredReadW("app-secret:google-oauth-tokens-v1.riviu-managers-phone", 1, 0,
                         ctypes.byref(ptr)):
        raise RuntimeError("Named application credential unavailable")
    try:
        return json.loads(ctypes.string_at(ptr.contents.CredentialBlob,
                          ptr.contents.CredentialBlobSize).decode("utf-16-le"))["accessToken"]
    finally:
        api.CredFree(ptr)


def journal(db):
    with sqlite3.connect(db.resolve().as_uri() + "?mode=ro", uri=True) as conn:
        conn.execute("PRAGMA query_only=ON")
        rows = conn.execute("SELECT key,value FROM settings WHERE key LIKE "
                            "'google.sheets.shared-journal.v2.%'").fetchall()
    found = [(key, raw, json.loads(raw)) for key, raw in rows
             if json.loads(raw).get("token") == TOKEN]
    if len(found) != 1:
        raise RuntimeError("Exact pending operation not found uniquely")
    key, raw, op = found[0]
    assert op["phase"] == "mutationPending" and op["receipt"] is None
    assert op["payloadHash"] == PAYLOAD_HASH
    assert op["payload"]["publicationId"] == PUBLICATION
    assert op["payload"]["rowKind"] == "internalReport"
    assert op["payload"]["rowRevision"] == op["payload"]["deliveryRevision"] == 23
    assert op["row"] == 452 and op["width"] == 20 and op["duplicate"] is True
    assert digest({"target": op["target"], "payload": op["payload"],
                   "writer": op["writer"], "upgradeConfirmed": op["upgradeConfirmed"]}) == PAYLOAD_HASH
    return key, hashlib.sha256(raw.encode()).hexdigest(), op


def remote(access, op):
    base = "https://sheets.googleapis.com/v4/spreadsheets/" + op["target"]["spreadsheetId"]
    query = urllib.parse.urlencode({"includeGridData": "true", "ranges": ["A1:AT1", "A453:AT453"],
        "fields": "spreadsheetId,properties(timeZone),developerMetadata,sheets(properties,developerMetadata,data(startRow,startColumn,rowData(values(formattedValue,userEnteredValue,note))))"}, doseq=True)
    request = urllib.request.Request(base + "?" + query,
                                    headers={"Authorization": "Bearer " + access})
    with urllib.request.urlopen(request, timeout=40) as response:
        return json.load(response)


def cell(value):
    entered = value.get("userEnteredValue") or None
    return {"formattedValue": value.get("formattedValue", ""),
            "userEnteredValue": entered, "note": value.get("note", "")}


def cells(data, row, width=20):
    chunks = [d for d in data["data"] if d.get("startRow", 0) == row
              and d.get("startColumn", 0) == 0]
    assert len(chunks) == 1
    values = chunks[0]["rowData"][0].get("values", [])
    assert all(not v.get("userEnteredValue", {}).get("formulaValue") for v in values)
    assert all(not cell(v)["formattedValue"] and not cell(v)["note"] and
               cell(v)["userEnteredValue"] is None for v in values[width:])
    return [cell(values[i] if i < len(values) else {}) for i in range(width)]


def build(op, data):
    target = op["target"]
    assert data["spreadsheetId"] == target["spreadsheetId"]
    sheets = [s for s in data["sheets"] if s["properties"]["sheetId"] == target["sheetGid"]]
    assert len(sheets) == 1
    sheet = sheets[0]
    assert data["properties"]["timeZone"] == "Asia/Bangkok"
    assert sheet["properties"]["gridProperties"]["rowCount"] == 1000
    assert sheet["properties"]["gridProperties"]["columnCount"] == 45
    metadata = data.get("developerMetadata", []) + sheet.get("developerMetadata", [])
    locks = [m for m in metadata if m.get("metadataKey") == LOCK_KEY]
    assert len(locks) == 1
    lock = locks[0]
    expected_lock = {"schemaVersion": 2, "operationToken": TOKEN, "writerId": op["writer"],
        "spreadsheetId": target["spreadsheetId"], "sheetGid": target["sheetGid"],
        "reportingEpoch": op["lockEpoch"], "payloadHash": PAYLOAD_HASH,
        "publicationId": PUBLICATION, "revision": 23, "phase": "acquired"}
    assert lock["metadataValue"] == compact(expected_lock)
    assert lock["metadataId"] == 1464869109 and lock["location"]["sheetId"] == 0
    owners = [m for m in metadata if m.get("metadataKey") == "riviu.direct.writer.v1"]
    assert len(owners) == 1
    owner = json.loads(owners[0]["metadataValue"])
    assert owner == {"schemaVersion": 2, "writerId": op["writer"],
                     "reportingEpoch": op["epoch"], "state": "ready"}
    p = op["payload"]
    assert op["epoch"] == op["lockEpoch"] == p["reportingEpoch"] == target["reportingEpoch"]
    before = cells(sheet, 452)
    assert before == op["scan"]["scan"]["matched"]["cells"]
    note = json.loads(before[3]["note"])
    assert note["rowRevision"] == 22 and note["publicationId"] == PUBLICATION
    assert note["assignmentId"] == PUBLICATION and note["reportingEpoch"] == op["epoch"]
    assert note["postedAt"] == note["canonicalPostedAt"] == p["postedAt"]
    assert note["canonicalDeliveryRevision"] == 0
    values = [c["formattedValue"] for c in before]
    assert digest(values) == note["rowFingerprint"]
    meta = [p[k] for k in ["machine", "tiktokAccount", "status", "stateNotes"]]
    partners = p["partners"] + [""] * (12 - len(p["partners"]))
    desired = [values[0], p["poster"], values[2], p["postUrl"], *meta, *partners]
    assert desired == values
    assert digest([p["poster"], p["postedAt"], p["postUrl"], meta, partners]) == note["payloadFingerprint"]
    header = [c["formattedValue"] for c in cells(sheet, 0)]
    assert len(header) == 20 and all(header)
    note["rowRevision"] = 23
    committed = dict(expected_lock, phase="committed")
    requests = [
        {"updateCells": {"start": {"sheetId": 0, "rowIndex": 452, "columnIndex": 0},
            "rows": [{"values": [{"userEnteredValue": {"numberValue": int(desired[0])}}] +
                     [{"userEnteredValue": {"stringValue": v}} for v in desired[1:]]}],
            "fields": "userEnteredValue"}},
        {"updateCells": {"start": {"sheetId": 0, "rowIndex": 452, "columnIndex": 3},
            "rows": [{"values": [{"note": compact(note)}]}], "fields": "note"}},
        {"updateDeveloperMetadata": {"dataFilters": [{"developerMetadataLookup": {
            "metadataId": lock["metadataId"], "metadataKey": LOCK_KEY,
            "metadataValue": lock["metadataValue"], "metadataLocation": {"sheetId": 0},
            "visibility": "DOCUMENT"}}], "developerMetadata": {"metadataValue": compact(committed)},
            "fields": "metadataValue"}}]
    return {"operationToken": TOKEN, "payloadHash": PAYLOAD_HASH, "publicationId": PUBLICATION,
            "spreadsheetId": target["spreadsheetId"], "sheetGid": 0, "sheetRow": 453,
            "beforeNote": json.loads(before[3]["note"]), "afterNote": note,
            "displayedValuesUnchanged": True, "rowFingerprint": note["rowFingerprint"],
            "headerFingerprint": digest(header), "owner": owner, "lockBefore": expected_lock,
            "lockAfter": committed, "requestBody": {"requests": requests}}


def main():
    if not __debug__:
        raise RuntimeError("Optimized Python disables validation; recovery refused")
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--db", required=True, type=Path)
    parser.add_argument("--out", required=True, type=Path)
    parser.add_argument("--execute-preview-sha256")
    parser.add_argument("--writers-drained", action="store_true")
    args = parser.parse_args()
    args.out.mkdir(parents=True, exist_ok=True)
    key, raw_hash, op = journal(args.db)
    access = credential()
    observed = remote(access, op)
    plan = build(op, observed)
    plan["journalKey"] = key
    plan["journalSha256"] = raw_hash
    plan["dbPath"] = str(args.db.resolve())
    preview = args.out / "repair-preview.json"
    if not args.execute_preview_sha256:
        save(args.out / "remote-before.json", observed)
        save(args.out / "journal-before.json", op)
        save(preview, plan)
        print(json.dumps({"mode": "preview", "readOnly": True, "requests": 3,
              "sha256": hashlib.sha256(preview.read_bytes()).hexdigest(), "path": str(preview)}))
        return
    assert args.writers_drained, "All cooperating writers must be drained first"
    running = subprocess.run(["powershell", "-NoProfile", "-Command",
        "@(Get-Process -Name riviu-managers-phone -ErrorAction SilentlyContinue).Count"],
        capture_output=True, text=True, check=True)
    assert running.stdout.strip() == "0", "Riviu application is still running"
    assert hashlib.sha256(preview.read_bytes()).hexdigest() == args.execute_preview_sha256
    assert json.loads(preview.read_text(encoding="utf-8")) == plan, "Fresh remote/journal differs"
    assert journal(args.db)[1] == raw_hash, "Journal changed before dispatch"
    save(args.out / "dispatch-intent.json", {"at": dt.datetime.now(dt.timezone.utc).isoformat(),
         "previewSha256": args.execute_preview_sha256, "requestBody": plan["requestBody"],
         "singleDispatch": True, "writersDrained": True}, exclusive=True)
    endpoint = "https://sheets.googleapis.com/v4/spreadsheets/" + plan["spreadsheetId"] + ":batchUpdate"
    request = urllib.request.Request(endpoint, data=compact(plan["requestBody"]).encode(),
        headers={"Authorization": "Bearer " + access, "Content-Type": "application/json"}, method="POST")
    try:
        with urllib.request.urlopen(request, timeout=40) as response:
            ack = {"status": response.status, "body": json.load(response)}
        save(args.out / "dispatch-ack.json", ack, exclusive=True)
    except Exception as error:
        save(args.out / "dispatch-error.json", {"type": type(error).__name__,
             "status": getattr(error, "code", None), "outcome": "uncertain; do not resend"}, exclusive=True)
        raise RuntimeError("Dispatch outcome uncertain; retained intent; no retry") from None
    after = remote(access, op)
    save(args.out / "remote-after.json", after, exclusive=True)
    sheet = next(s for s in after["sheets"] if s["properties"]["sheetId"] == 0)
    lock = [m for m in sheet.get("developerMetadata", []) if m.get("metadataKey") == LOCK_KEY]
    assert len(lock) == 1 and lock[0]["metadataValue"] == compact(plan["lockAfter"])
    actual = cells(sheet, 452)
    assert json.loads(actual[3]["note"]) == plan["afterNote"]
    assert digest([c["formattedValue"] for c in actual]) == plan["rowFingerprint"]
    assert journal(args.db)[1] == raw_hash, "Local journal unexpectedly changed"
    save(args.out / "repair-verified.json", {"at": dt.datetime.now(dt.timezone.utc).isoformat(),
         "committedMarkerVerified": True, "rowRevision": 23, "sheetRow": 453,
         "exactRowVerified": True, "localJournalUnmodified": True,
         "next": "Restart app; existing production reconcile reads receipt and releases its exact lock"}, exclusive=True)
    print("Exact fixed-row repair committed and verified; local journal remains unchanged")


if __name__ == "__main__":
    main()
