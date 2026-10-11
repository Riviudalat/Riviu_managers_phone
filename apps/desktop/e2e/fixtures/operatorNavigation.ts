import type { Page } from "@playwright/test";

// Test names (some from before the Vietnamese menu) → the sidebar entry and, for an entry with
// tabs, the tab that opens the page.
const ROUTES: Record<string, { entry: string; tab?: string }> = {
  "Thiết bị": { entry: "Trung tâm điều khiển" },
  "Flow": { entry: "Flow thiết bị" },
  "Tác vụ": { entry: "Lượt chạy" },
  "My Apps": { entry: "Ứng dụng", tab: "Quy trình ứng dụng" },
  "Ứng dụng của tôi": { entry: "Ứng dụng", tab: "Quy trình ứng dụng" },
  "Trung tâm ứng dụng": { entry: "Ứng dụng", tab: "Gói cài đặt" },
};

export async function openOperatorPage(page:Page,name:string){
  const { entry, tab } = ROUTES[name] ?? { entry: name };
  const navigation=page.getByRole("navigation",{name:"Điều hướng chính"});
  // The group check below reads the sidebar as it is now, so wait until it has rendered.
  await navigation.waitFor();
  // Short windows start with the groups folded; open the one holding the entry (pinned ones have none).
  const section=navigation.locator(".menu-group",{has:page.locator(`.menu-item[aria-label="${entry}"]`)});
  const group=section.locator(".sidebar-group-toggle");
  // The narrow rail shows every group without its heading, so there is nothing to open there.
  if(await group.count() && await group.isVisible() && await group.getAttribute("aria-expanded")==="false"){
    await group.click();
    // Let the group finish opening, so the entry is clicked (and scrolled to) at its final place.
    await section.locator(".sidebar-group-content").evaluate((content)=>Promise.all(content.getAnimations().map((animation)=>animation.finished)));
  }
  await navigation.getByRole("button",{name:entry,exact:true}).click();
  if(!tab)return;
  // The entry opens its first tab; click another only when it is not already the open one.
  const tabButton=page.getByRole("navigation",{name:"Trang Ứng dụng"}).getByRole("button",{name:tab,exact:true});
  if(await tabButton.getAttribute("aria-current")!=="page")await tabButton.click();
}
