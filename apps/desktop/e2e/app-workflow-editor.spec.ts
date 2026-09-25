import { test, expect } from "@playwright/test";
import { installTauriMock } from "./fixtures/tauriMock";
import type { AppWorkflowV1 } from "../src/appWorkflow";

for(const width of [1440,820])test(`app editor preserves graph, positions, config and revisions at ${width}`,async({page})=>{
 await page.setViewportSize({width,height:900});await installTauriMock(page);
 await page.addInitScript(()=>{const w=window as unknown as {__TAURI_INTERNALS__:{invoke:(c:string,a?:Record<string,unknown>)=>Promise<unknown>};__appSaved?:AppWorkflowV1};const invoke=w.__TAURI_INTERNALS__.invoke;
 w.__TAURI_INTERNALS__.invoke=async(c,a={})=>{if(c==='app_workflow_save'){const doc=structuredClone(a.document) as AppWorkflowV1;if((w.__appSaved?.revision??null)!==a.expectedRevision)throw new Error('revision conflict');w.__appSaved={...doc,revision:(w.__appSaved?.revision??0)+1};return structuredClone(w.__appSaved);}if(c==='app_workflow_validate')return null;return invoke(c,a);};});
 await page.goto('/');await page.getByRole('button',{name:'My Apps',exact:true}).click();await page.getByRole('button',{name:'Thêm Flow',exact:true}).first().click();
 await expect(page.locator('.app-step')).toHaveCount(12);await expect(page.locator('.aside')).toHaveAttribute('data-rail-collapsed','true');
 await page.getByRole('button',{name:'Lưu',exact:true}).click();await expect(page.getByText('Đã lưu bản 1',{exact:true})).toBeVisible();
 const initial=await page.evaluate(()=>(window as unknown as {__appSaved:AppWorkflowV1}).__appSaved);
 const like=page.locator('.app-step-like');await like.click();
 await page.getByLabel('Tỷ lệ thực hiện (%)',{exact:true}).fill('57');
 await page.getByRole('button',{name:'Lưu',exact:true}).click();await expect(page.getByText('Đã lưu bản 2',{exact:true})).toBeVisible();
 const saved=await page.evaluate(()=>(window as unknown as {__appSaved:AppWorkflowV1}).__appSaved);
 expect(saved.nodes.find(n=>n.action==='like')!.config.probability).toBe(57);expect(saved.edges).toEqual(initial.edges);
 await page.getByRole('button',{name:'Xóa bước',exact:true}).click();await expect(page.locator('.app-step')).toHaveCount(11);
 await page.getByRole('button',{name:'Hoàn tác',exact:true}).click();await expect(page.locator('.app-step')).toHaveCount(12);
 await page.screenshot({path:test.info().outputPath(`app-editor-${width}.png`)});
 expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
});
