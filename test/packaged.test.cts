const {test} = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs/promises'); const os = require('node:os'); const path = require('node:path');
const {_electron: electron} = require('playwright');
test('packaged application contains no redistributed TeX', async t => {
 const root = path.resolve(__dirname, '../../dist/win-unpacked');
 try { await fs.access(path.join(root, 'Refract.exe')); } catch { return t.skip('Build is not present'); }
 const {assertNoTex} = require('../scripts/distribution-boundary.cjs');
 await assertNoTex(root);
});

test('packaged application excludes optional Node canvas binaries', async t => {
 const modules = path.resolve(__dirname, '../../dist/win-unpacked/resources/app/node_modules');
 try { await fs.access(path.join(modules, 'pdfjs-dist')); } catch { return t.skip('Build is not present'); }
 const nativeScope: string = path.join(modules, '@napi-rs');
 const entries: string[] = await fs.readdir(nativeScope).catch((error: NodeJS.ErrnoException) => {
  if (error.code === 'ENOENT') return [];
  throw error;
 });
 assert.deepEqual(entries.filter(name => name === 'canvas' || name.startsWith('canvas-')), []);
});
test('packaged application compiles a project', {timeout:90000}, async t => {
 const exe = process.env.REFRACT_TEST_MAC_APP || path.resolve(__dirname,'../../dist/win-unpacked/Refract.exe');
 if (process.env.REFRACT_TEST_MAC_APP) await fs.access(exe);
 else try { await fs.access(exe); } catch { return t.skip('Build is not present'); }
 const data = await fs.mkdtemp(path.join(os.tmpdir(),'refract-packaged-'));
 const env: NodeJS.ProcessEnv = {...process.env,REFRACT_DATA_DIR:data, REFRACT_TEST_HIDDEN: '1'}; delete env.ELECTRON_RUN_AS_NODE;
 const app = await electron.launch({executablePath:exe,args:[],env});
 t.after(async()=>{ await app.close().catch(()=>{}); await fs.rm(data,{recursive:true,force:true,maxRetries:5,retryDelay:200}); });
 const page = await app.firstWindow(); const errors=[];
 page.on('pageerror',e=>errors.push(e.message)); page.on('dialog',d=>{errors.push(d.message());d.dismiss();});
 await page.locator('#new').click(); await page.locator('#source').waitFor({state:'visible'});
 await page.getByText('Compiled',{exact:true}).waitFor({timeout:30000});
 assert.equal(await page.locator('#pdf-area canvas').count(),1);
 const source=await page.locator('#source').inputValue();
 const font = process.platform === 'darwin' ? 'Helvetica' : 'Latin Modern Roman';
 await page.locator('#source').fill(source.replace('\\begin{document}',`\\usepackage{fontspec}\n\\setmainfont{${font}}\n\\begin{document}`));
 await page.locator('#tools').click(); await page.locator('#tool-info').click();
 await page.getByRole('combobox',{name:'LaTeX engine'}).selectOption('xelatex');
 await page.waitForFunction(()=>{const e=document.querySelector('select[aria-label="LaTeX engine"]') as HTMLSelectElement;return e && !e.disabled && e.value==='xelatex';});
 await page.locator('#tools').click();
 try {
  await page.getByText('Compiled',{exact:true}).waitFor({timeout:45000});
 } catch (error) {
  const log = await page.evaluate(async () => {
   const [project] = await store.list();
   const result = await store.compile(project.id);
   return result.log;
  }).catch(String);
  throw new Error(`XeLaTeX did not finish in the editor.\n${log}`, { cause: error });
 }
 await page.locator('#tools').click();await page.locator('#tool-logs').click();await page.getByText('Full build output',{exact:true}).click();assert.match(await page.locator('#tool-body').innerText(),/XeTeX/);
 assert.deepEqual(errors,[]);
});

test('packaged Windows application renders saved assistant formatting', {timeout:60000}, async t=>{
 const exe=path.resolve(__dirname,'../../dist/win-unpacked/Refract.exe');
 try {await fs.access(exe);} catch {return t.skip('Build is not present');}
 const data:string=await fs.mkdtemp(path.join(os.tmpdir(),'refract-packaged-format-'));
 const {ProjectStore}=require('../src/project-store.cjs'),{ChatService}=require('../src/chat.cjs');
 const projectStore=new ProjectStore(path.join(data,'projects')),project=await projectStore.create('Formatting');
 await new ChatService(projectStore).save(project.id,{sessionId:null,messages:[{role:'assistant',text:'**Ready** with $x^2$ and [paper](https://example.com).'}]});
 const env:NodeJS.ProcessEnv={...process.env,REFRACT_DATA_DIR:data,REFRACT_TEST_HIDDEN:'1'};delete env.ELECTRON_RUN_AS_NODE;
 const app=await electron.launch({executablePath:exe,args:[],env});
 t.after(async()=>{await app.close().catch(()=>{});await fs.rm(data,{recursive:true,force:true,maxRetries:5,retryDelay:200});});
 const page=await app.firstWindow();await page.locator('#list article').first().click();
 const reply=page.locator('.message.assistant .formatted-response');await reply.waitFor();
 assert.equal(await reply.locator('strong').innerText(),'Ready');assert.equal(await reply.locator('math').count(),1);
 assert.equal(await reply.getByRole('link',{name:'paper'}).getAttribute('href'),'https://example.com');
});

test('packaged Windows application navigates between source and PDF', {timeout:90000}, async t=>{
 const exe=path.resolve(__dirname,'../../dist/win-unpacked/Refract.exe');try{await fs.access(exe);}catch{return t.skip('Build is not present');}
 const data:string=await fs.mkdtemp(path.join(os.tmpdir(),'refract-packaged-synctex-'));
 const {ProjectStore}=require('../src/project-store.cjs'),projectStore=new ProjectStore(path.join(data,'projects')),project=await projectStore.create('SyncTeX');
 await projectStore.write(project.id,'main.tex','\\documentclass{article}\n\\begin{document}\nFirst page.\\newpage\n\\input{sections/body}\n\\end{document}\n');
 await projectStore.write(project.id,'sections/body.tex','Nested synchronized text.\n');
 const env:NodeJS.ProcessEnv={...process.env,REFRACT_DATA_DIR:data,REFRACT_TEST_HIDDEN:'1'};delete env.ELECTRON_RUN_AS_NODE;
 const app=await electron.launch({executablePath:exe,args:[],env});t.after(async()=>{await app.close().catch(()=>{});const resolved=await fs.realpath(data),parent=await fs.realpath(os.tmpdir());if(path.dirname(resolved).toLowerCase()!==parent.toLowerCase() || !path.basename(resolved).startsWith('refract-packaged-synctex-'))throw new Error('Unsafe cleanup path');await fs.rm(resolved,{recursive:true,force:true,maxRetries:5,retryDelay:200});});
 const page=await app.firstWindow(),dialogs:string[]=[];page.on('dialog',dialog=>{dialogs.push(dialog.message());void dialog.dismiss();});
 await page.locator('#list article').first().click();await page.getByText('Compiled',{exact:true}).waitFor({timeout:60000});await page.locator('#file-list [title="sections/body.tex"]').click();
 await page.getByRole('button',{name:'Show in PDF'}).click();await page.waitForFunction(()=>document.querySelector('.pdf-page-count')?.textContent==='2 / 2');
 const position=await page.evaluate(async id=>store.syncForward(id,'sections/body.tex',1),project.id);await page.locator('#file-list [title="main.tex"]').click();
 const canvas=page.locator('.pdf-pages canvas').nth(1);await canvas.scrollIntoViewIfNeeded();
 await canvas.evaluate((element:HTMLCanvasElement,p)=>{const bounds=element.getBoundingClientRect(),scale=Number(element.dataset.renderScale)||1.4;element.dispatchEvent(new MouseEvent('dblclick',{bubbles:true,clientX:bounds.left+p.x*bounds.width/(element.width/scale),clientY:bounds.top+p.y*bounds.height/(element.height/scale)}));},position);
 await page.waitForFunction(()=>{const el=document.querySelector('#source') as HTMLTextAreaElement;return el?.value==='Nested synchronized text.\n' && el.value.slice(el.selectionStart,el.selectionEnd)==='Nested synchronized text.';});assert.deepEqual(dialogs,[]);
});

test('packaged application exports and imports project files', {timeout:90000}, async t=>{
 const exe=path.resolve(__dirname,'../../dist/win-unpacked/Refract.exe');
 try {await fs.access(exe);} catch {return t.skip('Build is not present');}
 const data:string=await fs.mkdtemp(path.join(os.tmpdir(),'refract-packaged-archive-'));
 const env:NodeJS.ProcessEnv={...process.env,REFRACT_DATA_DIR:data,REFRACT_TEST_HIDDEN:'1'};delete env.ELECTRON_RUN_AS_NODE;
 const app=await electron.launch({executablePath:exe,args:[],env});
 t.after(async()=>{await app.close().catch(()=>{});await fs.rm(data,{recursive:true,force:true,maxRetries:5,retryDelay:200});});
 const page=await app.firstWindow();await page.locator('#new').click();await page.locator('#source').waitFor({state:'visible'});
 const source=await page.locator('#source').inputValue();
 const id=await page.evaluate(async()=>{const [project]=await store.list();await store.createFolder(project.id,'nested');await store.createFile(project.id,'nested/asset.bin');await store.write(project.id,'nested/asset.bin','archive content');return project.id;});
 const archive=path.join(data,'roundtrip.zip');
 await app.evaluate(({dialog},file)=>{dialog.showSaveDialog=async()=>({canceled:false,filePath:file});dialog.showOpenDialog=async()=>({canceled:false,filePaths:[file]});},archive);
 await page.evaluate(id=>store.exportProject(id),id);
 const imported=await page.evaluate(()=>store.importProject());
 assert.notEqual(imported.id,id);
 assert.equal(await page.evaluate(id=>store.read(id,'main.tex'),imported.id),source);
 assert.equal(await page.evaluate(id=>store.read(id,'nested/asset.bin'),imported.id),'archive content');
});

test('packaged application follows light and dark themes', {timeout:60000}, async t=>{
 const exe=path.resolve(__dirname,'../../dist/win-unpacked/Refract.exe');try {await fs.access(exe);} catch {return t.skip('Build is not present');}
 const data:string=await fs.mkdtemp(path.join(os.tmpdir(),'refract-packaged-theme-'));
 const env:NodeJS.ProcessEnv={...process.env,REFRACT_DATA_DIR:data,REFRACT_TEST_HIDDEN:'1'};delete env.ELECTRON_RUN_AS_NODE;
 const app=await electron.launch({executablePath:exe,args:[],env});
 t.after(async()=>{await app.close().catch(()=>{});await fs.rm(data,{recursive:true,force:true,maxRetries:5,retryDelay:200});});
 const page=await app.firstWindow();
 const channels=()=>page.locator('body').evaluate(el=>(getComputedStyle(el).backgroundColor.match(/\d+/g)||[]).slice(0,3).map(Number));
 await page.emulateMedia({colorScheme:'light'});assert.ok((await channels()).every(value=>value>180));
 await page.emulateMedia({colorScheme:'dark'});assert.ok((await channels()).every(value=>value<80));
});

test('packaged application starts without TeX and explains the external dependency', {timeout:60000}, async t=>{
 const exe=path.resolve(__dirname,'../../dist/win-unpacked/Refract.exe');try {await fs.access(exe);} catch {return t.skip('Build is not present');}
 const data:string=await fs.mkdtemp(path.join(os.tmpdir(),'refract-packaged-no-tex-'));
 const env:NodeJS.ProcessEnv={...process.env,REFRACT_DATA_DIR:data,REFRACT_TEST_HIDDEN:'1'};delete env.ELECTRON_RUN_AS_NODE;delete env.REFRACT_TEX_BIN;
 const app=await electron.launch({executablePath:exe,args:[],env});
 t.after(async()=>{await app.close().catch(()=>{});await fs.rm(data,{recursive:true,force:true,maxRetries:5,retryDelay:200});});
 const page=await app.firstWindow();await page.locator('#new').click();await page.locator('#source').waitFor({state:'visible'});
 const failure:string=await page.evaluate(async()=>{const [project]=await store.list();try{await store.compile(project.id);return '';}catch(error){return String(error);}});
 assert.match(failure,/REFRACT_TEX_BIN/);assert.match(failure,/separately installed/);
 assert.match(await page.locator('#source').inputValue(),/documentclass/);
 assert.equal(app.process().exitCode,null);
});
