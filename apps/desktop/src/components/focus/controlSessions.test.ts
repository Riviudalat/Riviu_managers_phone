import { expect, it, vi } from "vitest";
import { acquireControlSession } from "./controlSessions";
import { deviceControlBegin, deviceControlEnd } from "../../api";
vi.mock("../../api",()=>({deviceControlBegin:vi.fn(async()=>{}),deviceControlEnd:vi.fn(async()=>{})}));
it("shares one native session and closes only after the last window leaves",async()=>{
 const a=acquireControlSession("shared-phone"),b=acquireControlSession("shared-phone");await Promise.all([a.ready,b.ready]);
 expect(deviceControlBegin).toHaveBeenCalledExactlyOnceWith("shared-phone");a.release();await Promise.resolve();expect(deviceControlEnd).not.toHaveBeenCalled();b.release();await vi.waitFor(()=>expect(deviceControlEnd).toHaveBeenCalledExactlyOnceWith("shared-phone"));b.release();expect(deviceControlEnd).toHaveBeenCalledTimes(1);
});
