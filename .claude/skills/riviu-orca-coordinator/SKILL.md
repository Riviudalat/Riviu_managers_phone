---
name: riviu-orca-coordinator
description: >
  Dùng khi điều phối worker Orca cho Riviu: viết spec worker ("Read COMMON first"), mở worker
  bằng worker-start trong worktree con, chờ worker_done/escalation/question, ack heartbeat,
  cherry-pick commit lên main, release worker, dọn worktree vào Recycle Bin (kể cả khi
  vite/Playwright của worker còn giữ thư mục) và đưa "Bài học cho docs" vào owner doc.
  Không push, merge hay release khi người dùng chưa duyệt; không xóa vĩnh viễn worktree.
metadata:
  version: "1.0.0"
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

## Dừng lại hỏi người dùng khi

- worker xin chạm phone/controller/adb ngoài phạm vi spec;
- hai worker cần cùng một file;
- cần push, merge vào remote, tag hay release;
- worker báo outcome `failed` hai lần cho cùng một spec.
