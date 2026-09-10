import { expect,it } from "vitest";
import { scheduleDateIssue } from "./publishScheduleTimes";
it("rejects impossible dates and past time, accepts future local date",()=>{
 const now=new Date("2099-01-01T00:00").getTime();
 expect(scheduleDateIssue("2099-02-30","12:00",now)).toBe("Ngày hoặc giờ đăng không hợp lệ");
 expect(scheduleDateIssue("2099-01-01","00:00",now)).toBe("Giờ đăng phải ở tương lai");
 expect(scheduleDateIssue("2099-01-01","00:01",now)).toBe("");
 expect(scheduleDateIssue("2099-01-01","25:00",now)).toBe("Ngày hoặc giờ đăng không hợp lệ");
});
