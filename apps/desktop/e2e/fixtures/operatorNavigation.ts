import type { Page } from "@playwright/test";

export async function openOperatorPage(page:Page,name:string){
  if(["Nuôi TikTok","Tương tác","Đăng bài"].includes(name)){
    const navigation=page.getByRole("navigation",{name:"Điều hướng chính"});
    const group=navigation.getByRole("button",{name:"Automation",exact:true});
    if(await group.getAttribute("aria-expanded")==="false")await group.click();
    await navigation.getByRole("button",{name,exact:true}).click();
    return;
  }
  const label:Record<string,string>={"Flow":"Flow thiết bị","Tác vụ":"Lượt chạy","Thiết bị":"Control Center","Ứng dụng của tôi":"My Apps"};
  await page.getByRole("navigation",{name:"Điều hướng chính"}).getByRole("button",{name:label[name]??name,exact:true}).click();
}
