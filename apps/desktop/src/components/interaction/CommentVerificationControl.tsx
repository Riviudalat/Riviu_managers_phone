import { useState } from "react";
import { interactionVerifyComment } from "../../api";
import { describeError } from "../../describeError";
import type { CommentVerification } from "../../types";

export function CommentVerificationControl({campaignId,assignmentId,value,disabled=false}:{campaignId:string;assignmentId:string;value:CommentVerification;disabled?:boolean}) {
  const [busy,setBusy]=useState(false);
  const [error,setError]=useState<string|null>(null);
  async function check(){setBusy(true);setError(null);try{await interactionVerifyComment(campaignId,assignmentId);}catch(e){setError(describeError(e));}finally{setBusy(false);}}
  const label=value.state==="verified"?"Đã xác minh nội dung":value.state==="pending"?"Đã thao tác gửi — đang xác minh":"Chưa xác minh được nội dung — cần kiểm tra";
  return <div className="interaction-readback" aria-label="Xác minh bình luận">
    <small role="status">{label}{value.state==="pending"?` · lần ${value.attempts}/3`:""}</small>
    {value.nextCheckAtMs&&value.state==="pending"?<small>Kiểm tra tiếp lúc {new Date(value.nextCheckAtMs).toLocaleTimeString("vi-VN")}</small>:null}
    {value.reason&&value.state==="needsReview"?<small>{value.reason.startsWith("legacy_missing_evidence")?"Lịch sử thiếu bằng chứng đọc lại.":value.reason.includes("account")||value.reason.includes("author")?"Chưa đối chiếu được tài khoản của người đăng.":value.reason.includes("parent")?"Chưa thấy đúng nhánh bình luận cha.":value.reason.includes("not_visible")?"Chưa tìm thấy bình luận trong danh sách đã đọc.":"Chưa đủ bằng chứng sau các lần kiểm tra. Bạn có thể yêu cầu đọc lại."}</small>:null}
    {value.state==="needsReview"?<button type="button" className="btn btn-sm" disabled={disabled||busy} onClick={()=>void check()}>{busy?"Đang yêu cầu…":"Đọc lại bình luận"}</button>:null}
    {error?<small role="alert">{error}</small>:null}
  </div>;
}
