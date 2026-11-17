import test = require('node:test');
import assert = require('node:assert/strict');
import fs = require('node:fs/promises');
import os = require('node:os');
import path = require('node:path');
import playwright = require('playwright');
import fixture = require('./electron-fixture.cjs');
const {_electron:electron}=playwright;
const {prepareTestDesktop,useFakeCodex}=fixture;

test('hidden desktop fixture recovers the wide layout and supplies isolated provider discovery',{timeout:30000},async t=>{
 const data=await fs.mkdtemp(path.join(os.tmpdir(),'refract-desktop-fixture-'));
 const env:NodeJS.ProcessEnv={...process.env,REFRACT_DATA_DIR:data,REFRACT_TEST_HIDDEN:'1'};delete env.ELECTRON_RUN_AS_NODE;
 const app=await electron.launch({args:[path.resolve(__dirname,'../..')],env});
 t.after(async()=>{await app.close().catch(()=>{});await fs.rm(data,{recursive:true,force:true,maxRetries:5,retryDelay:200});});
 const page=await app.firstWindow();
 await app.evaluate(({BrowserWindow})=>BrowserWindow.getAllWindows()[0].setSize(1024,700));
 await page.waitForFunction(()=>window.innerWidth<=1200);
 await prepareTestDesktop(app);
 assert.equal(await page.evaluate(()=>document.readyState==='complete' && typeof (document.querySelector('#new') as HTMLButtonElement).onclick==='function'),true);
 assert.equal(await page.evaluate(()=>window.innerWidth>1200),true);
 assert.equal(await app.evaluate(({BrowserWindow})=>BrowserWindow.getAllWindows()[0].isVisible()),false);
 await useFakeCodex(app);
 const discovery=await page.evaluate(async()=>{
  const store=(window as unknown as {store:{discoverProviders():Promise<unknown>}}).store;
  return store.discoverProviders();
 });
 assert.deepEqual(discovery,{codex:true,opencode:false});
});
