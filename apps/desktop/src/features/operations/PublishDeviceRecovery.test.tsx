import {render,screen,fireEvent,waitFor} from "@testing-library/react";
import {vi,it,expect,beforeEach} from "vitest";
import {PublishDeviceRecovery} from "./PublishDeviceRecovery";
import {publishGet,publishRecoveryCapabilities,publishRetryAssignment,publishCheckLinks,operationDeviceLog,publishResumeVerification} from "../../api";
vi.mock("../../api",()=>({operationDeviceLog:vi.fn(async()=>({entries:[],truncated:false})),listDevices:vi.fn(async()=>[{udid:"device",status:"ready"}]),publishRetrySheetAssignment:vi.fn(),publishGet:vi.fn(),publishRecoveryCapabilities:vi.fn(),publishRetryAssignment:vi.fn(),publishCheckLinks:vi.fn(),publishResumeVerification:vi.fn()}));
vi.mock("./useMonitorRead",async()=>{const React=await import("react");return{useMonitorRead:(read:()=>Promise<unknown>)=>{const [value,setValue]=React.useState<unknown>(null);React.useEffect(()=>{void read().then(setValue);},[read]);return{value,error:null,retry:vi.fn()};}}});
beforeEach(()=>{vi.clearAllMocks();vi.mocked(publishGet).mockResolvedValue({campaign:{createdAt:"2026-09-29T00:00:00Z"},assignments:[{id:"a",udid:"device",state:"failedBeforeDispatch",effectIntent:null},{id:"other",udid:"other-device",state:"failedBeforeDispatch",effectIntent:null}]} as never);vi.mocked(publishRecoveryCapabilities).mockResolvedValue([{assignmentId:"a",revision:8,retryBeforePost:{allowed:true,reason:null},checkLink:{allowed:false,reason:null},resumeVerification:{allowed:false,reason:null}}] as never);});
it("retries only the selected device once with CAS and one request identity",async()=>{
 let done!:()=>void;vi.mocked(publishRetryAssignment).mockImplementation(()=>new Promise<void>(resolve=>{done=resolve;}));render(<PublishDeviceRecovery campaignId="campaign" udid="device"/>);
 const button=await screen.findByRole("button",{name:"Thử lại"});fireEvent.click(button);fireEvent.click(button);
 expect(publishRetryAssignment).toHaveBeenCalledExactlyOnceWith("a",true,8,expect.any(String));done();await screen.findByText(/Đã nhận thử lại đúng một lần/);
});
it("shows wait count and blocks manual racing with auto recovery",async()=>{
 vi.mocked(publishRecoveryCapabilities).mockResolvedValue([{assignmentId:"a",revision:8,retryBeforePost:{allowed:false,reason:"activePipeline"},recovery:{state:"retryWaiting",step:"sound",retriesUsed:2,maxRetries:3},checkLink:{allowed:false},resumeVerification:{allowed:false}}] as never);
 render(<PublishDeviceRecovery campaignId="campaign" udid="device"/>);await screen.findByText("Thử lại 2/3 · Chọn nhạc");expect(screen.getByRole("button",{name:"Thử lại"})).toBeDisabled();
});
it.each([
 {name:"submitted", evidence:null, expected:"Chưa rõ lịch kiểm tra tiếp theo.", action:"Kiểm tra liên kết"},
 {name:"pending future", evidence:{verificationStatus:{state:"pending",nextCheckAt:"2099-01-01T00:00:00Z"}}, expected:"Chờ lịch kiểm tra từ", action:"Kiểm tra liên kết"},
 {name:"pending overdue", evidence:{verificationStatus:{state:"pending",nextCheckAt:"2020-01-01T00:00:00Z"}}, expected:"Đã đến lịch kiểm tra", action:"Kiểm tra liên kết"},
 {name:"review", evidence:{verificationStatus:{state:"needsReview",reason:"Cần đối chiếu bằng chứng"},verificationBudget:{noProgressObservations:3,limit:3,metadataObservations:0,metadataLimit:12}}, expected:"Cần xem xét · Cần đối chiếu bằng chứng", action:"Tiếp tục kiểm tra link"},
 {name:"malformed", evidence:"{", expected:"Chưa rõ lịch kiểm tra tiếp theo.", action:"Kiểm tra liên kết"},
 {name:"invalid fields", evidence:{verificationStatus:{state:"pending",nextCheckAt:123},verificationBudget:{noProgressObservations:-1,limit:3}}, expected:"Chưa rõ lịch kiểm tra tiếp theo.", action:"Kiểm tra liên kết"},
 {name:"oversized", evidence:" ".repeat(262145), expected:"Chưa rõ lịch kiểm tra tiếp theo.", action:"Kiểm tra liên kết"},
 {name:"verified", evidence:{verificationStatus:{state:"needsReview",reason:"obsolete review"}}, expected:"Bài đã xác minh · Sheet đã ghi.", action:null},
])("submitted work projects $name without Post retry",async({name,evidence,expected,action})=>{
 const verified=name==="verified";
 vi.mocked(operationDeviceLog).mockResolvedValue({entries:[{action:"publishStep",state:"post_uncertain",text:"old Post uncertainty",at:"2026-09-29T00:00:00Z"}],truncated:false} as never);
 vi.mocked(publishGet).mockResolvedValue({campaign:{createdAt:"2026-09-29T00:00:00Z"},assignments:[{id:"a",ordinal:0,udid:"device",state:verified?"succeeded":"verifying",effectIntent:"post",errorCode:"account_mismatch",sheetDelivery:verified?{state:"sent"}:null,evidenceJson:typeof evidence==="string"?evidence:JSON.stringify(evidence)}]} as never);
 vi.mocked(publishRecoveryCapabilities).mockResolvedValue([{assignmentId:"a",revision:9,retryBeforePost:{allowed:false},checkLink:{allowed:!verified,reason:verified?"alreadyVerified":null},resumeVerification:{allowed:name==="review"},recovery:verified?{state:"exhausted",step:"sound",lastError:"obsolete recovery",lastErrorCode:"account_mismatch"}:null}] as never);
 render(<PublishDeviceRecovery campaignId="campaign" udid="device"/>);
 await screen.findByText(text=>text.startsWith(expected));
 if(verified){
  expect(screen.getByText(/8\/8 bước đã xác nhận/)).toBeInTheDocument();
  expect(screen.queryByText(/old Post uncertainty|obsolete recovery|obsolete review|Tài khoản đang mở khác|Đã hết lượt tự thử/)).not.toBeInTheDocument();
 }else{
  expect(screen.getByText(/Chưa rõ %/)).toBeInTheDocument();
  if(name==="review")expect(screen.getByText("Không có tiến triển 3/3 · Metadata 0/12")).toBeInTheDocument();
  if(name==="invalid fields")expect(screen.queryByText(/Không có tiến triển/)).not.toBeInTheDocument();
 }
 if(action){fireEvent.click(screen.getByRole("button",{name:action}));await waitFor(()=>name==="review"?expect(publishResumeVerification).toHaveBeenCalledWith("a",true,9):expect(publishCheckLinks).toHaveBeenCalledWith("campaign","device"));}
 expect(screen.queryByRole("button",{name:"Thử lại"})).not.toBeInTheDocument();
 expect(publishRetryAssignment).not.toHaveBeenCalled();
});
