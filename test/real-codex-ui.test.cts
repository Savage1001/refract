const {test}=require('node:test'),assert=require('node:assert/strict');
const fs=require('node:fs/promises'),path=require('node:path'),os=require('node:os');
const {_electron:electron}=require('playwright');
function redPng():Buffer {
 const zlib=require('node:zlib'),width=64,height=64,header=Buffer.alloc(13);header.writeUInt32BE(width,0);header.writeUInt32BE(height,4);header[8]=8;header[9]=2;
 const pixels=Buffer.alloc((1+width*3)*height);for(let y=0;y<height;y++)for(let x=0;x<width;x++)pixels[y*(1+width*3)+1+x*3]=255;
 const chunk=(type:string,data:Buffer)=>{const label=Buffer.from(type),size=Buffer.alloc(4),checksum=Buffer.alloc(4);size.writeUInt32BE(data.length);let crc=0xffffffff;for(const byte of Buffer.concat([label,data])){crc^=byte;for(let i=0;i<8;i++)crc=(crc>>>1)^((crc&1)?0xedb88320:0);}checksum.writeUInt32BE((crc^0xffffffff)>>>0);return Buffer.concat([size,label,data,checksum]);};
 return Buffer.concat([Buffer.from('89504e470d0a1a0a','hex'),chunk('IHDR',header),chunk('IDAT',zlib.deflateSync(pixels)),chunk('IEND',Buffer.alloc(0))]);
}

test('the desktop app resumes a real Codex conversation after restart', {timeout:240000,skip:process.env.REFRACT_REAL_CODEX_TEST!=='1'},async t=>{
 const tempParent=await fs.realpath(os.tmpdir()),data=await fs.mkdtemp(path.join(tempParent,'refract-codex-ui-'));
 const env: NodeJS.ProcessEnv={...process.env,REFRACT_DATA_DIR:data,REFRACT_TEST_HIDDEN:'1'};delete env.ELECTRON_RUN_AS_NODE;
 const root=path.resolve(__dirname,'../..'),packaged=process.env.REFRACT_TEST_PACKAGED==='1';let app=null;
 const launch=()=>electron.launch(packaged ? {executablePath:path.join(root,'dist','win-unpacked','Refract.exe'),args:[],env} : {args:[root],env});
 t.after(async()=>{
  if(app)await app.close().catch(()=>{});
  const resolved=await fs.realpath(data);
  if(path.dirname(resolved).toLowerCase()!==tempParent.toLowerCase() || !path.basename(resolved).startsWith('refract-codex-ui-')) throw new Error(`Unsafe cleanup path: ${resolved}`);
  await fs.rm(resolved,{recursive:true,force:true,maxRetries:5,retryDelay:200});
 });
 app=await launch();let page=await app.firstWindow();
 await page.evaluate(() => store.codexAccountSelect('cli'));
 await page.locator('#new').click();await page.locator('#source').waitFor({state:'visible'});
 await page.locator('#chat-input').fill('Reply with the single word REFRACTFIRST. Do not change any files.');await page.locator('#send').click();
 await page.waitForFunction(()=>document.querySelector('#chat-messages')?.textContent?.includes('REFRACTFIRST') && !(document.querySelector('#send') as HTMLButtonElement).disabled,null,{timeout:90000});
 const projects=(await fs.readdir(path.join(data,'projects'))).filter((name:string)=>/^[0-9a-f-]{36}$/.test(name)),chatFile=path.join(data,'projects',projects[0],'.refract-chat.json');
 const first=JSON.parse(await fs.readFile(chatFile,'utf8'));assert.ok(first.sessionId);assert.equal(first.messages.at(-1).role,'assistant');
 await app.close();app=null;
 app=await launch();page=await app.firstWindow();await page.locator('#list article').first().click();await page.locator('#source').waitFor({state:'visible'});
 await page.locator('#chat-input').fill('Reply with the single word REFRACTSECOND. Do not change any files.');await page.locator('#send').click();
 await page.waitForFunction(()=>document.querySelector('#chat-messages')?.textContent?.includes('REFRACTSECOND') && !(document.querySelector('#send') as HTMLButtonElement).disabled,null,{timeout:90000});
 const second=JSON.parse(await fs.readFile(chatFile,'utf8'));assert.equal(second.sessionId,first.sessionId);assert.equal(second.messages.length,4);assert.equal(second.messages.at(-1).role,'assistant');
});

test('the desktop app sends mixed attachments to real Codex and retains them after restart', {timeout:180000,skip:process.env.REFRACT_REAL_CODEX_TEST!=='1'},async t=>{
 const tempParent: string=await fs.realpath(os.tmpdir()),data: string=await fs.mkdtemp(path.join(tempParent,'refract-codex-attachments-'));
 const env: NodeJS.ProcessEnv={...process.env,REFRACT_DATA_DIR:data,REFRACT_TEST_HIDDEN:'1'};delete env.ELECTRON_RUN_AS_NODE;
 const root: string=path.resolve(__dirname,'../..');let app=null;
 t.after(async()=>{if(app)await app.close().catch(()=>{});const resolved=await fs.realpath(data);if(path.dirname(resolved).toLowerCase()!==tempParent.toLowerCase() || !path.basename(resolved).startsWith('refract-codex-attachments-'))throw new Error(`Unsafe cleanup path: ${resolved}`);await fs.rm(resolved,{recursive:true,force:true,maxRetries:5,retryDelay:200});});
 const note=path.join(data,'note.txt'),image=path.join(data,'pixel.png');
 await fs.writeFile(note,'The identifier is REFRACTUITEXT.');
 await fs.writeFile(image,redPng());
 const launch=()=>electron.launch(process.env.REFRACT_TEST_PACKAGED==='1' ? {executablePath:path.join(root,'dist','win-unpacked','Refract.exe'),args:[],env} : {args:[root],env});app=await launch();let page=await app.firstWindow();
 await page.evaluate(() => store.codexAccountSelect('cli'));
 await page.locator('#new').click();await page.locator('#source').waitFor({state:'visible'});
 await app.evaluate(({dialog},files)=>{dialog.showOpenDialog=async()=>({canceled:false,filePaths:files});},[note,image]);
 await page.locator('#attach').click();await page.locator('#attachment-list .draft-attachment').nth(1).waitFor();
 await page.locator('#chat-input').fill('Read the attached text file and identify the color of the attached image. Reply only with the identifier and color. Do not modify files.');await page.locator('#send').click();
 await page.waitForFunction(()=>document.querySelector('#chat-messages')?.textContent?.includes('REFRACTUITEXT') && !(document.querySelector('#send') as HTMLButtonElement).disabled,null,{timeout:90000});
 const projects=(await fs.readdir(path.join(data,'projects'))).filter((name:string)=>/^[0-9a-f-]{36}$/.test(name)),chatFile=path.join(data,'projects',projects[0],'.refract-chat.json');
 const first=JSON.parse(await fs.readFile(chatFile,'utf8'));assert.equal(first.messages.at(-1).role,'assistant');assert.match(first.messages.at(-1).text,/red/i);assert.equal(first.messages[0].attachments.length,2);
 await app.close();app=null;app=await launch();page=await app.firstWindow();await page.locator('#list article').first().click();
 await page.locator('#chat-messages').getByText('note.txt',{exact:true}).waitFor();await page.locator('#chat-messages .attachment-preview').nth(1).waitFor();assert.match(await page.locator('#chat-messages').innerText(),/REFRACTUITEXT/);
});

test('real Codex cancellation preserves a partial project edit in version history', {timeout:150000,skip:process.env.REFRACT_REAL_CODEX_TEST!=='1'},async t=>{
 const tempParent: string=await fs.realpath(os.tmpdir()),data: string=await fs.mkdtemp(path.join(tempParent,'refract-codex-cancel-edit-'));
 const env: NodeJS.ProcessEnv={...process.env,REFRACT_DATA_DIR:data,REFRACT_TEST_HIDDEN:'1'};delete env.ELECTRON_RUN_AS_NODE;
 const root: string=path.resolve(__dirname,'../..');let app=null;
 t.after(async()=>{if(app){const page=await app.firstWindow().catch(()=>null);if(page){const stop=page.getByTitle('Stop assistant');if(await stop.isVisible().catch(()=>false))await stop.click().catch(()=>{});}await app.close().catch(()=>{});}const resolved=await fs.realpath(data);if(path.dirname(resolved).toLowerCase()!==tempParent.toLowerCase() || !path.basename(resolved).startsWith('refract-codex-cancel-edit-'))throw new Error(`Unsafe cleanup path: ${resolved}`);await fs.rm(resolved,{recursive:true,force:true,maxRetries:5,retryDelay:200});});
 app=await electron.launch({args:[root],env});const page=await app.firstWindow();await page.evaluate(()=>store.codexAccountSelect('cli'));await page.locator('#new').click();await page.locator('#source').waitFor({state:'visible'});
 const [id]=(await fs.readdir(path.join(data,'projects'))).filter((name:string)=>/^[0-9a-f-]{36}$/.test(name)),project=path.join(data,'projects',id),file=path.join(project,'main.tex');
 await page.locator('#chat-input').fill('Append the exact line "% REFRACT_CANCEL_PARTIAL" to main.tex, then run a 45 second wait before replying. Do not modify any other files.');await page.locator('#send').click();
 const deadline=Date.now()+90000;let changed=false;
 while(Date.now()<deadline){if((await fs.readFile(file,'utf8')).includes('% REFRACT_CANCEL_PARTIAL')){changed=true;break;}if(!(await page.locator('#send').isDisabled()))break;await new Promise(resolve=>setTimeout(resolve,200));}
 assert.ok(changed,'Codex did not write the requested marker while active');
 const stop=page.getByTitle('Stop assistant');assert.equal(await stop.isVisible(),true,'Codex finished before cancellation');await stop.click();
 await page.waitForFunction(()=>!(document.querySelector('#send') as HTMLButtonElement).disabled,null,{timeout:30000});
 const chat=JSON.parse(await fs.readFile(path.join(project,'.refract-chat.json'),'utf8'));assert.equal(chat.messages.at(-1).role,'error');assert.match(chat.messages.at(-1).text,/cancel/i);
 const versions=await page.evaluate(id=>store.versionList(id),id),Zip=require('adm-zip');
 assert.match(new Zip(path.join(project,'.refract-versions',versions[0].id+'.zip')).getEntry('main.tex').getData().toString('utf8'),/REFRACT_CANCEL_PARTIAL/);
});
