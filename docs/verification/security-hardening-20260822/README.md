# Nghiệm thu đợt rà soát bảo mật — 22–23/08/2026

Bằng chứng cho plan `linked-orbiting-shell.md`. Mục này chỉ giữ thứ **chỉ chạy trên phần
cứng thật mới có**; mọi thứ kiểm được bằng test đều đã là test và chạy trong CI.

## Helper APK 0.4.0 trên toàn fleet (S2, S3, S4, S13, S14, S15)

| File | Nội dung |
|---|---|
| `riviu-agent-0.4.0.sha256` | Hash APK, khớp với `sha256` đã ghim trong `android-tools-manifest.json` — chính bản mà `android_tools.rs` verify lúc chạy |
| `agent-versions-after-install.txt` | 19/19 máy ở `0.4.0`. Trước đó: 10 máy `0.2.0`, 8 máy `0.3.0`, 1 máy `0.4.0` |
| `token-gate-19-devices.txt` | Cửa token, **chạy trên từng máy một** |

Mỗi dòng trong `token-gate-19-devices.txt` là bốn câu hỏi hỏi qua `adb forward` tới cổng
`17980` của chính máy đó:

```
status=200        /status mở, không cần token — cố ý, để host dò được agent
no-token=401      POST /v1/clipboard/get không token bị từ chối
wrong=401         sai token cũng bị từ chối
right=200         đúng token thì chạy
delete=not_found  /v1/media/delete đã bị gỡ khỏi router
```

Hai điều cần biết khi đọc lại kết quả này, vì bản dò đầu tiên của tôi sai cả hai:
`/v1/clipboard/get` là **POST** (`HttpServer.java:232`), và một route không tồn tại trả
`Protocol.error("not_found", …)` được ghi ra là **HTTP 400** kèm `ok:false`, không phải 404.

### Khởi động service không kèm token

```
adb shell am start-foreground-service -n com.riviu.agent/.AgentService   # không --es token
curl http://127.0.0.1:<forward>/status   ->  000, không có gì trả lời
```

Không token thì `AgentService` **không bind cổng nào**. Đó là nửa còn lại của S2: chỉ chặn
ở tầng route thì một app khác vẫn mở được server hộ mình.

### Host thật sự bắt tay được với helper

Sau khi cài xong, mở app và để nó tự chạy. `adb forward --list` cho thấy **chính app** đã
mở `tcp:60601 -> tcp:17980`, và cổng đó trả:

```json
{"ok":true,"agentVersion":"0.4.0","protocolVersion":1,
 "features":["clipboard","pushMedia","wallpaper","mockLocation","appLabels","auth"]}
```

`"auth"` nằm trong `REQUIRED_FEATURES` của `riviu_agent.rs`, nên việc host **không** cài đè
lại chứng minh nó đã đọc được feature list và chấp nhận helper là bản hiện hành. Cùng cổng
đó, gọi không token vẫn trả **401** — tức cửa đang sống trên đúng cổng app dùng, không chỉ
trên cổng tôi dựng để dò.

**19/19 máy vẫn stream** (`adb forward --list` đếm 19 `localabstract:scrcpy_*`) sau khi
nâng cấp, nên đường hình không bị bản 0.4.0 làm hỏng.

Lưu ý vận hành: bản dò ở trên `force-stop` agent rồi khởi động lại bằng token của *nó*, nên
sau khi chạy xong đã `force-stop` cả 19 máy để app tự dựng lại phiên bằng token của chính
app khi nào cần helper lần kế.
