import {render,screen,fireEvent,waitFor} from "@testing-library/react";
import {vi,it,expect,beforeEach} from "vitest";
import {PublishDeviceRecovery} from "./PublishDeviceRecovery";
import {publishGet,publishRecoveryCapabilities,publishRetryAssignment,publishCheckLinks} from "../../api";
vi.mock("../../api",()=>({listDevices:vi.fn(async()=>[{udid:"device",status:"ready"}]),publishRetrySheetAssignment:vi.fn(),publishGet:vi.fn(),publishRecoveryCapabilities:vi.fn(),publishRetryAssignment:vi.fn(),publishCheckLinks:vi.fn(),publishResumeVerification:vi.fn()}));
vi.mock("./useMonitorRead",async()=>{const React=await import("react");return{useMonitorRead:(read:()=>Promise<unknown>)=>{const [value,setValue]=React.useState<unknown>(null);React.useEffect(()=>{void read().then(setValue);},[read]);return{value,error:null,retry:vi.fn()};}}});
beforeEach(()=>{vi.clearAllMocks();vi.mocked(publishGet).mockResolvedValue({assignments:[{id:"a",udid:"device",state:"failedBeforeDispatch",effectIntent:null},{id:"other",udid:"other-device",state:"failedBeforeDispatch",effectIntent:null}]} as never);vi.mocked(publishRecoveryCapabilities).mockResolvedValue([{assignmentId:"a",revision:8,retryBeforePost:{allowed:true,reason:null},checkLink:{allowed:false,reason:null},resumeVerification:{allowed:false,reason:null}}] as never);});
it("retries only the selected device once with CAS and one request identity",async()=>{
 let done!:()=>void;vi.mocked(publishRetryAssignment).mockImplementation(()=>new Promise<void>(resolve=>{done=resolve;}));render(<PublishDeviceRecovery campaignId="campaign" udid="device"/>);
 const button=await screen.findByRole("button",{name:"Thử lại"});fireEvent.click(button);fireEvent.click(button);
 expect(publishRetryAssignment).toHaveBeenCalledExactlyOnceWith("a",true,8,expect.any(String));done();await screen.findByText(/Đã nhận thử lại đúng một lần/);
});
it("shows wait count and blocks manual racing with auto recovery",async()=>{
 vi.mocked(publishRecoveryCapabilities).mockResolvedValue([{assignmentId:"a",revision:8,retryBeforePost:{allowed:false,reason:"activePipeline"},recovery:{state:"retryWaiting",step:"sound",retriesUsed:2,maxRetries:3},checkLink:{allowed:false},resumeVerification:{allowed:false}}] as never);
 render(<PublishDeviceRecovery campaignId="campaign" udid="device"/>);await screen.findByText("Thử lại 2/3 · Chọn nhạc");expect(screen.getByRole("button",{name:"Thử lại"})).toBeDisabled();
});
it("submitted work offers link verification and never Post retry",async()=>{
 vi.mocked(publishGet).mockResolvedValue({assignments:[{id:"a",udid:"device",state:"verifying",effectIntent:"post"}]} as never);
 vi.mocked(publishRecoveryCapabilities).mockResolvedValue([{assignmentId:"a",revision:9,retryBeforePost:{allowed:false},checkLink:{allowed:true},resumeVerification:{allowed:false}}] as never);
 render(<PublishDeviceRecovery campaignId="campaign" udid="device"/>);fireEvent.click(await screen.findByRole("button",{name:"Kiểm tra liên kết"}));await waitFor(()=>expect(publishCheckLinks).toHaveBeenCalledWith("campaign","device"));expect(publishRetryAssignment).not.toHaveBeenCalled();
});
