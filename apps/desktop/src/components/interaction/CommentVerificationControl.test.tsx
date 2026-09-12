import {render,screen,fireEvent,waitFor} from "@testing-library/react";
import {describe,it,expect,vi} from "vitest";
import {CommentVerificationControl} from "./CommentVerificationControl";
vi.mock("../../api",()=>({interactionVerifyComment:vi.fn().mockResolvedValue({state:"pending"})}));
import {interactionVerifyComment} from "../../api";
const base={attempts:1,nextCheckAtMs:null,deadlineMs:null,reason:null,evidence:null};
describe("comment verification",()=>{
 it("pending delivery is not called verified or offered a send retry",()=>{
  render(<CommentVerificationControl campaignId="c" assignmentId="a" value={{...base,state:"pending"}}/>);
  expect(screen.getByRole("status").textContent).toContain("đang xác minh");
  expect(screen.queryByRole("button")).toBeNull();
 });
 it("needs review invokes observation for the exact assignment",async()=>{
  render(<CommentVerificationControl campaignId="c" assignmentId="a" value={{...base,state:"needsReview"}}/>);
  fireEvent.click(screen.getByRole("button",{name:"Đọc lại bình luận"}));
  await waitFor(()=>expect(interactionVerifyComment).toHaveBeenCalledWith("c","a"));
 });
});
