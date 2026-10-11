# Vòng đời worker Orca

`orca` resolve theo skill `orchestration` (PATH của phiên Orca, hoặc binary đi kèm bản cài
Orca trên máy). Không hard-code đường dẫn vào spec.

## Start

```sh
orca orchestration worker-start --spec "Read and do <path>/spec-<id>.md" \
  --worktree new-child --name <id> --base-branch main --agent claude --json
```

- `--name` thành tên worktree và branch; dùng id ngắn của spec (vd. `s1-skills`).
- Worktree mới chạy setup của repo. Worker tự `npm ci` trong worktree nếu cần UI gate.
- Mọi worktree dùng chung `target/` của checkout chính: chỉ một worker Cargo tại một lúc
  (máy ít RAM). Lên lịch worker Rust nối tiếp, worker UI/docs song song được.

## Chờ và ack

```sh
orca orchestration check --wait --types "worker_done,escalation,question" --timeout-ms 1800000 --json
```

- Chạy nền; nó in keepalive JSON ra stderr mỗi 15 s (`_keepalive`, không phải heartbeat).
- `--types` chỉ là điều kiện đánh thức; delivery trả về là **cả batch FIFO**, có thể gồm
  heartbeat. Xử lý mọi message trong batch rồi `check --ack <deliveryId>`. Không ack thì
  Run phát lại đúng batch đó mãi.
- Heartbeat 5 phút/lần là tín hiệu sống. Không có heartbeat > 15 phút và `worker-read`
  không có output mới: `worker-show`, rồi hỏi người dùng trước `worker-stop`.
- `question` từ `ask`: trả lời bằng `reply`. Worker đang block nên trả lời sớm.

## Đọc kết quả và merge

1. `worker_done` body 3 câu; outcome `failed` thì đọc lý do trước khi retry.
2. `git -C <main> log --oneline main..<branch>` để thấy commit thật.
3. `git cherry-pick <sha...>` trên main. Commit giữ trailer `Co-Authored-By`.
4. Chạy cổng tích hợp trên main theo
   [Cổng theo thay đổi](../../../../docs/development/testing.md#cổng-theo-thay-đổi), hẹp trước.

## Dọn worker

Thứ tự này; bước nào lỗi thì dừng và báo, không làm mạnh tay hơn:

1. `orca orchestration worker-release --dispatch <dispatchId>`. Idempotent; chỉ
   `release_unknown` mới exit 1. Output vẫn đọc được bằng `worker-read`.
2. Ack delivery cuối cùng.
3. Đưa thư mục worktree vào **Recycle Bin**, không xóa vĩnh viễn:

   ```powershell
   Add-Type -AssemblyName Microsoft.VisualBasic
   [Microsoft.VisualBasic.FileIO.FileSystem]::DeleteDirectory('<worktree path>', 'OnlyErrorDialogs', 'SendToRecycleBin')
   ```

4. `git worktree prune`, rồi `git branch -D <branch>` (chỉ sau khi commit đã ở main).

### Thư mục còn bị giữ

Lỗi "being used by another process" hoặc "system call level is not correct" khi Recycle:
`vite` / Playwright `node` của worker đã xong vẫn chạy từ worktree đó. Tìm **đúng** các
process có command line trỏ vào worktree, dừng chỉ chúng:

```powershell
$wt = '<worktree path>'
Get-CimInstance Win32_Process -Filter "Name='node.exe'" |
  Where-Object { $_.CommandLine -like "*$wt*" } |
  Select-Object ProcessId, CommandLine
# Xem danh sách, rồi: Stop-Process -Id <id,...>
```

Cùng kiểu đó: một vòng `bash` `until grep … tasks/<id>.output` mà worker để chạy nền rồi
quên (ví dụ chờ một build đã bị dừng). Command line của nó chứa đường dẫn worktree trong
thư mục `tasks` của session. Khi đó `worker-release` trả `release_unknown`. Dừng đúng vòng
đó, rồi release lại. Có process không lộ đường dẫn trên command line (shell có cwd trong
worktree) thì để lại và báo người dùng, không đoán.

Không dừng `node` của worktree khác, của controller hay Riviu MCP; không dừng `riviu*.exe`,
adb hay process không có đường dẫn worktree trong command line.

## Retry

Spec sai thì sửa spec rồi `worker-start --retry-of <dispatchId>`; worker cũ đã settle thì
release trước. Không chạy hai dispatch song song trên cùng một spec.
