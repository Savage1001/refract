const {launchTestDesktop}=require('./electron-fixture.cjs');
const {test}=require('node:test');
const assert=require('node:assert/strict');
const fs=require('node:fs/promises');
const os=require('node:os');
const path=require('node:path');
declare const store:any;
declare const pendingDraftSaves:Map<string,unknown>;
for(const scenario of ['reopen info','change engine','return home']) test(`draft save synchronizes settings after ${scenario}`, {timeout:60000}, async t=>{
 const data=await fs.mkdtemp(path.join(os.tmpdir(),'refract-draft-ui-'));
 const env:NodeJS.ProcessEnv={...process.env,REFRACT_DATA_DIR:data,REFRACT_TEST_HIDDEN:'1'};delete env.ELECTRON_RUN_AS_NODE;
 const app=await launchTestDesktop({args:[path.resolve(__dirname,'../..')],env});
 t.after(async()=>{await app.close().catch(()=>{});await fs.rm(data,{recursive:true,force:true,maxRetries:5,retryDelay:200});});
 const page=await app.firstWindow();await page.locator('#new').click();await page.locator('#source').waitFor();
 await app.evaluate(({ipcMain})=>{
   const original=(ipcMain as any)._invokeHandlers.get('projects:draft');
   ipcMain.removeHandler('projects:draft');
   ipcMain.handle('projects:draft',async(...args)=>{
     await new Promise<void>(resolve=>{(globalThis as any).releaseDraftSave=resolve;});
     return original(...args);
   });
 });
 await page.locator('#tools').click();await page.locator('#tool-info').click();
 await page.getByRole('checkbox',{name:/Draft mode/}).check();
 if(scenario==='change engine') {
   await app.evaluate(({ipcMain})=>{
     const original=(ipcMain as any)._invokeHandlers.get('projects:engine');ipcMain.removeHandler('projects:engine');
     ipcMain.handle('projects:engine',async(...args)=>{await new Promise<void>(resolve=>{(globalThis as any).releaseEngineSave=resolve;});return original(...args);});
   });
   await page.getByRole('combobox',{name:'LaTeX engine'}).selectOption('xelatex');
 }
 if(scenario==='return home') {await page.locator('#home').click();await page.locator('#list article').first().waitFor();}
 else if(scenario==='reopen info') {await page.locator('#tool-logs').click();await page.locator('#tool-info').click();}
 if(scenario!=='return home') assert.equal(await page.getByRole('checkbox',{name:/Draft mode/}).isDisabled(),true);
 await app.evaluate(()=>{(globalThis as any).releaseDraftSave();});
 await page.waitForFunction(()=>pendingDraftSaves.size===0);
 if(scenario==='change engine') {
   await app.evaluate(()=>{(globalThis as any).releaseEngineSave();});
   await page.waitForFunction(()=>{const input=document.querySelector('select[aria-label="LaTeX engine"]') as HTMLSelectElement;return input?.value==='xelatex' && !input.disabled;});
 }
 if(scenario==='return home') {await page.locator('#list article').first().click();await page.locator('#source').waitFor();await page.locator('#tools').click();await page.locator('#tool-info').click();}
 await page.waitForFunction(()=>{const input=document.querySelector('#tool-body input[type="checkbox"]') as HTMLInputElement;return input?.checked && !input.disabled;});
 assert.equal(await page.evaluate(async()=>{const [p]=await store.list();return p.draftMode;}),true);
});
