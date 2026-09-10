import { cleanup,fireEvent,render,screen,waitFor } from "@testing-library/react";
import { afterEach,expect,it,vi } from "vitest";
import { PublishScheduleRetime } from "./PublishScheduleRetime";
import { publishScheduleReschedule } from "../../api";
import type { PublishCampaignRecord } from "../../types";
vi.mock("../../api",()=>({publishScheduleReschedule:vi.fn()}));
afterEach(()=>{cleanup();vi.clearAllMocks();});
const campaign={id:"schedule-1",state:"scheduled",runAt:"2099-09-10T20:00:00",updatedAt:"2026-09-10T00:00:00Z"} as PublishCampaignRecord;
it("saves a new time with the observed revision and reports concurrent start failures",async()=>{
 vi.mocked(publishScheduleReschedule).mockRejectedValueOnce(new Error("lịch đã bắt đầu chạy"));
 const onSaved=vi.fn();render(<PublishScheduleRetime campaign={campaign} onSaved={onSaved}/>);
 fireEvent.click(screen.getByRole("button",{name:"Đổi giờ"}));
 fireEvent.change(screen.getByLabelText("Giờ bắt đầu mới"),{target:{value:"2099-09-11T10:00"}});
 fireEvent.click(screen.getByRole("button",{name:"Lưu giờ mới"}));
 await screen.findByRole("alert");expect(publishScheduleReschedule).toHaveBeenCalledWith(campaign.id,campaign.updatedAt,"2099-09-11T10:00");
 expect(onSaved).not.toHaveBeenCalled();
 vi.mocked(publishScheduleReschedule).mockResolvedValue(campaign);
 fireEvent.click(screen.getByRole("button",{name:"Lưu giờ mới"}));
 await waitFor(()=>expect(onSaved).toHaveBeenCalledOnce());
});
it("never offers retiming once a job has started",()=>{
 render(<PublishScheduleRetime campaign={{...campaign,state:"posting"}} onSaved={()=>{}}/>);
 expect(screen.queryByRole("button")).toBeNull();
});
