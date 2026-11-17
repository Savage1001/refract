import playwright = require('playwright');
import type {ElectronApplication} from 'playwright';
const {_electron:electron}=playwright;
type LaunchOptions=NonNullable<Parameters<typeof electron.launch>[0]>;

// Native windows can be clamped to a runner's small display on construction.
// Resize after launch so desktop tests begin in the documented split layout;
// individual responsive tests can still resize the real window themselves.
async function launchTestDesktop(options:LaunchOptions):Promise<ElectronApplication> {
 const app=await electron.launch(options);
 try {await prepareTestDesktop(app);return app;}
 catch(error) {await app.close().catch(()=>{});throw error;}
}
async function prepareTestDesktop(app:ElectronApplication):Promise<void> {
 const page=await app.firstWindow();
 await page.waitForLoadState('load');
 await app.evaluate(({BrowserWindow})=>{
  const window=BrowserWindow.getAllWindows()[0];
  if(window.isVisible())throw new Error('Desktop tests must keep their windows hidden');
  window.setSize(1440,900);window.webContents.setBackgroundThrottling(false);
 });
 await page.waitForFunction(()=>window.innerWidth>1200);
}

// These tests replace assistant execution; discovery must use the same fixture
// rather than depending on authenticated CLIs installed on the host machine.
async function useFakeCodex(app:ElectronApplication):Promise<void> {
 await app.evaluate(({ipcMain})=>{
  ipcMain.removeHandler('providers:discover');ipcMain.handle('providers:discover',()=>({codex:true,opencode:false}));
  ipcMain.removeHandler('codex:discover-models');ipcMain.handle('codex:discover-models',()=>({ok:true,models:[{id:'fixture',model:'fixture',displayName:'Fixture model',description:'Test fixture',supportedReasoningEfforts:[],defaultReasoningEffort:null,isDefault:true}]}));
 });
}
export={launchTestDesktop,prepareTestDesktop,useFakeCodex};
