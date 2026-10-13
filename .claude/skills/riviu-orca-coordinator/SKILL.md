---
name: riviu-orca-coordinator
description: >
  Dùng khi điều phối worker Orca cho Riviu: viết spec worker ("Read COMMON first"), mở worker
  bằng worker-start trong worktree con, chờ worker_done/escalation/question, ack heartbeat,
  cherry-pick commit lên main, release worker, dọn worktree vào Recycle Bin (kể cả khi
  vite/Playwright của worker còn giữ thư mục) và đưa "Bài học cho docs" vào owner doc.
  Không push, merge hay release khi người dùng chưa duyệt; không xóa vĩnh viễn worktree.
metadata:
  version: "1.1.0"
  origin: local
  project: riviu-manager
---

# Điều phối worker Orca cho Riviu

Skill này là trình tự của coordinator; cú pháp CLI chính chủ ở skill `orchestration`
(nạp nó để biết cách resolve binary `orca` và verbs). Vòng sửa lỗi tổng thể ở
`riviu-fix-cycle`.

## Vòng một worker

1. **Spec.** Viết `target/<run>/spec-<id>.md` từ [mẫu spec](references/spec-template.md):
   dòng đầu trỏ tới file common của run, nêu phạm vi file, cổng kiểm và báo cáo. Một worker
   = một phạm vi file không chồng nhau với worker đang chạy.
2. **Start.** `worker-start --spec "Read and do <spec path>" --worktree new-child --name <id>
   --base-branch main --agent claude`. Ghi lại `dispatchId`.
3. **Chờ.** `check --wait --types "worker_done,escalation,question"` chạy nền. Heartbeat đến
   như một delivery riêng: đọc, rồi `check --ack <deliveryId>`; không ack thì delivery cứ lặp lại.
   Trả lời `question` bằng `reply`; escalation thì quyết định hoặc hỏi người dùng.
4. **Đọc worker_done.** Body 3 câu, rồi branch/commit, gate, phần cần máy thật và "Bài học
   cho docs". Kiểm `git log main..<branch>` trước khi tin.
5. **Merge.** `git cherry-pick <commits>` lên main, theo thứ tự; xung đột thì giải trên main,
   không ép worker rebase giữa chừng.
6. **Dọn.** [Dọn worker](references/lifecycle.md#dọn-worker): `worker-release --dispatch`,
   ack, worktree vào Recycle Bin, `git worktree prune`, `git branch -D`.
7. **Bài học.** Mỗi dòng "Bài học cho docs" vào đúng owner doc nó nêu, trong commit riêng
   `docs(...)`. Không viết changelog.

Chi tiết lệnh, timeout và các ca lỗi: [vòng đời worker](references/lifecycle.md).

## Liên lạc với worker: ca đã mất giờ (13/10/2026)

- **`question` phải trả bằng `reply --id <msg_id>`.** Worker gọi `orchestration ask` chặn cho
  tới khi có reply đúng id; `send --to dispatch:<id>` không mở khóa nó. HX-START chờ 1 giờ vì
  lỗi này.
- Worker tự poll `check --terminal term_<…>` thì gửi tới `term_<…>` (lấy từ `from_handle` của
  tin nó gửi). Không chắc thì gửi cả hai handle.
- **Không bắt worker chờ "tin tiếp theo của coordinator".** Giao điều kiện tự kiểm: "chạy
  cargo khi không còn `rustc.exe`/`cargo.exe` 60 s", "khi `git log main..cattfan/<x>` có
  commit". Một vòng tin đi-về mất 10–30 phút vì worker ngủ giữa các lần poll.
- Chỉ một build Rust tại một thời điểm khi controller nghiệm thu đang chạy (WebView 3 GB,
  máy 32 GB). Xếp thứ tự worker rõ trong tin.
- Không chạy `check --wait` dài ở nền khi RAM căng: hệ thống giết tiến trình nền và bạn mất
  thông báo. Poll bằng `check` không chờ; worker im lặng thì `worker-show --dispatch`
  (`attention.requiresAction:true`, `categories:["input"]` = đang chờ gì đó) rồi
  `worker-read --dispatch` đọc đuôi transcript để biết nó chờ cái gì.
- Output JSON của `orca` có BOM: Python đọc bằng `utf-8-sig`/`lstrip('﻿')`.
- Worktree bị tiến trình giữ ("being used by another process"): để đó, thử lại sau; không
  xóa cưỡng bức, không `git worktree remove --force`.
- Worker không chạm điện thoại; mọi việc trên máy do coordinator làm, với bằng chứng (ảnh,
  log, serial, giờ) ghi vào spec cho worker.

## Dừng lại hỏi người dùng khi

- worker xin chạm phone/controller/adb ngoài phạm vi spec;
- hai worker cần cùng một file;
- cần push, merge vào remote, tag hay release;
- worker báo outcome `failed` hai lần cho cùng một spec.
