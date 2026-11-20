declare const store: any;
const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const os = require('node:os');
const path = require('node:path');
const { _electron: electron } = require('playwright');
const root = path.resolve(__dirname, '..', '..');
const {launchTestDesktop,useFakeCodex}=require('./electron-fixture.cjs');
const launchDesktop=async(env:NodeJS.ProcessEnv,fakeCodex=false)=>{
 const app=await launchTestDesktop(process.env.REFRACT_TEST_PACKAGED==='1'
  ? {executablePath:path.join(root,'dist','win-unpacked','Refract.exe'),args:[],env}
  : {args:[root],env});
 if(fakeCodex)await useFakeCodex(app);return app;
};

test('ZIP import offers an in-app choice of main LaTeX document', {timeout:60000}, async t => {
 const data = await fs.mkdtemp(path.join(os.tmpdir(), 'refract-ui-import-'));
 const archive = path.join(data, 'papers.zip');
 const Zip = require('adm-zip'), zip = new Zip();
 for (const file of ['first.tex', 'second.tex']) {
  zip.addFile(file, Buffer.from('\\documentclass{article}\n\\begin{document}\n' + file + '\n\\end{document}'));
 }
 zip.writeZip(archive);
 const env: NodeJS.ProcessEnv = {...process.env, REFRACT_DATA_DIR:data, REFRACT_TEST_HIDDEN:'1'};
 delete env.ELECTRON_RUN_AS_NODE;
 const app = await launchDesktop(env);
 t.after(async () => { await app.close(); await fs.rm(data, {recursive:true,force:true}); });
 await app.evaluate(({dialog}, file) => {
  dialog.showOpenDialog = async () => ({canceled:false, filePaths:[file]});
 }, archive);
 const page = await app.firstWindow();
 const picker = page.getByRole('dialog', {name:'Select the main LaTeX document'});
 await page.locator('#import').click();
 await picker.getByRole('button', {name:'Cancel'}).click();
 await picker.waitFor({state:'hidden'});
 assert.equal(await page.evaluate(async () => (await store.list()).length), 0);
 await page.locator('#import').click();
 await picker.getByRole('combobox', {name:'Main LaTeX document'}).selectOption({label:'second.tex'});
 await picker.getByRole('button', {name:'Import', exact:true}).click();
 await page.locator('#source').waitFor({state:'visible'});
 assert.match(await page.locator('#source').inputValue(), /second\.tex/);
});

test('multi-file paper edits and recompiles after restart', {timeout:120000}, async t => {
 const data:string=await fs.mkdtemp(path.join(os.tmpdir(),'refract-ui-paper-'));
 const {ProjectStore}=require('../src/project-store.cjs'), projectStore=new ProjectStore(path.join(data,'projects'));
 const project=await projectStore.create('Multi-file paper');
 await projectStore.write(project.id,'main.tex','\\documentclass{article}\n\\usepackage{graphicx}\n\\begin{document}\n\\input{sections/body}\n\\includegraphics[width=1cm]{figures/pixel.png}\n\\end{document}\n');
 await projectStore.write(project.id,'sections/body.tex','Original section text.\n');
 await fs.mkdir(path.join(projectStore.projectDir(project.id),'figures'),{recursive:true});
 await fs.writeFile(path.join(projectStore.projectDir(project.id),'figures','pixel.png'),Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR4nGP4z8AAAAMBAQDJ/pLvAAAAAElFTkSuQmCC','base64'));
 const env:NodeJS.ProcessEnv={...process.env,REFRACT_DATA_DIR:data,REFRACT_TEST_HIDDEN:'1'}; delete env.ELECTRON_RUN_AS_NODE;
 let app=await launchDesktop(env);
 t.after(async()=>{if(app) await app.close().catch(()=>{});await fs.rm(data,{recursive:true,force:true,maxRetries:5,retryDelay:200});});
 let page=await app.firstWindow(); await page.locator('#list article').first().click();
 await page.getByText('Compiled',{exact:true}).waitFor({timeout:60000});
 await page.locator('#file-list [title="sections/body.tex"]').click();
 await page.locator('#source').fill('Revised section text.\n');
 await page.getByText('Compiled',{exact:true}).waitFor({timeout:60000});
 await app.close(); app=null;
 app=await launchDesktop(env); page=await app.firstWindow();
 await page.locator('#list article').first().click();
 await page.locator('#file-list [title="sections/body.tex"]').click();
 assert.equal(await page.locator('#source').inputValue(),'Revised section text.\n');
 await page.getByText('Compiled',{exact:true}).waitFor({timeout:60000});
 assert.ok(await page.locator('#pdf-area canvas').count());
});
test('desktop creates, edits, compiles and reopens a project', { timeout: 90000 }, async t => {
 const data = await fs.mkdtemp(path.join(os.tmpdir(), 'refract-ui-'));
 const env: NodeJS.ProcessEnv = {...process.env, REFRACT_DATA_DIR:data, REFRACT_TEST_HIDDEN: '1'}; delete env.ELECTRON_RUN_AS_NODE;
 let app = await launchDesktop(env);
 t.after(async () => { if (app) await app.close().catch(() => {}); await fs.rm(data, {recursive:true,force:true, maxRetries:5, retryDelay:200}); });
 let page = await app.firstWindow();
 assert.equal(await page.locator('#project-navigation').isHidden(), true);
 assert.equal(await app.evaluate(({BrowserWindow}) => BrowserWindow.getAllWindows()[0].isMenuBarVisible()), false);
 const errors=[]; page.on('pageerror', e => errors.push(e.message));
 await page.emulateMedia({colorScheme:'dark'});
 assert.ok(await page.locator('body').evaluate(el => (getComputedStyle(el).backgroundColor.match(/\d+/g) || []).slice(0,3).every(channel => Number(channel) < 80)));
 await page.locator('#grid-view').click();
 assert.equal(await page.locator('#list').evaluate(el => el.classList.contains('grid')), true);
 await page.locator('#list-view').click();
 await page.locator('#new').click();
 await page.locator('#source').waitFor({state:'visible'});
 assert.equal(await page.locator('#home').innerText(), 'Refract');
 assert.equal(await page.locator('.app-titlebar').evaluate(el => getComputedStyle(el).backgroundColor), await page.locator('.sidebar').evaluate(el => getComputedStyle(el).backgroundColor));
 assert.ok(await page.locator('#home img').evaluate(el => (el as HTMLImageElement).naturalWidth > 0));
 const projectDirs = (await fs.readdir(path.join(data,'projects'))).filter((name:string)=>/^[0-9a-f-]{36}$/.test(name));
 await fs.writeFile(path.join(data, 'projects', projectDirs[0], 'pixel.png'), Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aJZkAAAAASUVORK5CYII=', 'base64'));
 await page.locator('#file-list button[title="pixel.png"]').waitFor({state:'visible'});
 await page.locator('#file-list button[title="pixel.png"] .tree-image-thumbnail').waitFor({state:'visible'});
 // Check the thumbnail and filename centers at different window widths and app zoom levels.
 for (const width of [900, 1440]) {
  for (const zoom of [0.8, 1, 1.25, 1.5]) {
   await app.evaluate(({BrowserWindow}, {width, zoom}) => {const win=BrowserWindow.getAllWindows()[0]; win.setSize(width, 900); win.webContents.setZoomFactor(zoom);}, {width, zoom});
   await page.evaluate(() => new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve()))));
   const centers=await page.locator('#file-list button[title="pixel.png"]').evaluate(button => {
    const image=button.querySelector('.tree-image-thumbnail').getBoundingClientRect(), text=button.querySelector('.tree-file-name').getBoundingClientRect();
    return {image:image.top+image.height/2, text:text.top+text.height/2};
   });
   assert.ok(Math.abs(centers.image-centers.text)<1, `Thumbnail alignment at width ${width}, zoom ${zoom}: ${JSON.stringify(centers)}`);
  }
 }
 await app.evaluate(({BrowserWindow}) => {const win=BrowserWindow.getAllWindows()[0]; win.webContents.setZoomFactor(1); win.setSize(1440, 900);});
 await page.locator('#file-list button[title="pixel.png"]').click();
 await page.locator('#project-image').waitFor({state:'visible'});
 assert.equal(await page.locator('#project-image').evaluate(el => el.naturalWidth), 1);
 assert.equal(await page.locator('#file-tabs .file-tab').count(), 2);
 await page.getByRole('button', {name:'main.tex', exact:true}).click();
 await page.locator('#source').waitFor({state:'visible'});
 await page.getByRole('button', {name:'Close pixel.png'}).click();
 assert.equal(await page.locator('#file-tabs .file-tab').count(), 1);
 await page.locator('#outline-toggle').click();
 assert.equal(await page.locator('#outline-list').isHidden(), true);
 await page.locator('#outline-toggle').click();
 await page.locator('#project-name').click();
 await page.locator('#rename-name').fill('Research paper');
 await page.locator('#rename-dialog button[value="rename"]').click();
 await page.getByText('Research paper', {exact:true}).waitFor();
 const source = await page.locator('#source').inputValue();
 assert.ok(await page.locator('#highlight .tex-command').count() > 0);
 await page.locator('#source').focus();
 await page.locator('#source').press('Control+End');
 await page.locator('#source').press('Tab');
 assert.ok((await page.locator('#source').inputValue()).endsWith('  '));
 await page.locator('#source').press('Control+z');
 assert.equal(await page.locator('#source').inputValue(), source);
 await page.locator('#source').fill(source.replace('Introduction', 'Test Section'));
 await page.getByRole('textbox', {name:'Search project'}).fill('Test Section');
 await page.locator('#file-search-results button').first().click();
 assert.equal(await page.locator('#source').evaluate(el => (el as HTMLInputElement).value.slice(el.selectionStart, el.selectionEnd)), 'Test Section');
 // Editing schedules compilation without a manual click.
 await page.getByText('Compiled', {exact:true}).waitFor({timeout:30000});
 assert.equal(await page.locator('#pdf-area canvas').count(), 1);
 assert.equal(await page.locator('#download').isEnabled(), true);
 const previewPdf: string = await page.evaluate(() => eval('compiledPdf'));
 const savedPdf = path.join(data,'download.pdf');
 await app.evaluate(({dialog}, file) => { dialog.showSaveDialog = async () => ({canceled:false, filePath:file}); }, savedPdf);
 await fs.writeFile(path.join(data,'projects',projectDirs[0],'.refract-build','main.pdf'),'different disk contents');
 await page.evaluate(pdf => store.savePdf(pdf), previewPdf);
 assert.deepEqual(await fs.readFile(savedPdf),Buffer.from(previewPdf,'base64'));
 const validSource = await page.locator('#source').inputValue();
 await page.locator('#source').fill(validSource.replace('\\end{document}', '\\undefinedrefractcommand\n\\end{document}'));
 assert.equal(await page.locator('#download').isDisabled(), true);
 assert.equal(await page.locator('#pdf-area canvas').count(), 0);
 await page.getByText('Errors', {exact:true}).waitFor({timeout:30000});
 assert.equal(await page.locator('#download').isDisabled(), true);
 await page.locator('#source').fill(validSource);
 await page.getByText('Compiled', {exact:true}).waitFor({timeout:30000});
 assert.equal(await page.locator('#download').isEnabled(), true);
 await page.locator('#tools').click();
 await page.locator('.raw-output summary').click(); assert.match(await page.locator('#tool-body').innerText(), /Output written on/); await page.getByRole('button',{name:'Collapse tools panel'}).click();
 await page.locator('#tool-info').click();
 assert.match(await page.locator('#tool-body').innerText(), /Research paper/);
 await page.locator('#source').evaluate(el => { const i = el.value.indexOf('Test Section'); el.focus(); el.setSelectionRange(i, i + 12); el.dispatchEvent(new Event('select'));  });
 await page.locator('#tool-comments').click();
 await page.getByRole('textbox', {name:'Add a comment'}).fill('Check this section');
 await page.getByRole('button', {name:'Comment', exact:true}).click();
 await page.getByText('Check this section', {exact:true}).waitFor();
 assert.match(await page.locator('.comment-reference').innerText(), /Test Section/);
 await page.getByRole('button', {name:'Resolve', exact:true}).click();
 await page.getByRole('button', {name:'Reopen', exact:true}).waitFor();
 await page.evaluate(async () => { const projects = await store.list(); await store.versionSnapshot(projects[0].id); });
 await page.locator('#tool-versions').click();
 await page.locator('.version-row').last().waitFor();
 await page.locator('.version-row').last().getByRole('button', {name:'Restore file'}).click(); await page.locator('#app-dialog-action').click();
 await page.waitForFunction(expected => (document.querySelector('#source') as HTMLTextAreaElement).value === expected, source);
 await page.locator('#source').fill(source.replace('Introduction', 'Test Section'));
 await page.locator('#tools').click();
 await page.getByText('Compiled', {exact:true}).waitFor({timeout:30000});
 assert.equal(await page.locator('#pdf-area canvas').isVisible(), true);
 assert.equal(await page.getByText('Welcome to Refract!').count(), 1);
 assert.equal(await page.getByText('Welcome to Prism!').count(), 0);
 assert.ok(await page.locator('.pdf-pane').evaluate(el => (getComputedStyle(el).backgroundColor.match(/\d+/g) || []).slice(0,3).every(channel => Number(channel) < 100)));
 await page.locator('#chat-input').focus(); await page.getByRole('button', {name:'Dismiss welcome'}).click();
 assert.equal(await page.getByText('Welcome to Refract!').count(), 0);
 await app.close(); app = null;
 app = await launchDesktop(env); page = await app.firstWindow();
 await page.getByText('Research paper', {exact:false}).waitFor();
 await page.locator('#list article').first().click();
 await page.locator('#source').waitFor({state:'visible'});
 assert.match(await page.locator('#source').inputValue(), /Test Section/);
 await page.locator('#duplicate').click();
 const duplicateDialog = page.getByRole('dialog', {name:/Duplicate Research paper/});
 await duplicateDialog.waitFor();
 assert.equal(await duplicateDialog.getByRole('button', {name:'Cancel'}).evaluate(button => button === document.activeElement), true);
 await page.keyboard.press('Escape');
 await duplicateDialog.waitFor({state:'hidden'});
 assert.equal(await page.locator('#duplicate').evaluate(button => button === document.activeElement), true);
 await page.locator('#duplicate').click();
 await duplicateDialog.getByRole('button', {name:'Duplicate', exact:true}).click();
 await page.getByRole('button',{name:'Research paper (copy)',exact:true}).waitFor();
 await page.locator('#source').waitFor({state:'visible'});
 assert.match(await page.locator('#source').inputValue(), /Test Section/);
 assert.deepEqual(errors, []);
});

test('a failed save prevents compilation until the same file is saved successfully', { timeout: 60000 }, async t => {
 const data = await fs.mkdtemp(path.join(os.tmpdir(), 'refract-save-fail-'));
 const env: NodeJS.ProcessEnv = {...process.env, REFRACT_DATA_DIR:data, REFRACT_TEST_HIDDEN:'1'}; delete env.ELECTRON_RUN_AS_NODE;
 const app = await launchDesktop(env,true);
 t.after(async () => { await app.close().catch(() => {}); await fs.rm(data,{recursive:true,force:true,maxRetries:5,retryDelay:200}); });
 await app.evaluate(({app})=>{
  const req=(process.getBuiltinModule('module') as any).createRequire(app.getAppPath()+'\\package.json');
  const {ChatService}=req('./build/src/chat.cjs');
  ChatService.prototype.send=async function(_id,prompt){(globalThis as any).__assistantRan=true;return {sessionId:null,messages:[{role:'user',text:prompt}]};};
 });
 const page = await app.firstWindow();
 await page.locator('#new').click();
 await page.getByText('Compiled',{exact:true}).waitFor({timeout:30000});
 const source = await page.locator('#source').inputValue();
 const [id] = (await fs.readdir(path.join(data,'projects'))).filter((name:string)=>/^[0-9a-f-]{36}$/.test(name));
 const file = path.join(data,'projects',id,'main.tex'), backup = path.join(data,'main-backup.tex');
 await fs.rename(file,backup); await fs.mkdir(file);
 await page.locator('#source').fill(source + '\nSave must fail');
 await page.getByRole('dialog',{name:/could not save file/i}).waitFor(); await page.locator('#app-dialog-action').click();
 await page.getByText('Errors',{exact:true}).waitFor({timeout:30000});
 assert.match(await page.locator('#pdf-area').innerText(),/Cannot compile until the file is saved/);
 assert.equal(await page.locator('#download').isDisabled(),true);
 await page.locator('#chat-input').fill('Keep this draft');const sendAttempt=page.locator('#send').click();
 await sendAttempt; await page.locator('#app-dialog').getByText(/Save pending changes/).waitFor(); await page.locator('#app-dialog-action').click();
 assert.equal(await page.locator('#chat-input').inputValue(),'Keep this draft');
 assert.match(await page.locator('#source').inputValue(),/Save must fail/);
 assert.equal(await app.evaluate(()=>(globalThis as any).__assistantRan===true),false);
 await fs.rmdir(file); await fs.rename(backup,file);
 await page.locator('#source').fill(source + '\n% saved after recovery');
 await page.getByText('Compiled',{exact:true}).waitFor({timeout:30000});
 assert.match(await fs.readFile(file,'utf8'),/saved after recovery/);
 assert.equal(await page.locator('#download').isEnabled(),true);
 await page.locator('#send').click();
 await page.waitForFunction(()=>(document.querySelector('#send') as HTMLButtonElement).disabled===false);
 assert.equal(await app.evaluate(()=>(globalThis as any).__assistantRan===true),true);
});

test('version comparison and project restore recover deleted and changed files', {timeout:60000}, async t => {
 const data=await fs.mkdtemp(path.join(os.tmpdir(),'refract-ui-restore-'));
 const env: NodeJS.ProcessEnv={...process.env,REFRACT_DATA_DIR:data,REFRACT_TEST_HIDDEN:'1'}; delete env.ELECTRON_RUN_AS_NODE;
 const app=await launchDesktop(env);
 t.after(async()=>{await app.close().catch(()=>{});await fs.rm(data,{recursive:true,force:true,maxRetries:5,retryDelay:200});});
 const page=await app.firstWindow(); await page.locator('#new').click(); await page.locator('#source').waitFor({state:'visible'});
 const original=await page.locator('#source').inputValue();
 const id=await page.evaluate(async()=>{const [project]=await store.list(); await store.versionSnapshot(project.id); return project.id;});
 await page.locator('#source').fill(original.replace('Introduction','Changed section'));
 await page.evaluate(async id=>{await store.createFile(id,'added.txt');await store.write(id,'added.txt','new file');},id);
 await page.locator('#tools').click(); await page.locator('#tool-versions').click();
 const oldRow=page.locator('.version-row').last(); await oldRow.waitFor();
 const compare=oldRow.locator('xpath=following-sibling::*[1]'); await compare.locator('summary').first().click();
 await compare.getByText('added: added.txt',{exact:true}).waitFor();
 const changedSummary=compare.getByText('changed: main.tex',{exact:true});
 await changedSummary.click();
 const changedFile=changedSummary.locator('..');
 assert.match(await compare.innerText(),/Changed section/);
 assert.match(await changedFile.locator('.version-column').first().locator('.changed').innerText(),/Introduction/);
 assert.match(await changedFile.locator('.version-column').last().locator('.changed').innerText(),/Changed section/);
 const wrap=changedFile.locator('.comparison-wrap');assert.equal(await wrap.getAttribute('aria-pressed'),'true');await wrap.click();assert.equal(await wrap.getAttribute('aria-pressed'),'false');
 assert.equal(await changedFile.locator('.version-content').first().evaluate(el => getComputedStyle(el).whiteSpace), 'pre');
 await wrap.click();
 assert.equal(await changedFile.locator('.version-content').first().evaluate(el => getComputedStyle(el).whiteSpace), 'pre-wrap');
 await oldRow.getByRole('button',{name:'Restore project'}).click(); await page.locator('#app-dialog-action').click();
 await page.waitForFunction(expected=>(document.querySelector('#source') as HTMLTextAreaElement).value===expected,original);
 await assert.rejects(fs.access(path.join(data,'projects',id,'added.txt')));
 const versions=await page.evaluate(id=>store.versionList(id),id);
 assert.ok(versions.some(version=>version.files.includes('added.txt')));
});

test('project restore warns when staging data cannot be cleaned', {timeout:60000}, async t=>{
 const data=await fs.mkdtemp(path.join(os.tmpdir(),'refract-ui-restore-warning-'));
 const env:NodeJS.ProcessEnv={...process.env,REFRACT_DATA_DIR:data,REFRACT_TEST_HIDDEN:'1'};delete env.ELECTRON_RUN_AS_NODE;
 const app=await launchDesktop(env);
 t.after(async()=>{await app.close().catch(()=>{});await fs.rm(data,{recursive:true,force:true,maxRetries:5,retryDelay:200});});
 const page=await app.firstWindow();await page.locator('#new').click();await page.locator('#source').waitFor({state:'visible'});
 const original=await page.locator('#source').inputValue();
 const id=await page.evaluate(async()=>{const [project]=await store.list();await store.versionSnapshot(project.id);return project.id;});
 await page.locator('#source').fill(original+'\n% changed');await page.evaluate(id=>store.read(id,'main.tex'),id);
 await app.evaluate(({app})=>{const req=(process.getBuiltinModule('module') as any).createRequire(app.getAppPath()+'\\package.json');const {Versions}=req('./build/src/versions.cjs');Versions.prototype.cleanStage=async()=>{};});
 await page.locator('#tools').click();await page.locator('#tool-versions').click();const oldRow=page.locator('.version-row').last();await oldRow.waitFor();
 await oldRow.getByRole('button',{name:'Restore project'}).click(); await page.locator('#app-dialog-action').click();
 await page.waitForFunction(expected=>(document.querySelector('#source') as HTMLTextAreaElement).value===expected,original);
 await page.waitForFunction(()=>!(document.querySelector('#source') as HTMLTextAreaElement).readOnly);
 await page.getByRole('dialog',{name:'Project restored'}).getByText(/temporary restore data remains/i).waitFor(); await page.locator('#app-dialog-action').click();
 assert.equal(await page.locator('#restore-notice').isVisible(),true);
});

test('retained restore data remains visible after reopening the app', {timeout:90000}, async t=>{
 const data:string=await fs.mkdtemp(path.join(os.tmpdir(),'refract-ui-retained-stage-'));
 const parent=await fs.realpath(os.tmpdir());
 const {ProjectStore}=require('../src/project-store.cjs'),store=new ProjectStore(path.join(data,'projects')),project=await store.create('Saved project');
 const stage=path.join(store.projectDir(project.id),'.refract-restore-11111111-1111-1111-1111-111111111111');await fs.mkdir(stage);await fs.writeFile(path.join(stage,'keep.txt'),'preserved');
 const env:NodeJS.ProcessEnv={...process.env,REFRACT_DATA_DIR:data,REFRACT_TEST_HIDDEN:'1'};delete env.ELECTRON_RUN_AS_NODE;let app=null;
 t.after(async()=>{if(app)await app.close().catch(()=>{});const resolved=await fs.realpath(data);if(path.dirname(resolved).toLowerCase()!==parent.toLowerCase() || !path.basename(resolved).startsWith('refract-ui-retained-stage-'))throw new Error('Unsafe cleanup path');await fs.rm(resolved,{recursive:true,force:true,maxRetries:5,retryDelay:200});});
 for(let attempt=0;attempt<2;attempt++){
  app=await launchDesktop(env);const page=await app.firstWindow();await page.locator('#list article').first().click();
  await page.locator('#restore-notice').waitFor({state:'visible'});assert.match(await page.locator('#restore-notice').innerText(),/temporary restore data remains/i);
  await app.close();app=null;
 }
 assert.equal(await fs.readFile(path.join(stage,'keep.txt'),'utf8'),'preserved');
});

test('comment navigation follows repeated text through edits, file rename and restart', {timeout:60000}, async t=>{
 const data=await fs.mkdtemp(path.join(os.tmpdir(),'refract-ui-comment-'));
 const env: NodeJS.ProcessEnv={...process.env,REFRACT_DATA_DIR:data,REFRACT_TEST_HIDDEN:'1'}; delete env.ELECTRON_RUN_AS_NODE;
 let app=await launchDesktop(env);
 t.after(async()=>{if(app)await app.close().catch(()=>{});await fs.rm(data,{recursive:true,force:true,maxRetries:5,retryDelay:200});});
 let page=await app.firstWindow();await page.locator('#new').click();await page.locator('#source').waitFor({state:'visible'});
 const source=(await page.locator('#source').inputValue()).replace('Introduction','Repeat Repeat');
 await page.locator('#source').fill(source);
 const id=await page.evaluate(async()=>{const [project]=await store.list();await store.read(project.id,'main.tex');return project.id;});
 await page.locator('#source').evaluate(el=>{const i=el.value.lastIndexOf('Repeat');el.focus();el.setSelectionRange(i,i+6);el.dispatchEvent(new Event('select'));});
 await page.locator('#tools').click();await page.locator('#tool-comments').click();
 await page.getByRole('textbox',{name:'Add a comment'}).fill('Second occurrence');await page.getByRole('button',{name:'Comment',exact:true}).click();
 await page.getByText('Second occurrence',{exact:true}).waitFor();
 await page.locator('.comment-reply-form summary').click(); await page.getByRole('textbox',{name:'Write a reply'}).fill('A saved reply'); await page.getByRole('button',{name:'Reply',exact:true}).click();
 await page.getByText('A saved reply',{exact:true}).waitFor();
 const changed='% prefix\n'+source; await page.locator('#source').fill(changed);
 await page.locator('.comment-reference').click();
 await page.waitForFunction(expected=>{const el=document.querySelector('#source') as HTMLTextAreaElement;return el.selectionStart===expected && el.selectionEnd===expected+6;},changed.lastIndexOf('Repeat'));
 const compiled=await page.evaluate(async id=>{
   const build=store.compile(id), rename=store.renameFile(id,'main.tex','renamed.tex');
   const [result]=await Promise.all([build,rename]); return result.ok;
 },id);
 assert.equal(compiled,true);
 await app.close();app=null;
 app=await launchDesktop(env);page=await app.firstWindow();await page.locator('#list article').first().click();await page.locator('#source').waitFor({state:'visible'});
 await page.locator('#tools').click();await page.locator('#tool-comments').click();await page.getByText('Second occurrence',{exact:true}).waitFor();
 await page.getByText('A saved reply',{exact:true}).waitFor();
 assert.match(await page.locator('.comment-reference').innerText(),/renamed\.tex/);
 await page.locator('.comment-reference').click();
 await page.waitForFunction(expected=>{const el=document.querySelector('#source') as HTMLTextAreaElement;return el.selectionStart===expected && el.selectionEnd===expected+6;},changed.lastIndexOf('Repeat'));
 const version=await page.evaluate(id=>store.versionSnapshot(id),id);
 const at=changed.lastIndexOf('Repeat'), edited=changed.slice(0,at)+'Edited'+changed.slice(at+6);
 await page.locator('#source').fill(edited);
 await page.evaluate(id=>store.read(id,'renamed.tex'),id);
 assert.equal(await page.evaluate(async id=>(await store.commentList(id))[0].quote,id),'Edited');
 await page.evaluate(({id,version})=>store.versionRestoreFile(id,version,'renamed.tex'),{id,version:version.id});
 assert.equal(await page.evaluate(async id=>(await store.commentList(id))[0].quote,id),'Repeat');
 // Repeat through the project-wide restore path.
 await page.locator('#source').fill(changed); await page.evaluate(id=>store.read(id,'renamed.tex'),id);
 await page.locator('#source').fill(edited);
 await page.evaluate(id=>store.read(id,'renamed.tex'),id);
 assert.equal(await page.evaluate(async id=>(await store.commentList(id))[0].quote,id),'Edited');
 const restored=await page.evaluate(({id,version})=>store.versionRestoreProject(id,version),{id,version:version.id});
 assert.equal(restored.project.id,id);assert.equal(restored.cleanupPending,0);
 assert.equal(await page.evaluate(async id=>(await store.commentList(id))[0].quote,id),'Repeat');
});

function twoPagePdf() {
 const objects=[
  '<< /Type /Catalog /Pages 2 0 R >>',
  '<< /Type /Pages /Kids [3 0 R 5 0 R] /Count 2 >>',
  '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Resources << /Font << /F1 7 0 R >> >> /Contents 4 0 R >>',
  '<< /Length 32 >>\nstream\nBT /F1 12 Tf 20 100 Td (One) Tj ET\nendstream',
  '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Resources << /Font << /F1 7 0 R >> >> /Contents 6 0 R >>',
  '<< /Length 32 >>\nstream\nBT /F1 12 Tf 20 100 Td (Two) Tj ET\nendstream',
  '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>'
 ];
 let output='%PDF-1.4\n'; const offsets=[0];
 for(let i=0;i<objects.length;i++){offsets.push(Buffer.byteLength(output));output+=`${i+1} 0 obj\n${objects[i]}\nendobj\n`;}
 const xref=Buffer.byteLength(output);output+=`xref\n0 ${offsets.length}\n0000000000 65535 f \n`;
 for(const offset of offsets.slice(1))output+=`${String(offset).padStart(10,'0')} 00000 n \n`;
 output+=`trailer\n<< /Size ${offsets.length} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF\n`;
 return Buffer.from(output);
}

test('project PDF and SVG previews stay separate from binary and text editing', {timeout:60000}, async t=>{
 const data=await fs.mkdtemp(path.join(os.tmpdir(),'refract-ui-preview-'));
 const env: NodeJS.ProcessEnv={...process.env,REFRACT_DATA_DIR:data,REFRACT_TEST_HIDDEN:'1'};delete env.ELECTRON_RUN_AS_NODE;
 const app=await launchDesktop(env);t.after(async()=>{await app.close().catch(()=>{});await fs.rm(data,{recursive:true,force:true,maxRetries:5,retryDelay:200});});
 const page=await app.firstWindow(),requests=[];page.on('request',request=>requests.push(request.url()));
 await page.locator('#new').click();await page.locator('#source').waitFor({state:'visible'});
 const [id]=(await fs.readdir(path.join(data,'projects'))).filter((name:string)=>/^[0-9a-f-]{36}$/.test(name)),project=path.join(data,'projects',id);
 await fs.writeFile(path.join(project,'paper.pdf'),twoPagePdf());
 await fs.writeFile(path.join(project,'pixel.bmp'),Buffer.from('424d3a0000000000000036000000280000000100000001000000010018000000000004000000000000000000000000000000000000000000ff00','hex'));
 await fs.writeFile(path.join(project,'broken.pdf'),'not a PDF');
 await fs.writeFile(path.join(project,'broken.png'),'not an image');
 await fs.writeFile(path.join(project,'diagram.svg'),'<svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" onload="window.top.svgExecuted=true"><script>window.top.svgExecuted=true</script><image href="https://example.invalid/tracker.png" width="5" height="5"/><rect width="20" height="20" fill="red"/></svg>');
 const binary=Buffer.from([0,255,1,2]);await fs.writeFile(path.join(project,'binary.tex'),binary);
 await fs.writeFile(path.join(project,'note.txt'),'Safe text');
 await page.locator('#home').click();await page.locator('#list article').first().click();await page.locator('#source').waitFor({state:'visible'});
 await page.locator('#file-list button[title="diagram.svg"]').click();await page.locator('#project-image').waitFor({state:'visible'});
 assert.equal(await page.getByRole('button',{name:'Show in PDF'}).isDisabled(),true);
 assert.equal(await page.getByRole('button',{name:'Commands',exact:true}).isDisabled(),true);
 await page.waitForFunction(()=>{const img=document.querySelector('#project-image') as HTMLImageElement;return img.complete && img.naturalWidth===20;});
 assert.equal(await page.evaluate(()=>('svgExecuted' in window)),false);
 assert.equal(requests.some(url=>url.includes('example.invalid')),false);
 await page.locator('#file-list button[title="pixel.bmp"]').click();
 await page.waitForFunction(()=>{const img=document.querySelector('#project-image') as HTMLImageElement;return img.complete && img.naturalWidth===1;});
 await page.locator('#file-list button[title="broken.png"]').click();await page.getByText('Could not preview this image.').waitFor({state:'visible'});assert.equal(await page.locator('#project-image').isHidden(),true);
 await page.locator('#file-list button[title="diagram.svg"]').click();await page.locator('#project-image').waitFor({state:'visible'});assert.equal(await page.locator('#file-preview').isHidden(),true);
 await page.locator('#file-list button[title="paper.pdf"]').click();
 await page.locator('#file-preview canvas').waitFor({state:'visible'});
 assert.match(await page.locator('#file-preview').innerText(),/Page 1 of 2/);
 assert.equal(await page.getByRole('button',{name:'Previous PDF page'}).isDisabled(),true);
 await page.getByRole('button',{name:'Next PDF page'}).click();
 await page.getByLabel('Page 2 of paper.pdf').waitFor({state:'visible'});
 assert.equal(await page.getByRole('button',{name:'Next PDF page'}).isDisabled(),true);
 await page.locator('#file-list button[title="broken.pdf"]').click();
 await page.getByText(/Could not preview PDF/).waitFor({state:'visible'});
 assert.equal(await page.locator('#source').isHidden(),true);
 await page.locator('#file-list button[title="binary.tex"]').click();
 await page.getByText(/cannot be previewed as text/).waitFor({state:'visible'});
 assert.match(await page.locator('#file-preview').innerText(),/cannot be previewed as text/);
 assert.equal(await page.locator('#source').isHidden(),true);assert.deepEqual(await fs.readFile(path.join(project,'binary.tex')),binary);
 await page.locator('#file-list button[title="note.txt"]').click();
 await page.locator('#source').waitFor({state:'visible'});assert.equal(await page.locator('#source').inputValue(),'Safe text');
 assert.equal(await page.getByRole('button',{name:'Commands',exact:true}).isDisabled(),false);
 assert.equal(await page.locator('#file-preview').isHidden(),true);
});

test('deleted files can be restored from the project UI', {timeout:60000}, async t=>{
 const data=await fs.mkdtemp(path.join(os.tmpdir(),'refract-ui-trash-'));
 const env: NodeJS.ProcessEnv={...process.env,REFRACT_DATA_DIR:data,REFRACT_TEST_HIDDEN:'1'};delete env.ELECTRON_RUN_AS_NODE;
 const app=await launchDesktop(env);t.after(async()=>{await app.close().catch(()=>{});await fs.rm(data,{recursive:true,force:true,maxRetries:5,retryDelay:200});});
 const page=await app.firstWindow();await page.locator('#new').click();await page.locator('#source').waitFor({state:'visible'});
 await page.evaluate(async()=>{const [p]=await store.list();await store.createFile(p.id,'recover.txt');await store.write(p.id,'recover.txt','saved content');});
 await page.locator('#home').click();await page.locator('#list article').first().click();await page.locator('#file-list button[title="recover.txt"]').waitFor();
 await page.locator('#file-list button[title="recover.txt"]').click({button:'right'});await page.locator('#file-delete').click(); await page.locator('#app-dialog-action').click();
 await page.locator('#file-list button[title="recover.txt"]').waitFor({state:'detached'});
 await page.locator('#deleted-files').click();await page.getByText('recover.txt',{exact:true}).waitFor();
 await page.locator('#trash-list').getByRole('button',{name:'Restore'}).click();await page.getByText('No deleted files',{exact:true}).waitFor();
 await page.locator('#trash-close').click();await page.locator('#source').waitFor({state:'visible'});assert.equal(await page.locator('#source').inputValue(),'saved content');
});

test('engine choice compiles, persists after restart and is copied', {timeout:90000}, async t=>{
 const data=await fs.mkdtemp(path.join(os.tmpdir(),'refract-ui-engine-'));
 const env: NodeJS.ProcessEnv={...process.env,REFRACT_DATA_DIR:data,REFRACT_TEST_HIDDEN:'1'};delete env.ELECTRON_RUN_AS_NODE;
 let app=await launchDesktop(env);t.after(async()=>{if(app)await app.close().catch(()=>{});await fs.rm(data,{recursive:true,force:true,maxRetries:5,retryDelay:200});});
 let page=await app.firstWindow();await page.locator('#new').click();await page.locator('#source').waitFor({state:'visible'});
 await page.locator('#tools').click();await page.locator('#tool-info').click();
 const selector=page.getByRole('combobox',{name:'LaTeX engine'});assert.equal(await selector.inputValue(),'pdflatex');
 await selector.selectOption('xelatex');await page.waitForFunction(()=>{const e=document.querySelector('select[aria-label="LaTeX engine"]') as HTMLSelectElement;return e && !e.disabled && e.value==='xelatex';});
 await page.locator('#tools').click();await page.getByText('Compiled',{exact:true}).waitFor({timeout:45000});
 await page.locator('#tools').click();await page.locator('#tool-logs').click();await page.locator('.raw-output summary').click();assert.match(await page.locator('#tool-body').innerText(),/XeTeX/);
 await app.close();app=null;app=await launchDesktop(env);page=await app.firstWindow();
 await page.locator('#list article').first().click();await page.locator('#source').waitFor({state:'visible'});
 await page.locator('#tools').click();await page.locator('#tool-info').click();assert.equal(await page.getByRole('combobox',{name:'LaTeX engine'}).inputValue(),'xelatex');
 await page.getByRole('combobox',{name:'LaTeX engine'}).selectOption('lualatex');
 await page.waitForFunction(()=>{const e=document.querySelector('select[aria-label="LaTeX engine"]') as HTMLSelectElement;return e && !e.disabled && e.value==='lualatex';});
 await page.locator('#tools').click();await page.getByText('Compiled',{exact:true}).waitFor({timeout:45000});
 await page.locator('#tools').click();await page.locator('#tool-logs').click();await page.locator('.raw-output summary').click();assert.match(await page.locator('#tool-body').innerText(),/LuaHBTeX|LuaTeX/);
 const copy=await page.evaluate(async()=>{const [p]=await store.list();return store.duplicate(p.id);});assert.equal(copy.engine,'lualatex');
});

test('nested file creation and uploads use the chosen folder', {timeout:60000}, async t=>{
 const data=await fs.mkdtemp(path.join(os.tmpdir(),'refract-ui-destination-'));
 const env: NodeJS.ProcessEnv={...process.env,REFRACT_DATA_DIR:data,REFRACT_TEST_HIDDEN:'1'};delete env.ELECTRON_RUN_AS_NODE;
 const file=path.join(data,'photo.bin'),folder=path.join(data,'resources');await fs.writeFile(file,Buffer.from([0,42,255]));await fs.mkdir(folder);await fs.writeFile(path.join(folder,'data.txt'),'nested upload');
 const app=await launchDesktop(env);t.after(async()=>{await app.close().catch(()=>{});await fs.rm(data,{recursive:true,force:true,maxRetries:5,retryDelay:200});});
 await app.evaluate(({dialog},paths)=>{dialog.showOpenDialog=async (_window,options)=>({canceled:false,filePaths:[options.properties.includes('openDirectory')?paths.folder:paths.file]});},{file,folder});
 const page=await app.firstWindow();await page.locator('#new').click();await page.locator('#source').waitFor({state:'visible'});
 await page.locator('#add-file').click();await page.locator('#file-name').fill('chapters');await page.locator('.file-alternatives summary').click();await page.locator('#file-dialog button[value="folder"]').click();
 await page.locator('#file-list summary[title="chapters"]').waitFor();
 await page.locator('#file-list summary[title="chapters"]').click({button:'right'});
 await page.getByRole('button',{name:'Add items'}).click();
 assert.equal(await page.getByRole('combobox',{name:'Destination folder'}).inputValue(),'chapters');
 await page.locator('#file-name').fill('child.tex');await page.locator('#file-dialog button[value="create"]').click();
 await page.locator('#file-list button[title="chapters/child.tex"]').waitFor();
 await page.locator('#add-file').click();await page.getByRole('combobox',{name:'Destination folder'}).selectOption('chapters');
 await page.locator('.file-alternatives summary').click();await page.locator('#file-dialog button[value="upload"]').click();await page.locator('#file-list button[title="chapters/photo.bin"]').waitFor();
 await page.locator('#add-file').click();await page.getByRole('combobox',{name:'Destination folder'}).selectOption('chapters');
 await page.locator('.file-alternatives summary').click();await page.locator('#file-dialog button[value="upload-folder"]').click();await page.locator('#file-list button[title="chapters/resources/data.txt"]').waitFor();
 const [id]=(await fs.readdir(path.join(data,'projects'))).filter((name:string)=>/^[0-9a-f-]{36}$/.test(name));assert.deepEqual(await fs.readFile(path.join(data,'projects',id,'chapters','photo.bin')),Buffer.from([0,42,255]));
});

test('moving a folder updates root, tabs and comment paths', {timeout:60000}, async t=>{
 const data=await fs.mkdtemp(path.join(os.tmpdir(),'refract-ui-folder-move-'));
 const env: NodeJS.ProcessEnv={...process.env,REFRACT_DATA_DIR:data,REFRACT_TEST_HIDDEN:'1'};delete env.ELECTRON_RUN_AS_NODE;
 let app=await launchDesktop(env);t.after(async()=>{if(app)await app.close().catch(()=>{});await fs.rm(data,{recursive:true,force:true,maxRetries:5,retryDelay:200});});
 let page=await app.firstWindow();await page.locator('#new').click();await page.locator('#source').waitFor({state:'visible'});
 const id=await page.evaluate(async()=>{const [p]=await store.list();await store.createFolder(p.id,'chapters');await store.createFolder(p.id,'drafts');await store.renameFile(p.id,'main.tex','chapters/main.tex');const s=await store.read(p.id,'chapters/main.tex'),start=s.indexOf('Introduction');await store.commentAdd(p.id,'chapters/main.tex','Introduction','Review',{start,end:start+12});return p.id;});
 await page.locator('#home').click();await page.locator('#list article').first().click();await page.locator('#source').waitFor({state:'visible'});
 await page.locator('#file-list summary[title="chapters"]').click({button:'right'});await page.getByRole('button',{name:'Move or rename folder'}).click();
 await page.getByRole('textbox',{name:'New folder path'}).fill('drafts/chapters');await page.locator('#folder-move-dialog button[value="move"]').click();
 await page.locator('#file-list button[title="drafts/chapters/main.tex"]').waitFor();
 await page.waitForFunction(()=>{const el=document.querySelector('#source') as HTMLTextAreaElement;return el.value.includes('Introduction');});
 const result=await page.evaluate(async id=>{const comments=await store.commentList(id);return {project:(await store.list()).find(p=>p.id===id),comments,location:await store.commentLocate(id,comments[0].id)};},id);
 assert.equal(result.project.rootFile,'drafts/chapters/main.tex');assert.equal(result.comments[0].file,'drafts/chapters/main.tex');
 assert.ok(result.location && result.location.end-result.location.start===12);
 await page.locator('#file-tabs button[title="drafts/chapters/main.tex"]').waitFor();
 await app.close();app=null;app=await launchDesktop(env);page=await app.firstWindow();await page.locator('#list article').first().click();
 await page.locator('#source').waitFor({state:'visible'});assert.match(await page.locator('#source').inputValue(),/Introduction/);
});

test('deleted folders can be restored from the project UI', {timeout:60000}, async t=>{
 const data=await fs.mkdtemp(path.join(os.tmpdir(),'refract-ui-trash-folder-'));
 const env: NodeJS.ProcessEnv={...process.env,REFRACT_DATA_DIR:data,REFRACT_TEST_HIDDEN:'1'};delete env.ELECTRON_RUN_AS_NODE;
 let app=await launchDesktop(env);t.after(async()=>{if(app)await app.close().catch(()=>{});await fs.rm(data,{recursive:true,force:true,maxRetries:5,retryDelay:200});});
 let page=await app.firstWindow();await page.locator('#new').click();await page.locator('#source').waitFor({state:'visible'});
 const id=await page.evaluate(async()=>{const [p]=await store.list();await store.createFolder(p.id,'chapters/empty');await store.renameFile(p.id,'main.tex','chapters/main.tex');const s=await store.read(p.id,'chapters/main.tex'),start=s.indexOf('Introduction');await store.commentAdd(p.id,'chapters/main.tex','Introduction','Review',{start,end:start+12});return p.id;});
 await page.locator('#home').click();await page.locator('#list article').first().click();await page.locator('#source').waitFor({state:'visible'});
 await page.locator('#file-list summary[title="chapters"]').click({button:'right'});await page.getByRole('button',{name:'Delete folder'}).click(); await page.locator('#app-dialog-action').click();
 await page.locator('#file-list summary[title="chapters"]').waitFor({state:'detached'});await page.locator('#deleted-files').click();await page.getByText('chapters/',{exact:true}).waitFor();
 await page.locator('#trash-list').getByRole('button',{name:'Restore'}).click();await page.getByText('No deleted files',{exact:true}).waitFor();await page.locator('#trash-close').click();
 await page.locator('#file-list button[title="chapters/main.tex"]').waitFor();await page.locator('#source').waitFor({state:'visible'});
 assert.match(await page.locator('#source').inputValue(),/Introduction/);
 const result=await page.evaluate(async id=>{const comments=await store.commentList(id);return {comment:comments[0],position:await store.commentLocate(id,comments[0].id)};},id);
 assert.equal(result.comment.file,'chapters/main.tex');assert.equal(result.position.end-result.position.start,12);
 assert.equal((await fs.stat(path.join(data,'projects',id,'chapters','empty'))).isDirectory(),true);
 await app.close();app=null;app=await launchDesktop(env);page=await app.firstWindow();await page.locator('#list article').first().click();await page.locator('#source').waitFor({state:'visible'});
 assert.match(await page.locator('#source').inputValue(),/Introduction/);
});

test('project list sorts and restores deleted projects with their comments', {timeout:60000}, async t=>{
 const data=await fs.mkdtemp(path.join(os.tmpdir(),'refract-ui-project-trash-'));
 const env: NodeJS.ProcessEnv={...process.env,REFRACT_DATA_DIR:data,REFRACT_TEST_HIDDEN:'1'};delete env.ELECTRON_RUN_AS_NODE;
 let app=await launchDesktop(env);t.after(async()=>{if(app)await app.close().catch(()=>{});await fs.rm(data,{recursive:true,force:true,maxRetries:5,retryDelay:200});});
 let page=await app.firstWindow();await page.locator('#new').waitFor();
 const ids=await page.evaluate(async()=>{const z=await store.create('Zeta'),a=await store.create('Alpha');await store.commentAdd(z.id,'main.tex','Introduction','Keep this');await store.versionSnapshot(z.id);return {z:z.id,a:a.id};});
 const id=ids.z;
 for(const [projectId,created] of [[ids.z,'2020-01-01T00:00:00.000Z'],[ids.a,'2021-01-01T00:00:00.000Z']]) {const file=path.join(data,'projects',projectId,'.refract.json'),meta=JSON.parse(await fs.readFile(file,'utf8'));meta.created=created;await fs.writeFile(file,JSON.stringify(meta));}
 await fs.utimes(path.join(data,'projects',ids.z,'main.tex'),new Date('2022-01-01'),new Date('2022-01-01'));
 await fs.utimes(path.join(data,'projects',ids.a,'main.tex'),new Date('2021-01-01'),new Date('2021-01-01'));
 await page.reload();await page.getByRole('button',{name:'Open Zeta'}).waitFor();
 await page.getByRole('combobox',{name:'Sort projects'}).selectOption('edited');assert.match((await page.locator('#list article span:first-child').allTextContents())[0],/Zeta/);
 await page.reload();await page.getByRole('button',{name:'Open Zeta'}).waitFor();assert.equal(await page.getByRole('combobox',{name:'Sort projects'}).inputValue(),'edited');
 await page.getByRole('combobox',{name:'Sort projects'}).selectOption('oldest');assert.match((await page.locator('#list article span:first-child').allTextContents())[0],/Zeta/);
 await page.getByRole('combobox',{name:'Sort projects'}).selectOption('newest');assert.match((await page.locator('#list article span:first-child').allTextContents())[0],/Alpha/);
 await page.getByRole('combobox',{name:'Sort projects'}).selectOption('name');
 const names=await page.locator('#list article span:first-child').allTextContents();assert.match(names[0],/Alpha/);assert.match(names[1],/Zeta/);
 await page.getByLabel('Actions for Zeta').click();await page.getByRole('button',{name:'Delete Zeta'}).click(); await page.locator('#app-dialog-action').click();
 await page.getByRole('button',{name:'Open Zeta'}).waitFor({state:'detached'});
 await app.close();app=null;app=await launchDesktop(env);page=await app.firstWindow();await page.getByRole('button',{name:'Open Alpha'}).waitFor();
 await page.getByRole('button',{name:'Deleted projects'}).click();await page.getByText('Zeta',{exact:true}).waitFor();
 await page.getByRole('button',{name:'Restore Zeta'}).click();await page.getByText('No deleted projects',{exact:true}).waitFor();await page.getByRole('button',{name:'Close',exact:true}).click();
 await page.getByRole('button',{name:'Open Zeta'}).waitFor();
 assert.equal((await page.evaluate(id=>store.commentList(id),id))[0].text,'Keep this');assert.ok((await page.evaluate(id=>store.versionList(id),id)).length>=1);
 await app.close();app=null;app=await launchDesktop(env);page=await app.firstWindow();await page.getByRole('button',{name:'Open Zeta'}).waitFor();
 assert.equal(await page.getByRole('combobox',{name:'Sort projects'}).inputValue(),'name');
});

test('project rename, duplication and engine setting persist independently', {timeout:60000}, async t=>{
 const data=await fs.mkdtemp(path.join(os.tmpdir(),'refract-ui-project-actions-'));
 const env: NodeJS.ProcessEnv={...process.env,REFRACT_DATA_DIR:data,REFRACT_TEST_HIDDEN:'1'};delete env.ELECTRON_RUN_AS_NODE;
 let app=await launchDesktop(env);t.after(async()=>{if(app)await app.close().catch(()=>{});await fs.rm(data,{recursive:true,force:true,maxRetries:5,retryDelay:200});});
 let page=await app.firstWindow();await page.locator('#new').click();await page.locator('#source').waitFor({state:'visible'});
 await page.locator('#project-name').click();await page.locator('#rename-name').fill('Research');await page.locator('#rename-dialog button[value="rename"]').click();
 await page.locator('#project-name').getByText('Research',{exact:true}).waitFor();
 await page.locator('#tools').click();await page.locator('#tool-info').click();await page.getByLabel('LaTeX engine').selectOption('xelatex');
 await page.waitForFunction(async()=>{const [p]=await store.list();return p.engine==='xelatex';});
 await page.waitForFunction(()=>{const input=document.querySelector('select[aria-label="LaTeX engine"]') as HTMLSelectElement;return input?.value==='xelatex' && !input.disabled;});
 await page.locator('#tools').click();
 await page.getByText('Compiled',{exact:true}).waitFor({timeout:45000});
 await page.locator('#duplicate').click();await page.locator('#app-dialog-action').click();await page.locator('#project-name').getByText('Research (copy)',{exact:true}).waitFor();
 await page.locator('#home').click();await page.getByRole('button',{name:'Open Research (copy)'}).waitFor();
 await app.close();app=null;app=await launchDesktop(env);page=await app.firstWindow();
 await page.getByRole('button',{name:'Open Research',exact:true}).waitFor();await page.getByRole('button',{name:'Open Research (copy)'}).click();await page.locator('#source').waitFor({state:'visible'});
 await page.locator('#tools').click();await page.locator('#tool-info').click();assert.equal(await page.getByLabel('LaTeX engine').inputValue(),'xelatex');
 await page.getByLabel('LaTeX engine').selectOption('lualatex');await page.waitForFunction(async()=>{const p=(await store.list()).find(p=>p.name==='Research (copy)');return p?.engine==='lualatex';});
 const projects=await page.evaluate(()=>store.list());assert.equal(projects.find(p=>p.name==='Research').engine,'xelatex');
});

test('project deletion waits for writes and does not leave a delayed snapshot', {timeout:60000}, async t=>{
 const data=await fs.mkdtemp(path.join(os.tmpdir(),'refract-ui-delete-race-'));
 const env: NodeJS.ProcessEnv={...process.env,REFRACT_DATA_DIR:data,REFRACT_TEST_HIDDEN:'1'};delete env.ELECTRON_RUN_AS_NODE;
 const app=await launchDesktop(env);t.after(async()=>{await app.close().catch(()=>{});await fs.rm(data,{recursive:true,force:true,maxRetries:5,retryDelay:200});});
 const page=await app.firstWindow();await page.locator('#new').waitFor();
 const result=await page.evaluate(async()=>{
   const p=await store.create('Race');await store.setEngine(p.id,'xelatex');const old=await store.read(p.id,'main.tex');
   const building=store.compile(p.id),saving=store.write(p.id,'main.tex',old+'\n% final saved edit'),deleting=store.trashProject(p.id);
   const [compiled]=await Promise.all([building,saving,deleting]);
   await new Promise(resolve=>setTimeout(resolve,3500));
   const deleted=await store.deletedProjects();let rejected=false,chatRejected=false,sendRejected=false;try{await store.write(p.id,'main.tex','late');}catch{rejected=true;}
   try{await store.chatCreate(p.id);}catch{chatRejected=true;}
   try{await store.chatSend(p.id,'This must not run','default',[]);}catch{sendRejected=true;}
   await store.restoreProject(deleted[0].id);
   return {compiled:compiled.ok,rejected,chatRejected,sendRejected,content:await store.read(p.id,'main.tex')};
 });
 assert.equal(result.compiled,true);assert.equal(result.rejected,true);assert.equal(result.chatRejected,true);assert.equal(result.sendRejected,true);assert.match(result.content,/final saved edit/);
});

test('closing during a queued save preserves the final version snapshot', {timeout:60000}, async t=>{
 const data=await fs.mkdtemp(path.join(os.tmpdir(),'refract-ui-close-snapshot-'));
 const env: NodeJS.ProcessEnv={...process.env,REFRACT_DATA_DIR:data,REFRACT_TEST_HIDDEN:'1'};delete env.ELECTRON_RUN_AS_NODE;
 let app=await launchDesktop(env);t.after(async()=>{if(app)await app.close().catch(()=>{});await fs.rm(data,{recursive:true,force:true,maxRetries:5,retryDelay:200});});
 const page=await app.firstWindow();await page.locator('#new').click();await page.locator('#source').waitFor({state:'visible'});
 const id=await page.evaluate(async()=>{const [p]=await store.list();await store.setEngine(p.id,'xelatex');void store.compile(p.id).catch(()=>{});return p.id;});
 const source=await page.locator('#source').inputValue();await page.locator('#source').fill(source+'\n% final close snapshot');
 await app.close();app=null;
 const folder=path.join(data,'projects',id,'.refract-versions'),entries=(await fs.readdir(folder)).filter(name=>name.endsWith('.json'));
 const versions=await Promise.all(entries.map(async name=>JSON.parse(await fs.readFile(path.join(folder,name),'utf8'))));versions.sort((a,b)=>b.sequence-a.sequence);
 const Zip=require('adm-zip'),zip=new Zip(path.join(folder,versions[0].id+'.zip'));
 assert.match(zip.getEntry('main.tex').getData().toString('utf8'),/final close snapshot/);
});

test('closing cancels a stalled real compile and preserves a queued edit', {timeout:60000}, async t=>{
 const data:string=await fs.mkdtemp(path.join(os.tmpdir(),'refract-ui-stalled-compile-'));
 const env:NodeJS.ProcessEnv={...process.env,REFRACT_DATA_DIR:data,REFRACT_TEST_HIDDEN:'1'};delete env.ELECTRON_RUN_AS_NODE;
 let app:any=await launchDesktop(env);
 t.after(async()=>{if(app)await app.close().catch(()=>{});await fs.rm(data,{recursive:true,force:true,maxRetries:5,retryDelay:200});});
 const page=await app.firstWindow();await page.locator('#new').click();await page.locator('#source').waitFor({state:'visible'});
 const id=await page.evaluate(async()=>{const [p]=await store.list();return p.id;});
 const stalled='\\documentclass{article}\n\\begin{document}\n\\newwrite\\marker\\immediate\\openout\\marker=refract-started.txt\\immediate\\write\\marker{started}\\immediate\\closeout\\marker\n\\loop\\iftrue\\repeat\n\\end{document}';
 await page.locator('#source').fill(stalled);
 const projectDir=path.join(data,'projects',id),deadline=Date.now()+20000;
 let started=false;while(Date.now()<deadline && !started){for(const file of [path.join(projectDir,'refract-started.txt'),path.join(projectDir,'.refract-build','refract-started.txt')]){try{await fs.access(file);started=true;break;}catch{}}if(!started)await new Promise(resolve=>setTimeout(resolve,100));}
 assert.equal(started,true,'LaTeX did not reach the deliberate infinite loop');
 const final=stalled+'\n% edit queued during compile';await page.locator('#source').fill(final);
 assert.equal(await fs.readFile(path.join(projectDir,'main.tex'),'utf8'),stalled);
 await app.close();app=null;
 assert.equal(await fs.readFile(path.join(projectDir,'main.tex'),'utf8'),final);
 const entries=await fs.readdir(path.join(projectDir,'.refract-versions'));
 const versions=await Promise.all(entries.filter(name=>name.endsWith('.json')).map(async name=>JSON.parse(await fs.readFile(path.join(projectDir,'.refract-versions',name),'utf8'))));
 versions.sort((a,b)=>b.sequence-a.sequence);const Zip=require('adm-zip');
 assert.equal(new Zip(path.join(projectDir,'.refract-versions',versions[0].id+'.zip')).getEntry('main.tex').getData().toString('utf8'),final);
});

test('closing waits for a stalled compiler even without a pending save', {timeout:60000}, async t=>{
 if(process.platform!=='win32')return t.skip('Windows process inspection');
 const data:string=await fs.mkdtemp(path.join(os.tmpdir(),'refract-ui-close-compiler-'));
 const env:NodeJS.ProcessEnv={...process.env,REFRACT_DATA_DIR:data,REFRACT_TEST_HIDDEN:'1'};delete env.ELECTRON_RUN_AS_NODE;
 let app:any=await launchDesktop(env);
 t.after(async()=>{if(app)await app.close().catch(()=>{});const resolved=await fs.realpath(data),parent=await fs.realpath(os.tmpdir());if(path.dirname(resolved).toLowerCase()!==parent.toLowerCase() || !path.basename(resolved).startsWith('refract-ui-close-compiler-'))throw new Error('Unsafe cleanup path');await fs.rm(resolved,{recursive:true,force:true,maxRetries:5,retryDelay:200});});
 const page=await app.firstWindow();await page.locator('#new').click();await page.locator('#source').waitFor({state:'visible'});
 const id=await page.evaluate(async()=>{const [p]=await store.list();return p.id;});
 const stalled='\\documentclass{article}\n\\begin{document}\n\\newwrite\\marker\\immediate\\openout\\marker=refract-started.txt\\immediate\\write\\marker{started}\\immediate\\closeout\\marker\n\\loop\\iftrue\\repeat\n\\end{document}';
 await page.locator('#source').fill(stalled);const projectDir=path.join(data,'projects',id),deadline=Date.now()+20000;
 let started=false;while(Date.now()<deadline && !started){for(const file of [path.join(projectDir,'refract-started.txt'),path.join(projectDir,'.refract-build','refract-started.txt')]){try{await fs.access(file);started=true;break;}catch{}}if(!started)await new Promise(resolve=>setTimeout(resolve,100));}
 assert.equal(started,true,'LaTeX did not reach the deliberate infinite loop');
 const {spawnSync}=require('node:child_process');
 const query=spawnSync('powershell.exe',['-NoProfile','-NonInteractive','-Command',"Get-CimInstance Win32_Process | Where-Object { $_.Name -match '^(pdftex|pdflatex)\\.exe$' -and $_.CommandLine.Contains($env:REFRACT_TEST_PROJECT) } | ForEach-Object { $_.ProcessId }"],{windowsHide:true,encoding:'utf8',env:{...process.env,REFRACT_TEST_PROJECT:projectDir}});
 assert.equal(query.status,0,query.stderr);const pids=query.stdout.trim().split(/\s+/).filter(Boolean).map(Number);assert.ok(pids.length,'Expected the running compiler process');
 await app.close();app=null;
 for(const pid of pids) {let alive=true;try{process.kill(pid,0);}catch(error){if(error.code==='ESRCH')alive=false;else throw error;}assert.equal(alive,false,`Compiler ${pid} remained after app exit`);}
});

test('closing the window preserves a pending edit and its snapshot', {timeout:60000}, async t=>{
 const data=await fs.mkdtemp(path.join(os.tmpdir(),'refract-ui-window-close-'));
 const env: NodeJS.ProcessEnv={...process.env,REFRACT_DATA_DIR:data,REFRACT_TEST_HIDDEN:'1'};delete env.ELECTRON_RUN_AS_NODE;
 let app=await launchDesktop(env);t.after(async()=>{if(app)await app.close().catch(()=>{});await fs.rm(data,{recursive:true,force:true,maxRetries:5,retryDelay:200});});
 const page=await app.firstWindow();await page.locator('#new').click();await page.locator('#source').waitFor({state:'visible'});
 const id=await page.evaluate(async()=>{const [p]=await store.list();await store.setEngine(p.id,'xelatex');void store.compile(p.id).catch(()=>{});return p.id;});
 const source=await page.locator('#source').inputValue();await page.locator('#source').fill(source+'\n% window close edit');
 const closed=app.waitForEvent('close');await app.evaluate(({BrowserWindow})=>BrowserWindow.getAllWindows()[0].close());await closed;app=null;
 const dir=path.join(data,'projects',id),folder=path.join(dir,'.refract-versions');assert.match(await fs.readFile(path.join(dir,'main.tex'),'utf8'),/window close edit/);
 const entries=(await fs.readdir(folder)).filter(name=>name.endsWith('.json'));
 const versions=await Promise.all(entries.map(async name=>JSON.parse(await fs.readFile(path.join(folder,name),'utf8'))));versions.sort((a,b)=>b.sequence-a.sequence);
 const Zip=require('adm-zip');assert.match(new Zip(path.join(folder,versions[0].id+'.zip')).getEntry('main.tex').getData().toString('utf8'),/window close edit/);
});

test('assistant operations protect a project and snapshot changes after navigation', {timeout:60000}, async t=>{
 const data=await fs.mkdtemp(path.join(os.tmpdir(),'refract-ui-assistant-'));
 const env: NodeJS.ProcessEnv={...process.env,REFRACT_DATA_DIR:data,REFRACT_TEST_HIDDEN:'1'};delete env.ELECTRON_RUN_AS_NODE;
 const app=await launchDesktop(env,true);t.after(async()=>{await app.evaluate(()=>{const state=globalThis as any;if(state.__fakeRelease)state.__fakeRelease();else state.__fakeReleaseBeforeStart=true;}).catch(()=>{});await app.close().catch(()=>{});const resolved=await fs.realpath(data),parent=await fs.realpath(os.tmpdir());if(path.dirname(resolved).toLowerCase()!==parent.toLowerCase() || !path.basename(resolved).startsWith('refract-ui-assistant-'))throw new Error('Unsafe cleanup path');await fs.rm(resolved,{recursive:true,force:true,maxRetries:5,retryDelay:200});});
 await app.evaluate(({app})=>{
  const req=(process.getBuiltinModule('module') as any).createRequire(app.getAppPath()+'\\package.json');
  const {ChatService}=req('./build/src/chat.cjs'), io=req('node:fs/promises');
  (globalThis as any).__fakeStarted=new Promise(resolve=>(globalThis as any).__fakeNotify=resolve);
  ChatService.prototype.send=async function(id,prompt,_chatId,_attachments,_signal,onProgress){
   onProgress('Running a command…');(globalThis as any).__fakeNotify();await new Promise<void>(resolve=>{const state=globalThis as any;state.__fakeRelease=resolve;if(state.__fakeReleaseBeforeStart)resolve();});
   const file=this.store.filePath(id,'main.tex'),content=await io.readFile(file,'utf8');await io.writeFile(file,content+'\n% assistant edit');
   return {sessionId:null,messages:[{role:'user',text:prompt},{role:'assistant',text:'Done'}]};
  };
 });
 const page=await app.firstWindow();await page.locator('#new').click();await page.locator('#source').waitFor({state:'visible'});
 const id=await page.evaluate(async()=>{const [p]=await store.list();return p.id;});
 const original=await page.locator('#source').inputValue();await page.locator('#source').fill(original+'\n% saved before assistant');
 await page.locator('#chat-input').fill('Make a change');await page.locator('#send').click();
 await app.evaluate(async()=>await (globalThis as any).__fakeStarted);
 await page.getByRole('status').getByText('Running a command…').waitFor();
 assert.equal(await page.getByRole('button',{name:'Commands',exact:true}).isDisabled(),true);
 const blocked=await page.evaluate(async id=>{
  const results=[];for(const action of [()=>store.write(id,'main.tex','unsafe'),()=>store.trashProject(id),()=>store.compile(id),()=>store.chatSend(id,'second','default',[])]){
   try{await action();results.push(false);}catch{results.push(true);}
  }return results;
 },id);assert.deepEqual(blocked,[true,true,true,true]);
 await page.locator('#home').click();await page.locator('#list article').first().click();await page.locator('#source').waitFor({state:'visible'});
 assert.equal(await page.locator('#source').isEditable(),false);
 await page.getByRole('status').getByText('Running a command…').waitFor();
 await app.evaluate(()=>{(globalThis as any).__fakeRelease();});
 await page.waitForFunction(()=>{const input=document.querySelector('#source') as HTMLTextAreaElement;return input.value.includes('assistant edit')&&!input.readOnly;});
 assert.equal(await page.getByRole('button',{name:'Commands',exact:true}).isDisabled(),false);
 assert.equal(await page.locator('.chat-progress').isVisible(),false);
 const versions=await page.evaluate(id=>store.versionList(id),id),folder=path.join(data,'projects',id,'.refract-versions'),Zip=require('adm-zip');
 const texts=await Promise.all(versions.map(async v=>new Zip(path.join(folder,v.id+'.zip')).getEntry('main.tex').getData().toString('utf8')));
 assert.ok(texts.some(text=>text.includes('saved before assistant')&&!text.includes('assistant edit')));
 assert.ok(texts.some(text=>text.includes('assistant edit')));
});

test('closing waits for the assistant and preserves its final snapshot', {timeout:60000}, async t=>{
 const data=await fs.mkdtemp(path.join(os.tmpdir(),'refract-ui-assistant-close-'));
 const env: NodeJS.ProcessEnv={...process.env,REFRACT_DATA_DIR:data,REFRACT_TEST_HIDDEN:'1'};delete env.ELECTRON_RUN_AS_NODE;
 let app=await launchDesktop(env,true);t.after(async()=>{if(app)await app.close().catch(()=>{});await fs.rm(data,{recursive:true,force:true,maxRetries:5,retryDelay:200});});
 await app.evaluate(({app})=>{
  const req=(process.getBuiltinModule('module') as any).createRequire(app.getAppPath()+'\\package.json');
  const {ChatService}=req('./build/src/chat.cjs'),io=req('node:fs/promises');
  ChatService.prototype.send=async function(id,prompt){
   await new Promise(resolve=>setTimeout(resolve,1000));
   const file=this.store.filePath(id,'main.tex'),content=await io.readFile(file,'utf8');
   await io.writeFile(file,content+'\n% assistant close edit');
   return {sessionId:null,messages:[{role:'user',text:prompt},{role:'assistant',text:'Done'}]};
  };
 });
 const page=await app.firstWindow();await page.locator('#new').click();await page.locator('#source').waitFor({state:'visible'});
 const id=await page.evaluate(async()=>{const [p]=await store.list();return p.id;});
 const source=await page.locator('#source').inputValue();await page.locator('#source').fill(source+'\n% saved before closing');
 await page.locator('#chat-input').fill('Make a change');await page.locator('#send').click();await app.close();app=null;
 const folder=path.join(data,'projects',id,'.refract-versions'),entries=(await fs.readdir(folder)).filter(name=>name.endsWith('.json'));
 const versions=await Promise.all(entries.map(async name=>JSON.parse(await fs.readFile(path.join(folder,name),'utf8'))));versions.sort((a,b)=>b.sequence-a.sequence);
 const Zip=require('adm-zip'),text=new Zip(path.join(folder,versions[0].id+'.zip')).getEntry('main.tex').getData().toString('utf8');
 assert.match(text,/saved before closing/);assert.match(text,/assistant close edit/);
});

test('find and replace edits the current file and saves through the editor', {timeout:60000}, async t=>{
 const data=await fs.mkdtemp(path.join(os.tmpdir(),'refract-ui-find-'));
 const env: NodeJS.ProcessEnv={...process.env,REFRACT_DATA_DIR:data,REFRACT_TEST_HIDDEN:'1'};delete env.ELECTRON_RUN_AS_NODE;
 const app=await launchDesktop(env);t.after(async()=>{await app.close().catch(()=>{});await fs.rm(data,{recursive:true,force:true,maxRetries:5,retryDelay:200});});
 const page=await app.firstWindow();await page.locator('#new').click();await page.locator('#source').waitFor({state:'visible'});
 const id=await page.evaluate(async()=>{const [p]=await store.list();return p.id;});
 const source=page.locator('#source');await source.fill('alpha beta alpha');await source.press('Control+f');
 await page.getByLabel('Find in file').fill('alpha');await page.getByLabel('Replace with').fill('gamma');
 await page.getByLabel('Wrap search').uncheck();await page.getByLabel('Next match').click();await page.getByLabel('Next match').click();assert.equal(await page.getByRole('status').filter({hasText:'End of file'}).count(),1);
 await page.getByLabel('Wrap search').check();
 await page.getByLabel('Next match').click();await page.getByRole('button',{name:'Replace',exact:true}).click();
 assert.equal(await source.inputValue(),'gamma beta alpha');
 await page.getByRole('button',{name:'All',exact:true}).focus();await page.keyboard.press('Enter');assert.equal(await source.inputValue(),'gamma beta gamma');
 await page.getByLabel('Close find').click();await source.press('Control+z');assert.equal(await source.inputValue(),'gamma beta alpha');
 await source.press('Control+y');assert.equal(await source.inputValue(),'gamma beta gamma');await app.close();
 assert.equal(await fs.readFile(path.join(data,'projects',id,'main.tex'),'utf8'),'gamma beta gamma');
});

test('find can match case insensitively without changing literal search or offsets', {timeout:60000}, async t=>{
 const data:string=await fs.mkdtemp(path.join(os.tmpdir(),'refract-ui-find-options-'));
 const env:NodeJS.ProcessEnv={...process.env,REFRACT_DATA_DIR:data,REFRACT_TEST_HIDDEN:'1'};delete env.ELECTRON_RUN_AS_NODE;
 const app=await launchDesktop(env);t.after(async()=>{
  await app.close().catch(()=>{});const resolved=await fs.realpath(data),parent=await fs.realpath(os.tmpdir());
  if(path.dirname(resolved).toLowerCase()!==parent.toLowerCase() || !path.basename(resolved).startsWith('refract-ui-find-options-'))throw new Error('Unsafe cleanup path');
  await fs.rm(resolved,{recursive:true,force:true,maxRetries:5,retryDelay:200});
 });
 const page=await app.firstWindow();await page.locator('#new').click();const source=page.locator('#source');await source.waitFor({state:'visible'});
 await source.fill('İ A.b a.B axb');await source.press('Control+f');
 await page.getByLabel('Find in file').fill('a.b');await page.getByLabel('Replace with').fill('X');
 assert.equal(await page.getByRole('status').filter({hasText:'No matches'}).count(),1);
 await page.getByLabel('Match case').uncheck();
 await page.getByRole('button',{name:'All',exact:true}).click();assert.equal(await source.inputValue(),'İ X X axb');
 await page.getByLabel('Close find').click();await source.press('Control+z');assert.equal(await source.inputValue(),'İ A.b a.B axb');
});

test('whole-word replacement respects Unicode letters, marks, digits and connectors', {timeout:60000}, async t=>{
 const data:string=await fs.mkdtemp(path.join(os.tmpdir(),'refract-ui-find-word-'));
 const env:NodeJS.ProcessEnv={...process.env,REFRACT_DATA_DIR:data,REFRACT_TEST_HIDDEN:'1'};delete env.ELECTRON_RUN_AS_NODE;
 const app=await launchDesktop(env);t.after(async()=>{await app.close().catch(()=>{});const resolved=await fs.realpath(data),parent=await fs.realpath(os.tmpdir());if(path.dirname(resolved).toLowerCase()!==parent.toLowerCase() || !path.basename(resolved).startsWith('refract-ui-find-word-'))throw new Error('Unsafe cleanup path');await fs.rm(resolved,{recursive:true,force:true,maxRetries:5,retryDelay:200});});
 const page=await app.firstWindow();await page.locator('#new').click();const source=page.locator('#source');await source.waitFor({state:'visible'});
 const original='cat CAT catfish écat caté cat_ cat2 𐐀cat cat𐐀 cat\u0301 (cat)';await source.fill(original);await source.press('Control+f');
 await page.getByLabel('Find in file').fill('cat');await page.getByLabel('Replace with').fill('X');await page.getByLabel('Match case').uncheck();await page.getByLabel('Whole word').check();
 await page.getByRole('button',{name:'All',exact:true}).click();assert.equal(await source.inputValue(),'X X catfish écat caté cat_ cat2 𐐀cat cat𐐀 cat\u0301 (X)');
 await page.getByLabel('Close find').click();await source.press('Control+z');assert.equal(await source.inputValue(),original);
});

test('project search navigates to the original Unicode text offset', {timeout:60000}, async t=>{
 const data:string=await fs.mkdtemp(path.join(os.tmpdir(),'refract-ui-search-unicode-'));
 const {ProjectStore}=require('../src/project-store.cjs'),projectStore=new ProjectStore(path.join(data,'projects')),project=await projectStore.create('Search');
 await projectStore.write(project.id,'main.tex','İ A.b axb');
 const env:NodeJS.ProcessEnv={...process.env,REFRACT_DATA_DIR:data,REFRACT_TEST_HIDDEN:'1'};delete env.ELECTRON_RUN_AS_NODE;
 const app=await launchDesktop(env);t.after(async()=>{await app.close().catch(()=>{});const resolved=await fs.realpath(data),parent=await fs.realpath(os.tmpdir());if(path.dirname(resolved).toLowerCase()!==parent.toLowerCase() || !path.basename(resolved).startsWith('refract-ui-search-unicode-'))throw new Error('Unsafe cleanup path');await fs.rm(resolved,{recursive:true,force:true,maxRetries:5,retryDelay:200});});
 const page=await app.firstWindow();await page.locator('#list article').first().click();const source=page.locator('#source');await source.waitFor({state:'visible'});
 await page.getByRole('textbox',{name:'Search project'}).fill('a.b');await page.locator('#file-search-results button').first().click();
 await page.waitForFunction(()=>{const element=document.querySelector('#source') as HTMLTextAreaElement;return element.value.slice(element.selectionStart,element.selectionEnd)==='A.b';});
 assert.equal(await source.evaluate((element:HTMLTextAreaElement)=>element.value.slice(element.selectionStart,element.selectionEnd)),'A.b');
});

test('editor indents selected lines, outdents and preserves undo and saves', {timeout:60000}, async t=>{
 const data=await fs.mkdtemp(path.join(os.tmpdir(),'refract-ui-indent-'));
 const env: NodeJS.ProcessEnv={...process.env,REFRACT_DATA_DIR:data,REFRACT_TEST_HIDDEN:'1'};delete env.ELECTRON_RUN_AS_NODE;
 const app=await launchDesktop(env);t.after(async()=>{await app.close().catch(()=>{});await fs.rm(data,{recursive:true,force:true,maxRetries:5,retryDelay:200});});
 const page=await app.firstWindow();await page.locator('#new').click();await page.locator('#source').waitFor({state:'visible'});
 const id=await page.evaluate(async()=>{const [p]=await store.list();return p.id;});
 const source=page.locator('#source');await source.fill('one\n  two\nthree');
 await source.evaluate((element: HTMLTextAreaElement)=>element.setSelectionRange(0,10));await source.press('Tab');
 assert.equal(await source.inputValue(),'  one\n    two\nthree');
 await source.press('Shift+Tab');assert.equal(await source.inputValue(),'one\n  two\nthree');
 await source.press('Control+z');assert.equal(await source.inputValue(),'  one\n    two\nthree');
 await source.evaluate((element: HTMLTextAreaElement)=>element.setSelectionRange(2,2));await source.press('Shift+Tab');
 assert.equal(await source.inputValue(),'one\n    two\nthree');
 await source.fill('\n\tone\n  two');await source.evaluate((element: HTMLTextAreaElement)=>element.setSelectionRange(0,0));
 await source.press('Shift+Tab');assert.equal(await source.inputValue(),'\n\tone\n  two');
 await source.evaluate((element: HTMLTextAreaElement)=>element.setSelectionRange(2,2));await source.press('Shift+Tab');
 assert.equal(await source.inputValue(),'\none\n  two');
 await source.fill('one\ntwo\nthree');await source.evaluate((element:HTMLTextAreaElement)=>element.setSelectionRange(0,8,'backward'));await source.press('Tab');
 assert.equal(await source.inputValue(),'  one\n  two\nthree');assert.equal(await source.evaluate((element:HTMLTextAreaElement)=>element.selectionDirection),'backward');
 await source.press('Control+z');assert.equal(await source.inputValue(),'one\ntwo\nthree');
 await app.close();assert.equal(await fs.readFile(path.join(data,'projects',id,'main.tex'),'utf8'),'one\ntwo\nthree');
});

test('syntax highlighting distinguishes escaped percent signs and comments', {timeout:60000}, async t=>{
 const data:string=await fs.mkdtemp(path.join(os.tmpdir(),'refract-ui-highlight-'));
 const env:NodeJS.ProcessEnv={...process.env,REFRACT_DATA_DIR:data,REFRACT_TEST_HIDDEN:'1'};delete env.ELECTRON_RUN_AS_NODE;
 const app=await launchDesktop(env);t.after(async()=>{await app.close().catch(()=>{});await fs.rm(data,{recursive:true,force:true,maxRetries:5,retryDelay:200});});
 const page=await app.firstWindow();await page.locator('#new').click();const source=page.locator('#source');await source.waitFor({state:'visible'});
 await source.fill('\\section{A} \\% literal % \\ignored\n$x$');
 assert.deepEqual(await page.locator('#highlight .tex-command').allTextContents(),['\\section','\\%']);
 assert.deepEqual(await page.locator('#highlight .tex-comment').allTextContents(),['% \\ignored']);
 assert.deepEqual(await page.locator('#highlight .tex-math').allTextContents(),['$','$']);
 assert.equal(await page.locator('#highlight').getAttribute('aria-hidden'),'true');
});

test('LaTeX completion filters commands, supports keyboard selection and undo', {timeout:60000}, async t=>{
 const data:string=await fs.mkdtemp(path.join(os.tmpdir(),'refract-ui-completion-'));
 const env:NodeJS.ProcessEnv={...process.env,REFRACT_DATA_DIR:data,REFRACT_TEST_HIDDEN:'1'};delete env.ELECTRON_RUN_AS_NODE;
 const app=await launchDesktop(env);t.after(async()=>{await app.close().catch(()=>{});const resolved=await fs.realpath(data),parent=await fs.realpath(os.tmpdir());if(path.dirname(resolved).toLowerCase()!==parent.toLowerCase() || !path.basename(resolved).startsWith('refract-ui-completion-'))throw new Error('Unsafe cleanup path');await fs.rm(resolved,{recursive:true,force:true,maxRetries:5,retryDelay:200});});
 const page=await app.firstWindow();await page.locator('#new').click();const source=page.locator('#source');await source.waitFor({state:'visible'});
 await source.fill('\\te');await source.press('Control+Space');assert.equal(await source.getAttribute('aria-expanded'),'true');assert.equal(await page.locator('#latex-completions').getByRole('option').count(),3);
 await source.press('ArrowDown');assert.equal(await page.getByRole('option',{name:'\\textit'}).getAttribute('aria-selected'),'true');await source.press('Enter');
 assert.equal(await source.inputValue(),'\\textit{}');assert.equal(await source.evaluate((el:HTMLTextAreaElement)=>el.selectionStart),8);
 assert.equal(await source.getAttribute('aria-expanded'),'false');await source.press('Control+z');assert.equal(await source.inputValue(),'\\te');
 await source.press('Control+Space');await source.press('Escape');assert.equal(await source.getAttribute('aria-expanded'),'false');
 await source.fill('\\section rest');await source.evaluate((el:HTMLTextAreaElement)=>el.setSelectionRange(4,4));await source.press('Control+Space');await page.getByRole('option',{name:'\\section',exact:true}).click();assert.equal(await source.inputValue(),'\\section{} rest');await source.press('Control+z');assert.equal(await source.inputValue(),'\\section rest');
 await source.fill('\\doc');await source.press('Control+Space');await source.press('Enter');assert.equal(await source.inputValue(),'\\documentclass{}');assert.equal(await source.evaluate((el:HTMLTextAreaElement)=>el.selectionStart),15);
 await source.fill('plain');await source.press('Control+Space');assert.equal(await source.getAttribute('aria-expanded'),'true');await source.press('ArrowLeft');assert.equal(await source.getAttribute('aria-expanded'),'false');
 await page.getByRole('button',{name:'Commands',exact:true}).click();assert.equal(await source.getAttribute('aria-expanded'),'true');await source.press('ArrowUp');assert.equal(await page.getByRole('option',{name:'\\right'}).getAttribute('aria-selected'),'true');assert.ok(await page.locator('#latex-completions').evaluate(el=>el.scrollTop)>0);await page.getByRole('option',{name:'\\begin'}).click();assert.equal(await source.inputValue(),'plai\\begin{}n');
});

test('switching project files preserves each selection and scroll position', {timeout:60000}, async t=>{
 const data:string=await fs.mkdtemp(path.join(os.tmpdir(),'refract-ui-editor-view-'));
 const {ProjectStore}=require('../src/project-store.cjs'),projectStore=new ProjectStore(path.join(data,'projects')),project=await projectStore.create('Editor views');
 const main=Array.from({length:150},(_,i)=>`Main line ${i+1}`).join('\n'),chapter=Array.from({length:120},(_,i)=>`Chapter line ${i+1}`).join('\n');
 await projectStore.write(project.id,'main.tex',main);await projectStore.write(project.id,'chapter.tex',chapter);
 const env:NodeJS.ProcessEnv={...process.env,REFRACT_DATA_DIR:data,REFRACT_TEST_HIDDEN:'1'};delete env.ELECTRON_RUN_AS_NODE;
 const app=await launchDesktop(env);t.after(async()=>{await app.close().catch(()=>{});await fs.rm(data,{recursive:true,force:true,maxRetries:5,retryDelay:200});});
 const page=await app.firstWindow();await page.locator('#list article').first().click();const editor=page.locator('#source');await editor.waitFor({state:'visible'});
 assert.equal(await page.locator('#file-tabs button[title="main.tex"]').getAttribute('aria-current'),'page');
 const first=await editor.evaluate((el:HTMLTextAreaElement)=>{const start=el.value.indexOf('Main line 90');el.setSelectionRange(start,start+12,'backward');el.scrollTop=1100;return {start:el.selectionStart,end:el.selectionEnd,direction:el.selectionDirection,top:el.scrollTop};});
 await page.locator('#file-list button[title="chapter.tex"]').click();await page.waitForFunction(()=>(document.querySelector('#source') as HTMLTextAreaElement).value.startsWith('Chapter line'));
 assert.equal(await page.locator('#file-tabs button[title="chapter.tex"]').getAttribute('aria-current'),'page');assert.equal(await page.locator('#file-tabs button[title="main.tex"]').getAttribute('aria-current'),null);
 const second=await editor.evaluate((el:HTMLTextAreaElement)=>{const start=el.value.indexOf('Chapter line 70');el.setSelectionRange(start,start+15);el.scrollTop=800;return {start:el.selectionStart,end:el.selectionEnd,top:el.scrollTop};});
 await page.getByRole('button',{name:'main.tex',exact:true}).click();await page.waitForFunction(()=>(document.querySelector('#source') as HTMLTextAreaElement).value.startsWith('Main line'));
 assert.deepEqual(await editor.evaluate((el:HTMLTextAreaElement)=>({start:el.selectionStart,end:el.selectionEnd,direction:el.selectionDirection,top:el.scrollTop})),first);
 await page.getByRole('button',{name:'chapter.tex',exact:true}).click();await page.waitForFunction(()=>(document.querySelector('#source') as HTMLTextAreaElement).value.startsWith('Chapter line'));
 assert.deepEqual(await editor.evaluate((el:HTMLTextAreaElement)=>({start:el.selectionStart,end:el.selectionEnd,top:el.scrollTop})),second);
 await page.getByRole('button',{name:'Close chapter.tex'}).focus();await page.keyboard.press('Enter');await page.waitForFunction(()=>(document.querySelector('#source') as HTMLTextAreaElement).value.startsWith('Main line'));
 assert.equal(await page.locator('#file-tabs button[title="main.tex"]').evaluate(el=>document.activeElement===el),true);
 await page.locator('#home').click();await page.locator('#list article').first().click();await page.waitForFunction(()=>(document.querySelector('#source') as HTMLTextAreaElement).value.startsWith('Main line'));
 assert.deepEqual(await editor.evaluate((el:HTMLTextAreaElement)=>({start:el.selectionStart,end:el.selectionEnd,direction:el.selectionDirection,top:el.scrollTop})),first);
});

test('stopping an assistant snapshots partial project changes', {timeout:60000}, async t=>{
 const data=await fs.mkdtemp(path.join(os.tmpdir(),'refract-ui-stop-'));
 const env: NodeJS.ProcessEnv={...process.env,REFRACT_DATA_DIR:data,REFRACT_TEST_HIDDEN:'1'};delete env.ELECTRON_RUN_AS_NODE;
 const app=await launchDesktop(env,true);t.after(async()=>{await app.close().catch(()=>{});await fs.rm(data,{recursive:true,force:true,maxRetries:5,retryDelay:200});});
 await app.evaluate(({app})=>{
  const req=(process.getBuiltinModule('module') as any).createRequire(app.getAppPath()+'\\package.json');
  const {ChatService}=req('./build/src/chat.cjs'),io=req('node:fs/promises');
  (globalThis as any).__stopStarted=new Promise(resolve=>(globalThis as any).__stopNotify=resolve);
  ChatService.prototype.send=async function(id,prompt,_chatId,_attachments,signal){
   const file=this.store.filePath(id,'main.tex');await io.appendFile(file,'\n% partial assistant edit');
   (globalThis as any).__stopNotify();await new Promise<void>(resolve=>signal.addEventListener('abort',()=>resolve(),{once:true}));
   return {sessionId:null,messages:[{role:'user',text:prompt},{role:'error',text:'Assistant operation cancelled.'}]};
  };
 });
 const page=await app.firstWindow();await page.locator('#new').click();await page.locator('#source').waitFor({state:'visible'});
 const id=await page.evaluate(async()=>{const [p]=await store.list();return p.id;});
 await page.locator('#chat-input').fill('Make a change');await page.locator('#send').click();
 await app.evaluate(async()=>await (globalThis as any).__stopStarted);
 assert.equal(await page.locator('#send').isVisible(),false);
 const stop=page.getByRole('button',{name:'Stop',exact:true});assert.equal(await stop.isVisible(),true);
 await stop.click();
 await page.waitForFunction(()=>{const source=document.querySelector('#source') as HTMLTextAreaElement;return !source.readOnly&&source.value.includes('partial assistant edit');});
 assert.equal(await page.locator('#send').isVisible(),true);assert.equal(await stop.isVisible(),false);
 assert.match(await page.locator('#chat-messages').innerText(),/cancelled/);
 const versions=await page.evaluate(id=>store.versionList(id),id),folder=path.join(data,'projects',id,'.refract-versions'),Zip=require('adm-zip');
 assert.match(new Zip(path.join(folder,versions[0].id+'.zip')).getEntry('main.tex').getData().toString('utf8'),/partial assistant edit/);
});

test('send button keeps a centered arrow at wide and narrow composer widths', {timeout:60000}, async t=>{
 const data:string=await fs.mkdtemp(path.join(os.tmpdir(),'refract-ui-send-icon-'));
 const env:NodeJS.ProcessEnv={...process.env,REFRACT_DATA_DIR:data,REFRACT_TEST_HIDDEN:'1'};delete env.ELECTRON_RUN_AS_NODE;
 const app=await launchDesktop(env);t.after(async()=>{await app.close().catch(()=>{});await fs.rm(data,{recursive:true,force:true,maxRetries:5,retryDelay:200});});
 const page=await app.firstWindow();await page.locator('#new').click();await page.locator('#source').waitFor({state:'visible'});
 const measure=()=>page.locator('#send').evaluate((button:HTMLButtonElement)=>{
  const icon=button.querySelector('svg') as SVGSVGElement|null,box=button.getBoundingClientRect(),glyph=icon?icon.getBoundingClientRect():null;
  return {width:box.width,height:box.height,iconWidth:glyph?glyph.width:0,iconHeight:glyph?glyph.height:0,offsetX:glyph?(glyph.left+glyph.width/2)-(box.left+box.width/2):0,offsetY:glyph?(glyph.top+glyph.height/2)-(box.top+box.height/2):0,radius:getComputedStyle(button).borderRadius};
 });
 const check=(label:string,metrics:any)=>{
  assert.equal(metrics.width,36,`${label}: send button keeps its 36px width`);
  assert.equal(metrics.height,36,`${label}: send button keeps its 36px height`);
  assert.equal(metrics.iconWidth,16,`${label}: arrow renders at its intended 16px width`);
  assert.equal(metrics.iconHeight,16,`${label}: arrow renders at its intended 16px height`);
  assert.ok(Math.abs(metrics.offsetX)<0.5,`${label}: arrow is centered horizontally (offset ${metrics.offsetX})`);
  assert.ok(Math.abs(metrics.offsetY)<0.5,`${label}: arrow is centered vertically (offset ${metrics.offsetY})`);
  assert.equal(metrics.radius,'50%',`${label}: send button stays circular`);
 };
 check('wide',await measure());
 await app.evaluate(({BrowserWindow})=>BrowserWindow.getAllWindows()[0].setSize(900,700));await page.waitForFunction(()=>innerWidth<1000);
 check('narrow',await measure());
 assert.equal(await page.getByRole('button',{name:'Send',exact:true}).count(),1);
 assert.equal(await page.locator('#send svg').getAttribute('aria-hidden'),'true');
 await page.evaluate(()=>{(document.querySelector('#send') as HTMLButtonElement).hidden=true;});
 assert.equal(await page.locator('#send').isVisible(),false);
 assert.deepEqual(await page.locator('#send').evaluate((button:HTMLButtonElement)=>{const box=button.getBoundingClientRect();return {width:box.width,height:box.height};}),{width:0,height:0},'a hidden send button leaves no layout space while a response runs');
 await page.evaluate(()=>{(document.querySelector('#send') as HTMLButtonElement).hidden=false;});
 assert.equal(await page.locator('#send').isVisible(),true);
});

test('compiled PDF viewer navigates pages, zooms and disables stale controls', {timeout:90000}, async t=>{
 const data=await fs.mkdtemp(path.join(os.tmpdir(),'refract-ui-pdf-controls-'));
 const env: NodeJS.ProcessEnv={...process.env,REFRACT_DATA_DIR:data,REFRACT_TEST_HIDDEN:'1'};delete env.ELECTRON_RUN_AS_NODE;
 const app=await launchDesktop(env);t.after(async()=>{await app.close().catch(()=>{});await fs.rm(data,{recursive:true,force:true,maxRetries:5,retryDelay:200});});
 const page=await app.firstWindow();await page.locator('#new').click();await page.locator('#source').waitFor({state:'visible'});
 await page.locator('#source').fill('\\documentclass{article}\n\\begin{document}\nFirst page\\newpage Second page\n\\end{document}');
 await page.getByText('Compiled',{exact:true}).waitFor({timeout:45000});
 const prev=page.getByLabel('Previous compiled PDF page'),next=page.getByLabel('Next compiled PDF page'),zoom=page.getByLabel('Zoom in compiled PDF');
 assert.equal(await page.locator('.pdf-page-count').innerText(),'1 / 2');assert.equal(await prev.isDisabled(),true);
 await next.click();await page.waitForFunction(()=>document.querySelector('.pdf-page-count')?.textContent==='2 / 2');
 assert.equal(await next.isDisabled(),true);await prev.click();await page.waitForFunction(()=>document.querySelector('.pdf-page-count')?.textContent==='1 / 2');
 const jump=page.getByLabel('Go to compiled PDF page'),current=page.getByRole('button',{name:'Change compiled PDF page',includeHidden:true});
 assert.equal(await jump.isHidden(),true);
 await current.click();assert.equal(await jump.inputValue(),'1');
 assert.deepEqual(await jump.evaluate((input:HTMLInputElement)=>[input.selectionStart,input.selectionEnd]),[0,1]);
 await jump.fill('2');await jump.press('Enter');await page.waitForFunction(()=>document.querySelector('.pdf-page-count')?.textContent==='2 / 2');
 await current.click();await jump.fill('3');await jump.press('Enter');assert.equal(await current.textContent(),'2');
 await jump.fill('1.5');await jump.press('Enter');assert.equal(await current.textContent(),'2');
 assert.equal(await jump.getAttribute('aria-invalid'),'true');
 await jump.fill('1');await jump.press('Escape');
 assert.equal(await jump.isHidden(),true);assert.equal(await current.evaluate(element=>element===document.activeElement),true);
 assert.equal(await page.locator('.pdf-page-count').innerText(),'2 / 2');
 await current.press('Enter');await jump.fill('1');await jump.press('Tab');
 assert.equal(await jump.isHidden(),true);assert.equal(await page.locator('.pdf-page-count').innerText(),'2 / 2');
 await current.click();await jump.fill('1');await jump.press('Enter');await page.waitForFunction(()=>document.querySelector('.pdf-page-count')?.textContent==='1 / 2');
 const initialWidth: number = await page.locator('.pdf-pages canvas').first().evaluate(element => (element as HTMLCanvasElement).width);
 await zoom.click();assert.equal(await page.locator('.pdf-zoom-value').innerText(),'125%');
 assert.equal(await page.locator('.pdf-pages .pdf-page-frame').first().evaluate(element=>(element as HTMLElement).style.width),'125%');
 await page.waitForFunction(width => (document.querySelector('.pdf-pages canvas') as HTMLCanvasElement)?.width > width, initialWidth);
 const zoomOut=page.getByLabel('Zoom out compiled PDF');
 for(let i=0;i<5;i++) await zoom.click();
 assert.equal(await page.locator('.pdf-zoom-value').innerText(),'250%');
 assert.equal(await zoom.isDisabled(),true);
 for(let i=0;i<9;i++) await zoomOut.click();
 assert.equal(await page.locator('.pdf-zoom-value').innerText(),'25%');
 assert.equal(await zoomOut.isDisabled(),true);
 await zoom.click();assert.equal(await page.locator('.pdf-zoom-value').innerText(),'50%');
 await page.locator('#zoom-fit').click();assert.equal(await page.locator('.pdf-zoom-value').innerText(),'Fit');assert.equal(await page.locator('#zoom-fit').getAttribute('aria-pressed'),'true');
 await page.getByRole('button',{name:'Fit PDF page to view'}).click();assert.equal(await page.getByRole('button',{name:'Fit PDF page to view'}).getAttribute('aria-pressed'),'true');
 await page.locator('#source').fill('unsaved change');assert.equal(await zoom.isDisabled(),true);assert.equal(await jump.isDisabled(),true);
});

test('compiled PDF links open external URLs and navigate to internal destinations', {timeout:90000}, async t=>{
 const data=await fs.mkdtemp(path.join(os.tmpdir(),'refract-ui-pdf-links-'));
 const env:NodeJS.ProcessEnv={...process.env,REFRACT_DATA_DIR:data,REFRACT_TEST_HIDDEN:'1'};delete env.ELECTRON_RUN_AS_NODE;
 const app=await launchDesktop(env);t.after(async()=>{await app.close().catch(()=>{});await fs.rm(data,{recursive:true,force:true,maxRetries:5,retryDelay:200});});
 await app.evaluate(()=>{const electron=(process.getBuiltinModule('module') as any).createRequire(process.cwd()+'\\package.json')('electron');(globalThis as any).__linkOpened=new Promise<string>(resolve=>{electron.shell.openExternal=async url=>{resolve(url);};});});
 const page=await app.firstWindow();await page.locator('#new').click();await page.locator('#source').waitFor({state:'visible'});
 await page.locator('#source').fill('\\documentclass{article}\n\\usepackage{hyperref}\n\\begin{document}\n\\href{https://example.com/path}{External} \\hyperlink{target}{Jump}\\newpage\n\\hypertarget{target}{Destination}\n\\end{document}');
 await page.getByText('Compiled',{exact:true}).waitFor({timeout:60000});
 const external=page.getByLabel('Open PDF link'),internal=page.getByLabel('Go to linked PDF location');
 assert.equal(await external.count(),1);assert.equal(await internal.count(),1);
 await external.click();
 assert.equal(await app.evaluate(async()=>await (globalThis as any).__linkOpened),'https://example.com/path');
 await internal.focus();await internal.press('Enter');await page.waitForFunction(()=>document.querySelector('.pdf-page-count')?.textContent==='2 / 2');
});

test('SyncTeX navigates between a nested source and compiled PDF', {timeout:90000}, async t=>{
 const data:string=await fs.mkdtemp(path.join(os.tmpdir(),'refract-ui-synctex-'));
 const {ProjectStore}=require('../src/project-store.cjs'),projectStore=new ProjectStore(path.join(data,'projects')),project=await projectStore.create('SyncTeX');
 await projectStore.write(project.id,'main.tex','\\documentclass{article}\n\\begin{document}\nFirst page.\\newpage\n\\input{sections/body}\n\\end{document}\n');
 await projectStore.write(project.id,'sections/body.tex','% Padding\n'.repeat(120)+'Nested synchronized text.\n');
 const env:NodeJS.ProcessEnv={...process.env,REFRACT_DATA_DIR:data,REFRACT_TEST_HIDDEN:'1'};delete env.ELECTRON_RUN_AS_NODE;
 const app=await launchDesktop(env);t.after(async()=>{await app.evaluate(()=>{const state=globalThis as any;state.__syncCancel=true;if(state.__syncRelease)state.__syncRelease();}).catch(()=>{});await app.close().catch(()=>{});const resolved=await fs.realpath(data),parent=await fs.realpath(os.tmpdir());if(path.dirname(resolved).toLowerCase()!==parent.toLowerCase() || !path.basename(resolved).startsWith('refract-ui-synctex-'))throw new Error('Unsafe cleanup path');await fs.rm(resolved,{recursive:true,force:true,maxRetries:5,retryDelay:200});});
 const page=await app.firstWindow();const dialogs:string[]=[];page.on('dialog',dialog=>{dialogs.push(dialog.message());void dialog.dismiss();});
 await page.locator('#list article').first().click();await page.getByText('Compiled',{exact:true}).waitFor({timeout:60000});
 await page.locator('#file-list [title="sections/body.tex"]').click();const source=page.locator('#source');await source.waitFor({state:'visible'});
 await source.evaluate((element:HTMLTextAreaElement)=>element.setSelectionRange(element.value.indexOf('Nested'),element.value.indexOf('Nested')));await page.getByRole('button',{name:'Show in PDF'}).click();
 await page.waitForFunction(()=>document.querySelector('.pdf-page-count')?.textContent==='2 / 2');
 const position=await page.evaluate(async id=>store.syncForward(id,'sections/body.tex',121),project.id);
 await page.locator('#file-list [title="main.tex"]').click();
 const canvas=page.locator('.pdf-pages canvas').nth(1);await canvas.scrollIntoViewIfNeeded();
 await canvas.evaluate((element:HTMLCanvasElement,p)=>{const bounds=element.getBoundingClientRect(),scale=Number(element.dataset.renderScale)||1.4;const x=bounds.left+p.x*bounds.width/(element.width/scale),y=bounds.top+p.y*bounds.height/(element.height/scale);element.dispatchEvent(new MouseEvent('dblclick',{bubbles:true,clientX:x,clientY:y}));},position);
 await page.waitForFunction(()=>{const element=document.querySelector('#source') as HTMLTextAreaElement;return element?.value.endsWith('Nested synchronized text.\n') && element.value.slice(element.selectionStart,element.selectionEnd)==='Nested synchronized text.';});
 const assertSourceVisible=async()=>{
  const view=await source.evaluate((element:HTMLTextAreaElement)=>{
   const style=getComputedStyle(element),height=parseFloat(style.lineHeight),line=element.value.slice(0,element.selectionStart).split('\n').length-1;
   const top=parseFloat(style.paddingTop)+line*height-element.scrollTop;
   return {top,bottom:top+height,height:element.clientHeight,scroll:element.scrollTop,gutter:document.querySelector('#numbers')!.scrollTop,highlight:document.querySelector('#highlight')!.scrollTop};
  });
  assert.ok(view.scroll>0);assert.ok(view.top>=0 && view.bottom<=view.height,'Selected source line is visible');
  assert.equal(view.gutter,view.scroll);assert.equal(view.highlight,view.scroll);
 };
 await assertSourceVisible();
 await source.evaluate((element:HTMLTextAreaElement)=>{element.scrollTop=0;});
 await page.locator('#file-list [title="main.tex"]').click();await canvas.focus();await canvas.press('Enter');
 await page.waitForFunction(()=>{const element=document.querySelector('#source') as HTMLTextAreaElement;return element?.value.endsWith('Nested synchronized text.\n') && element.value.slice(element.selectionStart,element.selectionEnd)==='Nested synchronized text.';});
 await assertSourceVisible();
 await app.evaluate(({ipcMain})=>{
  const state=globalThis as any;state.__syncStarted=new Promise<void>(resolve=>{state.__syncNotify=resolve;});
  ipcMain.removeHandler('synctex:forward');ipcMain.handle('synctex:forward',async()=>{state.__syncNotify();await new Promise<void>(resolve=>{state.__syncRelease=resolve;if(state.__syncCancel)resolve();});return {page:2,x:100,y:100};});
 });
 await page.getByRole('button',{name:'Show in PDF'}).click();await app.evaluate(async()=>await (globalThis as any).__syncStarted);
 await source.fill('Changed');assert.equal(await page.getByRole('button',{name:'Show in PDF'}).isDisabled(),true);
 await app.evaluate(()=>{(globalThis as any).__syncRelease();});await page.waitForTimeout(250);assert.equal(await page.locator('.pdf-page-count').innerText(),'0 / 0');
 assert.deepEqual(dialogs,[]);
});

test('SyncTeX navigates an accented LuaLaTeX source file', {timeout:90000}, async t=>{
 const data:string=await fs.mkdtemp(path.join(os.tmpdir(),'refract-ui-sync-unicode-'));
 const {ProjectStore}=require('../src/project-store.cjs'),projectStore=new ProjectStore(path.join(data,'projects')),project=await projectStore.create('SyncTeX Unicode');
 await projectStore.write(project.id,'main.tex','\\documentclass{article}\n\\begin{document}\nFirst.\\newpage\n\\input{sections/résumé}\n\\end{document}\n');
 await projectStore.write(project.id,'sections/résumé.tex','Accented source filename.\n');await projectStore.setEngine(project.id,'lualatex');
 const env:NodeJS.ProcessEnv={...process.env,REFRACT_DATA_DIR:data,REFRACT_TEST_HIDDEN:'1'};delete env.ELECTRON_RUN_AS_NODE;
 const app=await launchDesktop(env);t.after(async()=>{await app.close().catch(()=>{});const resolved=await fs.realpath(data),parent=await fs.realpath(os.tmpdir());if(path.dirname(resolved).toLowerCase()!==parent.toLowerCase() || !path.basename(resolved).startsWith('refract-ui-sync-unicode-'))throw new Error('Unsafe cleanup path');await fs.rm(resolved,{recursive:true,force:true,maxRetries:5,retryDelay:200});});
 const page=await app.firstWindow(),dialogs:string[]=[];page.on('dialog',dialog=>{dialogs.push(dialog.message());void dialog.dismiss();});
 await page.locator('#list article').first().click();await page.getByText('Compiled',{exact:true}).waitFor({timeout:60000});
 await page.locator('#file-list [title="sections/résumé.tex"]').click();await page.getByRole('button',{name:'Show in PDF'}).click();await page.waitForFunction(()=>document.querySelector('.pdf-page-count')?.textContent==='2 / 2');
 const position=await page.evaluate(async id=>store.syncForward(id,'sections/résumé.tex',1),project.id);await page.locator('#file-list [title="main.tex"]').click();
 const canvas=page.locator('.pdf-pages canvas').nth(1);await canvas.scrollIntoViewIfNeeded();
 await canvas.evaluate((element:HTMLCanvasElement,p)=>{const bounds=element.getBoundingClientRect(),scale=Number(element.dataset.renderScale)||1.4;element.dispatchEvent(new MouseEvent('dblclick',{bubbles:true,clientX:bounds.left+p.x*bounds.width/(element.width/scale),clientY:bounds.top+p.y*bounds.height/(element.height/scale)}));},position);
 await page.waitForFunction(()=>{const el=document.querySelector('#source') as HTMLTextAreaElement;return el?.value==='Accented source filename.\n' && el.value.slice(el.selectionStart,el.selectionEnd)==='Accented source filename.';});assert.deepEqual(dialogs,[]);
});

test('long PDFs have content immediately on fast scroll and sharpen pages in view', {timeout:60000}, async t=>{
 const data: string=await fs.mkdtemp(path.join(os.tmpdir(),'refract-ui-long-pdf-'));
 const env: NodeJS.ProcessEnv={...process.env,REFRACT_DATA_DIR:data,REFRACT_TEST_HIDDEN:'1'};delete env.ELECTRON_RUN_AS_NODE;
 const app=await launchDesktop(env);t.after(async()=>{await app.close().catch(()=>{});await fs.rm(data,{recursive:true,force:true,maxRetries:5,retryDelay:200});});
 const page=await app.firstWindow();await page.locator('#new').click();await page.locator('#source').waitFor({state:'visible'});
 await page.locator('#source').fill('\\documentclass{article}\n\\begin{document}\n' + Array.from({length:20},(_,i)=>`Page ${i+1}\\newpage\n`).join('') + '\\end{document}');
 await page.getByText('Compiled',{exact:true}).waitFor({timeout:45000});
 const canvases=page.locator('.pdf-pages canvas');assert.equal(await canvases.count(),20);
 // Check actual painted pixels, including pages that have never been in view.
 const painted = await canvases.evaluateAll(elements=>elements.map(element=>{
   const canvas=element as HTMLCanvasElement, pixels=canvas.getContext('2d')!.getImageData(0,0,canvas.width,canvas.height).data;
   return pixels.some((value,index)=>index%4!==3 && value<200 && pixels[index-index%4+3]>0);
 }));
 assert.deepEqual(painted,Array(20).fill(true));
 // Jump without waiting for the scroll debounce or another render.
 const immediate = await page.evaluate(()=>{
   const area=document.querySelector<HTMLElement>('#pdf-area')!;
   area.scrollTop=area.scrollHeight;
   const canvas=[...area.querySelectorAll('canvas')].at(-1)!;
   return {scale:Number(canvas.dataset.renderScale),width:canvas.width};
 });
 assert.ok(immediate.scale>0);
 await page.waitForFunction(width=>([...document.querySelectorAll('.pdf-pages canvas')].at(-1) as HTMLCanvasElement).width>width,immediate.width);
 const initialWidth:number=await canvases.last().evaluate(el=>(el as HTMLCanvasElement).width);
 await canvases.first().scrollIntoViewIfNeeded();
 await page.getByLabel('Zoom in compiled PDF').click();
 await page.waitForFunction(width=>(document.querySelector('.pdf-pages canvas') as HTMLCanvasElement)?.width>width,initialWidth);
 assert.equal(await canvases.last().evaluate(el=>(el as HTMLCanvasElement).width),initialWidth);
 const jump=page.getByLabel('Go to compiled PDF page');await page.getByRole('button',{name:'Change compiled PDF page'}).click();await jump.fill('12');await jump.press('Enter');
 const pagePosition=()=>canvases.nth(11).evaluate(el=>{
   const area=document.querySelector('#pdf-area') as HTMLElement,bounds=el.getBoundingClientRect();
   return (area.getBoundingClientRect().top+area.clientHeight/2-bounds.top)/bounds.height;
 });
 const position=await pagePosition();
 for(const control of [page.getByLabel('Zoom in compiled PDF'),page.getByLabel('Zoom out compiled PDF'),page.locator('#zoom-fit')]) {
   await control.click();
   await page.waitForTimeout(250);
   assert.equal(await page.locator('.pdf-page-count').innerText(),'12 / 20');
   const actual=await pagePosition();assert.ok(Math.abs(actual-position)<0.01,`Zoom preserves the visible position within the page: ${position} -> ${actual}`);
 }
 await page.getByLabel('Zoom in compiled PDF').click();
 await canvases.last().scrollIntoViewIfNeeded();
 await page.waitForFunction(width=>[...document.querySelectorAll('.pdf-pages canvas')].at(-1) instanceof HTMLCanvasElement && ([...document.querySelectorAll('.pdf-pages canvas')].at(-1) as HTMLCanvasElement).width>width,initialWidth);
 await canvases.first().scrollIntoViewIfNeeded();
 const beforeScaling:number=await canvases.first().evaluate(el=>(el as HTMLCanvasElement).width);
 await canvases.first().focus();
 await page.evaluate(()=>{Object.defineProperty(window,'devicePixelRatio',{configurable:true,value:4});window.dispatchEvent(new Event('resize'));});
 await page.waitForFunction(width=>(document.querySelector('.pdf-pages canvas') as HTMLCanvasElement)?.width>width,beforeScaling);
 assert.equal(await canvases.first().evaluate(el=>document.activeElement===el),true);
 const resolution=await canvases.first().evaluate(el=>({width:(el as HTMLCanvasElement).width,height:(el as HTMLCanvasElement).height,displayWidth:el.getBoundingClientRect().width}));
 assert.ok(resolution.width>=Math.floor(resolution.displayWidth*4)-1);assert.ok(resolution.width*resolution.height<=16010000);
});

test('app startup recovers a project interrupted during restore', {timeout:60000}, async t => {
 const data: string = await fs.mkdtemp(path.join(os.tmpdir(),'refract-ui-recover-'));
 const {ProjectStore} = require('../src/project-store.cjs'), {Versions} = require('../src/versions.cjs');
 const store = new ProjectStore(path.join(data,'projects')), project = await store.create('Paper'), versions = new Versions(store);
 await store.write(project.id,'a.txt','old'); const old = await versions.snapshot(project.id); await store.write(project.id,'a.txt','new');
 const {spawn} = require('node:child_process');
 const code: number = await new Promise((resolve,reject) => { const child = spawn(process.execPath,[path.join(__dirname,'fixtures','restore-child.cjs'),store.root,project.id,old.id],{windowsHide:true,stdio:'ignore'}); child.on('error',reject); child.on('close',resolve); });
 assert.equal(code,73);
 const env: NodeJS.ProcessEnv = {...process.env,REFRACT_DATA_DIR:data,REFRACT_TEST_HIDDEN:'1'}; delete env.ELECTRON_RUN_AS_NODE;
 const app = await launchDesktop(env);
 t.after(async()=>{await app.close().catch(()=>{});await fs.rm(data,{recursive:true,force:true,maxRetries:5,retryDelay:200});});
 const page = await app.firstWindow(); await page.locator('#list article').first().waitFor();
 assert.equal(await store.read(project.id,'a.txt'),'new');
 await assert.rejects(fs.access(path.join(store.projectDir(project.id),'.refract-restore-journal.json')),{code:'ENOENT'});
});

test('assistant Markdown and math render safely after restarting', {timeout:60000}, async t=>{
 const data:string=await fs.mkdtemp(path.join(os.tmpdir(),'refract-ui-format-'));
 const {ProjectStore}=require('../src/project-store.cjs'),{ChatService}=require('../src/chat.cjs');
 const projectStore=new ProjectStore(path.join(data,'projects')),project=await projectStore.create('Formatting');
 const response='**Bold** and $x^2$ and \\(y^2\\)\n\n$$\\frac{1}{2}$$\n\n\\[z=1\\]\n\n```tex\n$literal$\n```\n\n<img src="https://example.com/tracker" onerror="window.injected=1">\n<script>window.injected=1</script>\n\n[unsafe](javascript:alert(1)) [file](file:///secret) [paper](https://example.com/paper)';
 await new ChatService(projectStore).save(project.id,{sessionId:null,messages:[{role:'user',text:'**plain user message**'},{role:'assistant',text:response}]});
 const env:NodeJS.ProcessEnv={...process.env,REFRACT_DATA_DIR:data,REFRACT_TEST_HIDDEN:'1'};delete env.ELECTRON_RUN_AS_NODE;
 let app:any=null;t.after(async()=>{if(app)await app.close().catch(()=>{});await fs.rm(data,{recursive:true,force:true,maxRetries:5,retryDelay:200});});
 for(let run=0;run<2;run++) {
  app=await launchDesktop(env);
  await app.evaluate(()=>{const electron=(process.getBuiltinModule('module') as any).createRequire(process.cwd()+'\\package.json')('electron');(globalThis as any).__opened=[];electron.shell.openExternal=async url=>{(globalThis as any).__opened.push(url);};});
  const page=await app.firstWindow();await page.locator('#list article').first().click();
  const reply=page.locator('.message.assistant .formatted-response');await reply.waitFor();
  assert.equal(await reply.locator('strong').innerText(),'Bold');assert.equal(await reply.locator('math').count(),4);
  assert.match(await reply.locator('pre code').innerText(),/\$literal\$/);
  assert.equal(await reply.locator('img,script,iframe').count(),0);
  assert.equal(await reply.locator('a[href]').count(),1);
  assert.deepEqual(await app.evaluate(()=>(globalThis as any).__opened),[]);
  await reply.getByRole('link',{name:'paper'}).click();
  await app.evaluate(async()=>{for(let i=0;i<100 && !(globalThis as any).__opened.length;i++)await new Promise(resolve=>setTimeout(resolve,10));});
  assert.deepEqual(await app.evaluate(()=>(globalThis as any).__opened),['https://example.com/paper']);
  for(const invalid of ['file:///secret','javascript:alert(1)','https://user:pass@example.com/']) await assert.rejects(page.evaluate(url=>store.openExternal(url),invalid));
  assert.equal(await page.evaluate(()=>(window as any).injected),undefined);
  assert.equal(await page.locator('.message.user').innerText(),'**plain user message**');
  await app.close();app=null;
 }
});

test('rapid chat switching keeps the latest conversation and separate drafts', {timeout:60000}, async t=>{
 const data:string=await fs.mkdtemp(path.join(os.tmpdir(),'refract-ui-chat-switch-'));
 const {ProjectStore}=require('../src/project-store.cjs'),{ChatService}=require('../src/chat.cjs');
 const projectStore=new ProjectStore(path.join(data,'projects')),project=await projectStore.create('Chat switching'),chats=new ChatService(projectStore);
 const first=await chats.create(project.id),second=await chats.create(project.id);
 await chats.save(project.id,{sessionId:null,messages:[{role:'user',text:'First conversation'},{role:'assistant',text:'First reply'}]},first);
 await chats.save(project.id,{sessionId:null,messages:[{role:'user',text:'Second conversation'},{role:'assistant',text:'Second reply'}]},second);
 const env:NodeJS.ProcessEnv={...process.env,REFRACT_DATA_DIR:data,REFRACT_TEST_HIDDEN:'1'};delete env.ELECTRON_RUN_AS_NODE;
 const app=await launchDesktop(env);t.after(async()=>{await app.close().catch(()=>{});await fs.rm(data,{recursive:true,force:true,maxRetries:5,retryDelay:200});});
 const page=await app.firstWindow();await page.locator('#list article').first().click();await page.locator('#source').waitFor({state:'visible'});
 await page.locator('#chats-tab').click();await page.getByRole('button',{name:'First conversation',exact:true}).waitFor();
 await page.locator('#chat-input').fill('Default draft');
 await app.evaluate(({app},firstId)=>{
  const req=(process.getBuiltinModule('module') as any).createRequire(app.getAppPath()+'\\package.json'),{ChatService}=req('./build/src/chat.cjs'),load=ChatService.prototype.load;
  (globalThis as any).__heldChat=new Promise(resolve=>(globalThis as any).__releaseChat=resolve);
  (globalThis as any).__chatStarted=new Promise(resolve=>(globalThis as any).__chatNotify=resolve);
  ChatService.prototype.load=async function(id,chatId){if(chatId===firstId){(globalThis as any).__chatNotify();await (globalThis as any).__heldChat;}return load.call(this,id,chatId);};
 },first);
 await page.getByRole('button',{name:'First conversation',exact:true}).click();await app.evaluate(async()=>await (globalThis as any).__chatStarted);
 assert.equal(await page.locator('#send').isDisabled(),true);
 await page.getByRole('button',{name:'Second conversation',exact:true}).click();await page.getByText('Second reply',{exact:true}).waitFor();
 assert.equal(await page.locator('#chat-input').inputValue(),'');await page.locator('#chat-input').fill('Second draft');
 await app.evaluate(()=>(globalThis as any).__releaseChat());
 await page.getByRole('button',{name:'Chat',exact:true}).click();await page.waitForFunction(()=>(document.querySelector('#chat-input') as HTMLTextAreaElement).value==='Default draft');
 await page.getByRole('button',{name:'Second conversation',exact:true}).click();await page.getByText('Second reply',{exact:true}).waitFor();
 assert.equal(await page.locator('#chat-input').inputValue(),'Second draft');assert.equal(await page.getByText('First reply',{exact:true}).count(),0);
});

test('failed send reload cannot replace a newly selected conversation', {timeout:60000}, async t=>{
 const data:string=await fs.mkdtemp(path.join(os.tmpdir(),'refract-ui-chat-reload-'));
 const {ProjectStore}=require('../src/project-store.cjs'),{ChatService}=require('../src/chat.cjs');
 const projectStore=new ProjectStore(path.join(data,'projects')),project=await projectStore.create('Chat reload'),chats=new ChatService(projectStore);
 const other=await chats.create(project.id);
 await chats.save(project.id,{sessionId:null,messages:[{role:'user',text:'Other conversation'},{role:'assistant',text:'Other reply'}]},other);
 const env:NodeJS.ProcessEnv={...process.env,REFRACT_DATA_DIR:data,REFRACT_TEST_HIDDEN:'1'};delete env.ELECTRON_RUN_AS_NODE;
 const app=await launchDesktop(env,true);t.after(async()=>{await app.close().catch(()=>{});await fs.rm(data,{recursive:true,force:true,maxRetries:5,retryDelay:200});});
 const page=await app.firstWindow();await page.locator('#list article').first().click();await page.locator('#source').waitFor({state:'visible'});
 await page.locator('#chats-tab').click();await page.getByRole('button',{name:'Other conversation',exact:true}).waitFor();
 await app.evaluate(({ipcMain,app})=>{
  const state=globalThis as any,req=(process.getBuiltinModule('module') as any).createRequire(app.getAppPath()+'\\package.json'),{ChatService}=req('./build/src/chat.cjs'),load=ChatService.prototype.load;
  state.__reloadStarted=new Promise<void>(resolve=>state.__reloadNotify=resolve);
  let held=false;ChatService.prototype.load=async function(id,chatId){if(chatId==='default' && !held){held=true;state.__reloadNotify();await new Promise<void>(resolve=>state.__reloadRelease=resolve);}return load.call(this,id,chatId);};
  ipcMain.removeHandler('chat:send');ipcMain.handle('chat:send',()=>{throw new Error('Simulated failure');});
 });
 await page.locator('#chat-input').fill('Retry me');await page.locator('#send').click();
 await app.evaluate(async()=>await (globalThis as any).__reloadStarted);
 await page.getByRole('dialog', {name:'Could not send message'}).getByRole('button', {name:'Close'}).click();
 await page.getByRole('button',{name:'Other conversation',exact:true}).click();await page.getByText('Other reply',{exact:true}).waitFor();
 await app.evaluate(()=>(globalThis as any).__reloadRelease());await page.waitForFunction(()=>!(document.querySelector('#send') as HTMLButtonElement).disabled);
 assert.equal(await page.getByText('Other reply',{exact:true}).count(),1);
 await page.getByRole('button',{name:'Chat',exact:true}).click();await page.waitForFunction(()=>(document.querySelector('#chat-input') as HTMLTextAreaElement).value==='Retry me');
});

test('unsent chat text and attachments survive restart and removal', {timeout:60000}, async t=>{
 const data:string=await fs.mkdtemp(path.join(os.tmpdir(),'refract-ui-chat-draft-'));
 const {ProjectStore}=require('../src/project-store.cjs'),projectStore=new ProjectStore(path.join(data,'projects')),project=await projectStore.create('Draft project');
 const attachment=path.join(data,'notes.txt');await fs.writeFile(attachment,'source notes');
 const env:NodeJS.ProcessEnv={...process.env,REFRACT_DATA_DIR:data,REFRACT_TEST_HIDDEN:'1'};delete env.ELECTRON_RUN_AS_NODE;
 let app:any=null;t.after(async()=>{if(app)await app.close().catch(()=>{});await fs.rm(data,{recursive:true,force:true,maxRetries:5,retryDelay:200});});
 const open=async()=>{app=await launchDesktop(env);const page=await app.firstWindow();await page.locator('#list article').first().click();await page.locator('#source').waitFor({state:'visible'});return page;};
 let page=await open();await page.locator('#chat-input').fill('A draft to keep');
 await app.evaluate(({dialog},file)=>{dialog.showOpenDialog=async()=>({canceled:false,filePaths:[file]});},attachment);
 await page.locator('#attach').click();await page.getByRole('button',{name:'Remove notes.txt'}).waitFor();
 await app.close();app=null;
 page=await open();assert.equal(await page.locator('#chat-input').inputValue(),'A draft to keep');
 await page.getByRole('button',{name:'Remove notes.txt'}).click();await page.locator('#chat-input').fill('');
 await app.close();app=null;
 page=await open();assert.equal(await page.locator('#chat-input').inputValue(),'');assert.equal(await page.locator('#attachment-list button').count(),0);
 assert.equal((await projectStore.list()).some(p=>p.id===project.id),true);
});

test('attachment selection enforces the message limit and blocks sending while pending', {timeout:90000}, async t=>{
 const data:string=await fs.mkdtemp(path.join(os.tmpdir(),'refract-ui-attachment-limit-'));
 const {ProjectStore}=require('../src/project-store.cjs'),{ChatService}=require('../src/chat.cjs'),projectStore=new ProjectStore(path.join(data,'projects')),project=await projectStore.create('Attachments'),chats=new ChatService(projectStore);
 const other=await chats.create(project.id);await chats.save(project.id,{sessionId:null,messages:[{role:'user',text:'Other conversation'}]},other);
 const file=path.join(data,'notes.txt');await fs.writeFile(file,'notes');
 const env:NodeJS.ProcessEnv={...process.env,REFRACT_DATA_DIR:data,REFRACT_TEST_HIDDEN:'1'};delete env.ELECTRON_RUN_AS_NODE;
 const app=await launchDesktop(env);t.after(async()=>{await app.evaluate(()=>{const state=globalThis as any;state.__attachCancel=true;if(state.__attachRelease)state.__attachRelease();}).catch(()=>{});await app.close().catch(()=>{});const resolved=await fs.realpath(data),parent=await fs.realpath(os.tmpdir());if(path.dirname(resolved).toLowerCase()!==parent.toLowerCase() || !path.basename(resolved).startsWith('refract-ui-attachment-limit-'))throw new Error('Unsafe cleanup path');await fs.rm(resolved,{recursive:true,force:true,maxRetries:5,retryDelay:200});});
 const page=await app.firstWindow(),dialogs:string[]=[];page.on('dialog',dialog=>{dialogs.push(dialog.message());void dialog.dismiss();});await app.evaluate(({ipcMain})=>{ipcMain.removeHandler('providers:discover');ipcMain.handle('providers:discover',()=>({codex:true,opencode:false}));});await page.locator('#list article').first().click();await page.locator('#source').waitFor({state:'visible'});
 await app.evaluate(({dialog},file)=>{dialog.showOpenDialog=async()=>({canceled:false,filePaths:Array(10).fill(file)});},file);
 await page.locator('#attach').click();await page.waitForFunction(()=>document.querySelectorAll('#attachment-list button').length===10);assert.equal(await page.locator('#attach').isDisabled(),true);
 await page.locator('#attachment-list button').first().click();assert.equal(await page.locator('#attach').isDisabled(),false);
 await app.evaluate(({dialog},file)=>{dialog.showOpenDialog=async()=>({canceled:false,filePaths:[file,file]});},file);
 await page.locator('#attach').click();await page.waitForFunction(()=>!(document.querySelector('#attach') as HTMLButtonElement).disabled);await page.locator('#app-dialog').getByText(/at most 1/).waitFor(); await page.locator('#app-dialog-action').click(); assert.deepEqual(dialogs,[]);
 assert.equal((await fs.readdir(path.join(projectStore.projectDir(project.id),'.refract-attachments'))).length,10);
 await app.evaluate(({dialog,ipcMain},file)=>{const state=globalThis as any;state.__sent=0;state.__attachStarted=new Promise<void>(resolve=>{state.__attachNotify=resolve;});dialog.showOpenDialog=async()=>{state.__attachNotify();await new Promise<void>(resolve=>{state.__attachRelease=resolve;if(state.__attachCancel)resolve();});return {canceled:false,filePaths:[file]};};ipcMain.removeHandler('chat:send');ipcMain.handle('chat:send',async()=>{state.__sent++;return {messages:[]};});},file);
 await page.locator('#chat-input').fill('Wait for attachment');await page.locator('#attach').click();await app.evaluate(async()=>await (globalThis as any).__attachStarted);assert.equal(await page.locator('#send').isDisabled(),true);await page.locator('#send').dispatchEvent('click');assert.equal(await app.evaluate(()=>(globalThis as any).__sent),0);
 await page.locator('#chats-tab').click();await page.getByRole('button',{name:'Other conversation',exact:true}).click();await page.getByText('Other conversation',{exact:true}).last().waitFor();await page.waitForFunction(()=>!(document.querySelector('#send') as HTMLButtonElement).disabled);
 await page.getByRole('button',{name:'Chat',exact:true}).click();await page.waitForFunction(()=>(document.querySelector('#chat-input') as HTMLTextAreaElement).value==='Wait for attachment');assert.equal(await page.locator('#send').isDisabled(),true);assert.equal(await page.locator('#attach').isDisabled(),true);
 await app.evaluate(()=>{(globalThis as any).__attachRelease();});await page.waitForFunction(()=>document.querySelectorAll('#attachment-list button').length===10);assert.equal(await page.locator('#send').isDisabled(),false);assert.equal(await page.locator('#chat-input').inputValue(),'Wait for attachment');
});

test('saved Codex preferences and explicit snapshots reach chat turns', {timeout:60000}, async t=>{
 const data=await fs.mkdtemp(path.join(os.tmpdir(),'refract-ui-preferences-'));
 const env: NodeJS.ProcessEnv={...process.env,REFRACT_DATA_DIR:data,REFRACT_TEST_HIDDEN:'1'};delete env.ELECTRON_RUN_AS_NODE;
 const app=await launchDesktop(env);
 t.after(async()=>{await app.close().catch(()=>{});await fs.rm(data,{recursive:true,force:true,maxRetries:5,retryDelay:200});});
 await app.evaluate(({app,ipcMain})=>{
  ipcMain.removeHandler('providers:discover');
  ipcMain.handle('providers:discover',()=>({codex:true,opencode:false}));
  ipcMain.removeHandler('codex:discover-models');
  ipcMain.handle('codex:discover-models',()=>({ok:true,models:[{
   id:'available-model',model:'available-model',displayName:'Available model',description:'',
   supportedReasoningEfforts:[{effort:'low',description:''}],defaultReasoningEffort:'low',isDefault:true
  }]}));
  const req=(process.getBuiltinModule('module') as any).createRequire(app.getAppPath()+'\\package.json');
  const {ChatService,codexArguments}=req('./build/src/chat.cjs'), original=ChatService.prototype.send;
  (globalThis as any).__codexRuns=[];
  ChatService.prototype.send=function(...args){
   this.runner=async (_cwd,session,_prompt,images,_signal,_progress,_onSession,options)=>{
    (globalThis as any).__codexRuns.push({session,args:codexArguments('codex.js',session,images,options)});
    return {sessionId:'thread',text:'ok'};
   };
   return original.apply(this,args);
  };
 });
 const page=await app.firstWindow();await page.waitForLoadState('load');await page.reload();
 await page.locator('#new').click();await page.locator('#source').waitFor({state:'visible'});
 await page.waitForFunction(()=>!(document.querySelector('#chat-provider') as HTMLSelectElement).disabled);
 await page.evaluate(async()=>{
  const pickerWindow=window as Window & {codexPickerReady?: (provider: 'codex') => Promise<Omit<import('../src/codex-preferences.cjs').CodexPreferences, 'favorites'>>};
  if (!pickerWindow.codexPickerReady) throw new Error('Model picker is unavailable');
  const api=store, [project]=await api.list();
  const selection=await pickerWindow.codexPickerReady('codex');
  await api.chatSend(project.id,'default','default',[],'codex',selection);
  await api.codexPreferencesSave({model:'--vendor/model with space',reasoningEffort:'high',favorites:[]});
  const chat=await api.chatCreate(project.id);
  await api.chatSend(project.id,'new',chat,[]);
  await api.chatSend(project.id,'resume',chat,[]);
  await api.codexPreferencesSave({model:'other-model',reasoningEffort:'low',favorites:[],fastMode:true});
  await api.chatSend(project.id,'snapshot',chat,[],'codex',{model:'chosen-model',reasoningEffort:'high',fastMode:false});
 });
 const runs=await app.evaluate(()=>(globalThis as any).__codexRuns);
 assert.equal(runs.length,4);
 assert.equal(runs[0].session,null);assert.ok(!runs[0].args.some((arg:string)=>arg.startsWith('--model')));
 assert.ok(runs[0].args.includes('model_reasoning_effort="low"'));
 assert.ok(runs[0].args.includes('service_tier="default"'));
 assert.equal(runs[1].session,null);assert.equal(runs[2].session,'thread');
 for(const run of runs.slice(1,3)){assert.ok(run.args.includes('--model=--vendor/model with space'));assert.ok(run.args.includes('model_reasoning_effort="high"'));}
 assert.equal(runs[3].session,'thread');
 assert.ok(runs[3].args.includes('--model=chosen-model'));
 assert.ok(runs[3].args.includes('model_reasoning_effort="high"'));
 assert.ok(runs[3].args.includes('service_tier="default"'));
});

test('chat provider selection remains available for a started conversation', {timeout:60000}, async t=>{
 const data=await fs.mkdtemp(path.join(os.tmpdir(),'refract-ui-provider-'));
 const env:NodeJS.ProcessEnv={...process.env,REFRACT_DATA_DIR:data,REFRACT_TEST_HIDDEN:'1'};delete env.ELECTRON_RUN_AS_NODE;
 const app=await launchDesktop(env);
 t.after(async()=>{await app.close().catch(()=>{});await fs.rm(data,{recursive:true,force:true,maxRetries:5,retryDelay:200});});
 const page=await app.firstWindow();
 await app.evaluate(({ipcMain})=>{ipcMain.removeHandler('providers:discover');ipcMain.handle('providers:discover',()=>({codex:true,opencode:true}));ipcMain.removeHandler('opencode:discover-models');ipcMain.handle('opencode:discover-models',()=>({ok:true,models:[{id:'test/model',name:'model',variants:[]}]}));ipcMain.removeHandler('chat:send');ipcMain.handle('chat:send',async (_event,_id,prompt,_chatId,_attachments,provider)=>{(globalThis as any).__provider=provider;return {provider,sessionId:'session',messages:[{role:'user',text:prompt},{role:'assistant',text:'Done'}]};});});
 await page.waitForLoadState('load'); await page.reload();
 await page.locator('#new').click();await page.locator('#source').waitFor({state:'visible'});
 const provider=page.locator('#chat-provider');await page.waitForFunction(()=>!(document.querySelector('#chat-provider') as HTMLSelectElement).disabled);assert.equal(await provider.inputValue(),'codex');assert.equal(await provider.isEnabled(),true);
 await page.locator('.model-trigger').click();await page.getByLabel('OpenCode models').click();assert.equal(await provider.inputValue(),'codex');await page.getByRole('button',{name:'model',exact:true}).click();
 await page.locator('#chat-input').fill('Hello');await page.locator('#send').click();await page.locator('#chat-messages').getByText('Done',{exact:true}).waitFor();
 assert.equal(await app.evaluate(()=>(globalThis as any).__provider),'opencode');await page.waitForFunction(()=>!(document.querySelector('#chat-provider') as HTMLSelectElement).disabled);
 await page.locator('.model-trigger').click();await page.getByLabel('Codex models').click();assert.equal(await provider.inputValue(),'opencode');await page.getByRole('button',{name:'CLI default',exact:true}).click();assert.equal(await provider.inputValue(),'codex');await page.keyboard.press('Escape');
 await page.getByRole('button',{name:'New chat',exact:false}).click();await page.waitForFunction(()=>!document.querySelector('.message'));assert.equal(await page.locator('#chat-input').inputValue(),'');
});

test('new chats offer only installed providers', {timeout:60000}, async t=>{
 const data=await fs.mkdtemp(path.join(os.tmpdir(),'refract-ui-discovery-'));
 const env:NodeJS.ProcessEnv={...process.env,REFRACT_DATA_DIR:data,REFRACT_TEST_HIDDEN:'1'};delete env.ELECTRON_RUN_AS_NODE;
 const app=await launchDesktop(env);
 t.after(async()=>{await app.close().catch(()=>{});await fs.rm(data,{recursive:true,force:true,maxRetries:5,retryDelay:200});});
 const page=await app.firstWindow();
 await app.evaluate(({ipcMain})=>{ipcMain.removeHandler('providers:discover');ipcMain.handle('providers:discover',()=>({codex:false,opencode:true}));ipcMain.removeHandler('opencode:discover-models');ipcMain.handle('opencode:discover-models',()=>({ok:true,models:[{id:'test/model',name:'model',variants:[]}]}));});
 await page.waitForLoadState('load'); await page.reload();
 await page.locator('#new').click();await page.locator('#source').waitFor({state:'visible'});
 await page.waitForFunction(()=>!(document.querySelector('#chat-provider') as HTMLSelectElement).disabled);
 assert.deepEqual(await page.locator('#chat-provider option').evaluateAll(options=>options.map(option=>(option as HTMLOptionElement).value)),['opencode']);
 assert.equal(await page.locator('#chat-provider').inputValue(),'opencode');
 await page.waitForFunction(()=>document.querySelector('.model-trigger-label')?.textContent==='model');
});

test('saved chats retain an unavailable provider and cannot send', {timeout:60000}, async t=>{
 const data=await fs.mkdtemp(path.join(os.tmpdir(),'refract-ui-missing-provider-'));
 const env:NodeJS.ProcessEnv={...process.env,REFRACT_DATA_DIR:data,REFRACT_TEST_HIDDEN:'1'};delete env.ELECTRON_RUN_AS_NODE;
 const app=await launchDesktop(env);
 t.after(async()=>{await app.close().catch(()=>{});await fs.rm(data,{recursive:true,force:true,maxRetries:5,retryDelay:200});});
 const page=await app.firstWindow();
 await app.evaluate(({ipcMain})=>{
  ipcMain.removeHandler('providers:discover');ipcMain.handle('providers:discover',()=>({codex:true,opencode:false}));
  ipcMain.removeHandler('chat:load');ipcMain.handle('chat:load',()=>({provider:'opencode',sessionId:'session',messages:[{role:'user',text:'Previous message'}]}));
 });
 await page.locator('#new').click();await page.getByText('Previous message',{exact:true}).waitFor();
 await page.waitForFunction(()=>document.querySelector('#chat-provider')?.textContent?.includes('not installed'));
 assert.equal(await page.locator('#chat-provider').inputValue(),'opencode');
 assert.equal(await page.locator('#chat-provider').isEnabled(),true);
 await page.locator('#chat-input').fill('New message');
 assert.equal(await page.locator('#send').isDisabled(),true);
 await page.locator('.model-trigger').click();await page.getByLabel('OpenCode models').click();await page.getByText('OpenCode unavailable',{exact:true}).last().waitFor();await page.getByLabel('Codex models').click();await page.getByRole('button',{name:'CLI default',exact:true}).click();
 assert.equal(await page.locator('#chat-provider').inputValue(),'codex');assert.equal(await page.locator('#send').isEnabled(),true);
});


test('handoff marker persists and new chat starts empty', {timeout:60000}, async t=>{
 const data=await fs.mkdtemp(path.join(os.tmpdir(),'refract-ui-handoff-'));
 const {ProjectStore}=require('../src/project-store.cjs'),{ChatService}=require('../src/chat.cjs');
 const projectStore=new ProjectStore(path.join(data,'projects')),project=await projectStore.create('Handoff example'),chats=new ChatService(projectStore);
 await chats.save(project.id,{provider:'opencode',sessionId:'session',messages:[{role:'user',text:'Help me improve my introduction.'},{role:'assistant',text:'I can help clarify the main argument.'},{role:'handoff',from:'codex',to:'opencode',status:'complete'},{role:'user',text:'Suggest a more concise opening.'},{role:'assistant',text:'Here is a shorter opening for your introduction.'}]});
 const env:NodeJS.ProcessEnv={...process.env,REFRACT_DATA_DIR:data,REFRACT_TEST_HIDDEN:'1'};delete env.ELECTRON_RUN_AS_NODE;
 const app=await launchDesktop(env);t.after(async()=>{await app.close().catch(()=>{});await fs.rm(data,{recursive:true,force:true,maxRetries:5,retryDelay:200});});
 const page=await app.firstWindow();await page.locator('#list article').first().click();
 const marker=page.getByLabel('Context handoff: Codex to OpenCode');await marker.waitFor();assert.match(await marker.innerText(),/Codex.*OpenCode/);
 if(process.env.REFRACT_SCREENSHOTS) await page.locator('#chat').screenshot({path:path.join(process.env.REFRACT_SCREENSHOTS,'handoff.png')});
 await page.getByRole('button',{name:'New chat',exact:true}).click();await page.waitForFunction(()=>!document.querySelector('.message'));
 assert.equal(await marker.count(),0);assert.equal(await page.locator('#chat-input').inputValue(),'');
 if(process.env.REFRACT_SCREENSHOTS) await page.locator('#chat').screenshot({path:path.join(process.env.REFRACT_SCREENSHOTS,'new-chat.png')});
 const newChat=page.getByRole('button',{name:'New chat',exact:true}), tooltip=page.locator('.new-chat-tooltip');
 await page.locator('#chat-input').focus();
 await newChat.hover();assert.equal(await tooltip.isVisible(),true);assert.equal(await tooltip.innerText(),'New chat');
 if(process.env.REFRACT_SCREENSHOTS) await page.locator('#chat').screenshot({path:path.join(process.env.REFRACT_SCREENSHOTS,'new-chat-tooltip.png')});
 await page.locator('#chat-input').hover();assert.equal(await tooltip.isHidden(),true);
 await page.locator('#chat-input').focus();await page.keyboard.press('Shift+Tab');await page.keyboard.press('Shift+Tab');assert.equal(await newChat.evaluate(el=>el===document.activeElement),true);assert.equal(await tooltip.isVisible(),true);
});
test('outline navigation restores horizontal visibility', {timeout:30000}, async t => {
 const data = await fs.mkdtemp(path.join(os.tmpdir(), 'refract-outline-'));
 const env: NodeJS.ProcessEnv = {...process.env, REFRACT_DATA_DIR:data, REFRACT_TEST_HIDDEN:'1'}; delete env.ELECTRON_RUN_AS_NODE;
 const app = await launchDesktop(env);
 t.after(async () => { await app.close().catch(() => {}); await fs.rm(data,{recursive:true,force:true,maxRetries:5,retryDelay:200}); });
 const page = await app.firstWindow(); await page.locator('#new').click();
 const editor = page.locator('#source'); await editor.waitFor({state:'visible'});
 await editor.fill('x'.repeat(1000)+'\n\\section{Start}\n'+' '.repeat(200)+'\\section{Indented}');
 await editor.evaluate((el:HTMLTextAreaElement) => { el.scrollLeft = el.scrollWidth; });
 await page.locator('#outline-list').getByRole('button',{name:'Start'}).click();
 assert.equal(await editor.evaluate((el:HTMLTextAreaElement) => el.scrollLeft), 0);
 await page.locator('#outline-list').getByRole('button',{name:'Indented'}).click();
 assert.ok(await editor.evaluate((el:HTMLTextAreaElement) => el.scrollLeft) > 0);
});

test('editor build waiting for a write cannot race an assistant operation',{timeout:60000},async t=>{
 const data=await fs.mkdtemp(path.join(os.tmpdir(),'refract-build-race-'));
 const env:NodeJS.ProcessEnv={...process.env,REFRACT_DATA_DIR:data,REFRACT_TEST_HIDDEN:'1'};delete env.ELECTRON_RUN_AS_NODE;
 const app=await launchDesktop(env);t.after(async()=>{await app.evaluate(()=>{(globalThis as any).__releaseRaceWrite?.();}).catch(()=>{});await app.close().catch(()=>{});const resolved=await fs.realpath(data),parent=await fs.realpath(os.tmpdir());if(path.dirname(resolved).toLowerCase()!==parent.toLowerCase() || !path.basename(resolved).startsWith('refract-build-race-'))throw new Error('Unsafe cleanup path');await fs.rm(resolved,{recursive:true,force:true,maxRetries:5,retryDelay:200});});
 const page=await app.firstWindow();await page.locator('#new').click();await page.locator('#source').waitFor({state:'visible'});
 const id=await page.evaluate(async()=>{const [project]=await store.list();return project.id;});
 await app.evaluate(({app})=>{
  const req=(process.getBuiltinModule('module') as any).createRequire(app.getAppPath()+'\\package.json');
  const {ProjectStore}=req('./build/src/project-store.cjs'),{AssistantCoordinator}=req('./build/src/assistant-coordinator.cjs'),{ChatService}=req('./build/src/chat.cjs');
  const originalWrite=ProjectStore.prototype.write,originalRun=AssistantCoordinator.prototype.run,state=globalThis as any;
  state.__raceWriteStarted=new Promise(resolve=>state.__notifyRaceWrite=resolve);state.__raceAssistantStarted=new Promise(resolve=>state.__notifyRaceAssistant=resolve);
  ProjectStore.prototype.write=async function(...args){if(state.__blockRaceWrite){state.__blockRaceWrite=false;state.__notifyRaceWrite();await new Promise(resolve=>state.__releaseRaceWrite=resolve);}return originalWrite.apply(this,args);};
  AssistantCoordinator.prototype.run=function(...args){const result=originalRun.apply(this,args);state.__notifyRaceAssistant();return result;};
  ChatService.prototype.send=async()=>({sessionId:null,messages:[]});state.__blockRaceWrite=true;
 });
 await page.evaluate(id=>{(window as any).__raceWrite=store.write(id,'main.tex','\\documentclass{article}\\begin{document}ok\\end{document}');},id);
 await app.evaluate(async()=>await (globalThis as any).__raceWriteStarted);
 await page.evaluate(id=>{(window as any).__raceBuild=store.compile(id).then(()=>false,()=>true);(window as any).__raceChat=store.chatSend(id,'test','default',[]);},id);
 await app.evaluate(async()=>await (globalThis as any).__raceAssistantStarted);await app.evaluate(()=>(globalThis as any).__releaseRaceWrite());
 const rejected=await page.evaluate(async()=>{await (window as any).__raceWrite;const blocked=await (window as any).__raceBuild;await (window as any).__raceChat;return blocked;});assert.equal(rejected,true);
});

test('keyboard text-entry focus remains visible in both themes and window sizes', {timeout:60000}, async t => {
 const data: string = await fs.mkdtemp(path.join(os.tmpdir(), 'refract-focus-'));
 const env: NodeJS.ProcessEnv = {...process.env, REFRACT_DATA_DIR:data, REFRACT_TEST_HIDDEN:'1'}; delete env.ELECTRON_RUN_AS_NODE;
 const app = await electron.launch({args:[root], env});
 t.after(async () => { await app.close().catch(() => {}); await fs.rm(data, {recursive:true,force:true,maxRetries:5,retryDelay:200}); });
 const page = await app.firstWindow();
 await page.locator('#new').click();
 await page.locator('#source').waitFor({state:'visible'});
 for (const colorScheme of ['light', 'dark'] as const) {
  await page.emulateMedia({colorScheme});
  for (const size of [{width:1440,height:900}, {width:900,height:650}]) {
   await page.setViewportSize(size);
   await page.locator('#source').focus();
   assert.equal(await page.locator('#source').evaluate(el => getComputedStyle(el).outlineStyle), 'solid');
   await page.locator('#chat-input').focus();
   assert.notEqual(await page.locator('.composer').evaluate(el => getComputedStyle(el).boxShadow), 'none');
  }
 }
});
