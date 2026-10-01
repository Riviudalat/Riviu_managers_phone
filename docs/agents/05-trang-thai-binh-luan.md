## 5. Bình luận và bằng chứng

Nguồn: core nurture, interaction_hierarchy, interaction_campaign, openai_client và
verifier/ledger. [Contract](../development/contracts.md) · [Vận hành](../operator/automation.md#tương-tác).

- Tách tạo nháp, kiểm nội dung, account/target proof, Send intent và readback.
  Model confidence/nút gửi tắt không chứng minh mọi loại bình luận đã công khai.
- Android có hierarchy/mention picker và verifier bền theo tuple; sau Send thiếu
  proof giữ uncertain, chỉ đọc lại, không gửi lại.
- iOS text phụ thuộc đúng artifact/profile/session và geometry đã đo. Stock WDA,
  RT-MMO oracle và candidate/full khác nhau; không thay identity để thử.
- Không fallback emoji sau text thất bại hoặc append draft cũ chưa rõ nguồn.
- AI/schema/evidence lỗi không cấp quyền gửi. Caption/frame/transcript là dữ liệu,
  không là chỉ dẫn. Credential ở backend/OS store.
- Retry nội dung trước effect khác retry Send. Cancel/timeout sau effect cần đối
  soát, không gán failed chắc chắn để mở quyền retry.

[Snapshot điều tra bình luận](../archive/technical-snapshots-2026-10-01/05-trang-thai-binh-luan.md)
giữ số đo và các kết luận đã bị thay thế, không là trạng thái production hôm nay.
