import { describe, expect, it } from "vitest";
import {
  publishPreflightProblem,
  publishTikTokBuildLabel,
} from "./publishPreflightDisplay";

describe("publish preflight operator wording", () => {
  it("names a competing host and the USB repair before a publish run starts", () => {
    const issue={code:"automation_transport_conflict",message:"Máy đang cắm USB nhưng còn máy khác điều khiển ADB qua mạng: 192.168.1.43. Ngắt kết nối ở máy phụ."};
    expect(publishPreflightProblem(issue)).toEqual({title:"Kết nối điều khiển cần kiểm tra",action:issue.message});
  });
  it("shows actual global and Asia versions without promoting unsupported builds", () => {
    expect(
      publishTikTokBuildLabel({
        packageName: "com.zhiliaoapp.musically",
        version: "45.7.3",
        locale: "en-US",
      }),
    ).toContain("TikTok quốc tế · Phiên bản 45.7.3");
    expect(
      publishTikTokBuildLabel({
        packageName: "com.zhiliaoapp.musically",
        version: "45.7.3",
        locale: "en-US",
      }),
    ).toContain("en-US");
    expect(
      publishTikTokBuildLabel({
        packageName: "com.ss.android.ugc.trill",
        version: "38.3.2",
        locale: "en",
      }),
    ).toContain("TikTok châu Á · Phiên bản 38.3.2");
    expect(publishTikTokBuildLabel({})).toBe(
      "Chưa xác định bản TikTok · Chưa đọc được phiên bản · Chưa đọc được ngôn ngữ",
    );
    expect(
      publishTikTokBuildLabel({
        packageName: "unknown.package",
        locale: "not_a_locale_!!",
      }),
    ).toContain("Ứng dụng TikTok khác");
  });

  it("distinguishes multiple installed packages from an unreadable build", () => {
    const ambiguous = publishPreflightProblem({
      code: "tiktok_build_unreadable",
      message:
        "phone: more than one measured TikTok build is installed, and none of them is in the foreground to break the tie",
    });
    expect(ambiguous.title).toContain("nhiều bản TikTok");
    expect(ambiguous.action).toContain("Mở đúng bản TikTok");
    expect(ambiguous.action).toContain("Không cần gỡ");
    expect(
      publishPreflightProblem({
        code: "tiktok_build_unreadable",
        message: "transport disconnected",
      }).title,
    ).toBe("Chưa đọc được bản TikTok trên máy");
  });

  it("maps measured-build and disconnected-machine failures to different next actions", () => {
    for (const code of ["composer_unmeasured", "sound_picker_unmeasured"]) {
      const result = publishPreflightProblem({
        code,
        message: "raw locator failure",
      });
      expect(result.title).toContain("Chưa hỗ trợ");
      expect(result.action).toContain("hiệu chỉnh");
      expect(JSON.stringify(result)).not.toContain("raw locator");
    }
    expect(
      publishPreflightProblem({
        code: "device_missing",
        message: "not in roster",
      }).action,
    ).toContain("kết nối USB/Wi-Fi");
    expect(
      publishPreflightProblem({
        code: "future_unknown",
        message: "sensitive raw runtime text",
      }).action,
    ).toContain("Chi tiết kỹ thuật");
  });
});
