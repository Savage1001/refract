const {launchTestDesktop}=require('./electron-fixture.cjs');
const {test}=require('node:test');
const assert=require('node:assert/strict');
const fs=require('node:fs/promises'), os=require('node:os'), path=require('node:path');
test('project first page appears in list and tiles', {timeout:120000}, async()=>{
 const data=await fs.mkdtemp(path.join(os.tmpdir(),'refract-thumbnail-'));
 const {ProjectStore}=require('../src/project-store.cjs');
 const store=new ProjectStore(path.join(data,'projects')); await store.create('First page preview');
 const env:NodeJS.ProcessEnv={...process.env,REFRACT_DATA_DIR:data,REFRACT_TEST_HIDDEN:'1'};delete env.ELECTRON_RUN_AS_NODE;
 const app=await launchTestDesktop({args:[path.resolve(__dirname,'../..')],env});
 try {
 const page=await app.firstWindow();await page.locator('#list article').click();
 await page.getByText('Compiled',{exact:true}).waitFor({timeout:90000});
 await page.reload();
 await page.locator('.project-page-thumbnail').waitFor();
 const first = await page.locator('.project-page-thumbnail').getAttribute('src');
 // Cached thumbnails must be attached in the same synchronous render, even if IPC stalls.
 await app.evaluate(({ipcMain})=>{ipcMain.removeHandler('projects:preview');ipcMain.handle('projects:preview',()=>new Promise(()=>{}));});
 assert.equal(await page.evaluate(()=>{eval('renderProjects()');return document.querySelector('.project-page-thumbnail')?.getAttribute('src');}),first);
 await page.locator('#grid-view').click();
 assert.equal(await page.locator('.project-page-thumbnail').getAttribute('src'),first);
 await page.reload();
 await page.locator('#list article').waitFor();
 assert.equal(await page.locator('.project-page-thumbnail').getAttribute('src'),first);
 // Changed PDF bytes must replace the cached fingerprint after validation.
 const before = await page.evaluate(()=>JSON.parse(localStorage.getItem('refract-project-thumbnails-v1')!)[0][1].fingerprint);
 const projectDirs = (await fs.readdir(path.join(data,'projects'))).filter((name:string)=>/^[0-9a-f-]{36}$/.test(name));
 const projectPath = path.join(data,'projects',projectDirs[0],'.refract-build','main.pdf');
 const changedPdf = Buffer.concat([await fs.readFile(projectPath),Buffer.from('\n% Updated PDF\n')]).toString('base64');
 await app.evaluate(({ipcMain},pdf)=>{ipcMain.removeHandler('projects:preview');ipcMain.handle('projects:preview',()=>pdf);},changedPdf);
 await page.evaluate(()=>eval('pendingThumbnails.clear();renderProjects()'));
 await page.waitForFunction(old=>JSON.parse(localStorage.getItem('refract-project-thumbnails-v1')!)[0][1].fingerprint!==old,before);
 // Missing output invalidates a persisted thumbnail after background validation.
 await app.evaluate(({ipcMain})=>{ipcMain.removeHandler('projects:preview');ipcMain.handle('projects:preview',()=>null);});
 await page.evaluate(()=>eval('pendingThumbnails.clear();renderProjects()'));
 await page.locator('.project-page-thumbnail').waitFor({state:'detached'});

 } finally {await app.close();await fs.rm(data,{recursive:true,force:true});}
});

