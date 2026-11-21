const {test} = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const os = require('node:os');
const path = require('node:path');
const {ProjectStore} = require('../src/project-store.cjs');
const {ChatService,terminateProcessTree,cancelProcess,codexArguments,CodexEventStream,CodexRunError} = require('../src/chat.cjs');
test('sessions are bound to the active project workspace', async t => {
 const root = await fs.mkdtemp(path.join(os.tmpdir(), 'refract-workspace-')); t.after(() => fs.rm(root,{recursive:true,force:true}));
 const store = new ProjectStore(root), project = await store.create('Paper');
 const calls: Array<{cwd:string; session:string|null; prompt:string}> = [];
 const service = new ChatService(store, async (cwd,session,prompt) => { calls.push({cwd,session,prompt}); return {sessionId:'new-session',text:'ok'}; });
 await service.save(project.id,{sessionId:'old-session',sessionWorkspace:'elsewhere',messages:[]});
 await service.send(project.id,'first'); await service.send(project.id,'second');
 assert.equal(calls[0].session,null); assert.equal(calls[1].session,'new-session');
 for (const call of calls) { assert.equal(call.cwd,store.projectDir(project.id)); assert.ok(call.prompt.includes(call.cwd)); }
});
test('an unverified compilation claim gets one correction in the same session', async t => {
 const root = await fs.mkdtemp(path.join(os.tmpdir(), 'refract-build-claim-'));
 t.after(() => fs.rm(root, {recursive:true,force:true}));
 const store = new ProjectStore(root), project = await store.create('Paper');
 for (const repair of [true, false]) {
  const calls: Array<{session: string | null; prompt: string}> = [];
  let verified = false;
  const service = new ChatService(store, async (_cwd, session, prompt) => {
   calls.push({session,prompt});
   verified = repair && calls.length === 2;
   return {sessionId:'build-session',text:repair ? 'I kept the source unchanged and built with XeLaTeX.' : 'I kept the source unchanged and built it with XeLaTeX.'};
  });
  const chat = await service.send(project.id, 'Build the paper.', 'default', [], undefined, undefined, {hasSuccessfulBuild: () => verified});
  assert.equal(calls.length, 2);
  assert.equal(calls[1].session, 'build-session');
  assert.match(calls[1].prompt, /call the Refract build tool now/);
  assert.match(chat.messages.at(-1).text, repair ? /built with XeLaTeX/ : /couldn’t verify/);
 }
});
test('switching Codex connections carries history into a new thread', async t => {
 const root = await fs.mkdtemp(path.join(os.tmpdir(), 'refract-chat-connection-'));
 t.after(() => fs.rm(root,{recursive:true,force:true}));
 const store = new ProjectStore(root), project = await store.create('Paper');
 const calls: {session: string | null; prompt: string}[] = [];
 const chats = new ChatService(store, async (_cwd,session,prompt) => {
  calls.push({session,prompt});
  return {sessionId:session ?? `thread-${calls.length}`,text:'reply'};
 });
 const account = { launch: {command:'unused',args:[],accountKey:'fixture',revision:1} };
 await chats.send(project.id,'CLI question');
 await chats.send(project.id,'Account question','default',[],undefined,undefined,account);
 await chats.send(project.id,'Account followup','default',[],undefined,undefined,account);
 await chats.send(project.id,'Back to CLI');
 assert.deepEqual(calls.map(call=>call.session),[null,null,'thread-2',null]);
 assert.match(calls[1].prompt,/CLI question/);
 assert.match(calls[3].prompt,/Account followup/);
});
test('chat and session survive a restart and are isolated by project', async t => {
 const root = await fs.mkdtemp(path.join(os.tmpdir(), 'refract-chat-')); t.after(() => fs.rm(root,{recursive:true,force:true}));
 const store = new ProjectStore(root); const p = await store.create('One'), q = await store.create('Two');
 let seen; const run = async (cwd, id, prompt) => { seen = id; return {sessionId:'thread-1',text:'reply '+prompt}; };
 await new ChatService(store,run).send(p.id,'hello');
 const service = new ChatService(store,run); await service.send(p.id,'again');
 assert.equal(seen,'thread-1'); assert.equal((await service.load(p.id)).messages.length,4); assert.equal((await service.load(q.id)).messages.length,0);
});

test('multiple chats remain distinct when reopened', async t => {
 const root = await fs.mkdtemp(path.join(os.tmpdir(), 'refract-multichat-')); t.after(() => fs.rm(root,{recursive:true,force:true}));
 const store = new ProjectStore(root), p = await store.create('Paper');
 const service = new ChatService(store, async (_cwd,id,prompt) => ({sessionId:id || prompt,text:prompt}));
 await service.send(p.id, 'first'); const second = await service.create(p.id);
 await service.send(p.id, 'second', second);
 assert.equal((await service.list(p.id)).length, 2);
 assert.equal((await service.load(p.id)).messages[0].text, 'first');
 assert.equal((await service.load(p.id,second)).messages[0].text, 'second');
});

test('chat names persist and reject invalid values without losing messages', async t => {
 const root = await fs.mkdtemp(path.join(os.tmpdir(), 'refract-rename-chat-')); t.after(() => fs.rm(root,{recursive:true,force:true}));
 const store = new ProjectStore(root), p = await store.create('Paper');
 const service = new ChatService(store, async () => ({sessionId:'session',text:'reply'}));
 await service.send(p.id,'hello'); const id=await service.create(p.id);
 await service.rename(p.id,'default','  Main  '); await service.rename(p.id,id,'Research');
 await assert.rejects(service.rename(p.id,id,'  '),/Chat name/);
 await assert.rejects(service.rename(p.id,id,'x'.repeat(101)),/Chat name/);
 const reopened=new ChatService(store); const chats=await reopened.list(p.id);
 assert.equal(chats.find(chat=>chat.id==='default').title,'Main');
 assert.equal(chats.find(chat=>chat.id===id).title,'Research');
 assert.equal((await reopened.load(p.id)).messages[0].text,'hello');
});

test('chat images are stored and passed to the assistant', async t => {
 const root = await fs.mkdtemp(path.join(os.tmpdir(), 'refract-images-')); t.after(() => fs.rm(root,{recursive:true,force:true}));
 const store = new ProjectStore(path.join(root,'projects')), project = await store.create('Paper');
 const image = path.join(root,'figure.png'); await fs.writeFile(image,Buffer.from([1,2,3]));
 let passed; const service = new ChatService(store, async (_cwd,_id,_prompt,images) => { passed = images; return {sessionId:'one',text:'ok'}; });
 const attachments = await service.attach(project.id,[image]);
 await service.send(project.id,'Describe this','default',attachments);
 assert.deepEqual(await fs.readFile(passed[0]),Buffer.from([1,2,3]));
 assert.equal((await service.load(project.id)).messages[0].attachments[0].name,'figure.png');
 await assert.rejects(service.send(project.id,'bad','default',[{token:'../x.png',name:'x'}]),/Invalid attachment/);
});

test('pasted images are validated, stored, and passed to the assistant', async t => {
 const root=await fs.mkdtemp(path.join(os.tmpdir(),'refract-paste-'));t.after(()=>fs.rm(root,{recursive:true,force:true}));
 const store=new ProjectStore(root),project=await store.create('Paper');let passed;
 const service=new ChatService(store,async (_cwd,_id,_prompt,images)=>{passed=images;return {sessionId:'one',text:'ok'};});
 const bytes=Buffer.from([137,80,78,71,13,10,26,10,1]);
 await assert.rejects(service.attachPastedImage(project.id,Buffer.from('invalid')),/Unsupported image format/);
 await assert.rejects(service.attachPastedImage(project.id,new Uint8Array(20*1024*1024+1)),/too large/);
 const attachment=await service.attachPastedImage(project.id,bytes);
 await service.send(project.id,'Describe this','default',[attachment]);
 assert.deepEqual(await fs.readFile(passed[0]),bytes);
 assert.equal((await service.load(project.id)).messages[0].attachments[0].name,'Pasted screenshot.png');
});

test('document attachments are available to the assistant and preserved in history', async t => {
 const root = await fs.mkdtemp(path.join(os.tmpdir(), 'refract-docs-')); t.after(() => fs.rm(root,{recursive:true,force:true}));
 const store = new ProjectStore(path.join(root,'projects')), project = await store.create('Paper');
 const doc = path.join(root,'notes.txt'); await fs.writeFile(doc,'Important facts');
 let received; const service = new ChatService(store, async (cwd,_id,prompt,images) => { received = {cwd,prompt,images}; return {sessionId:'one',text:'ok'}; });
 const attachments = await service.attach(project.id,[doc]); await service.send(project.id,'Read this','default',attachments);
 assert.match(received.prompt,/Attached files in the project workspace/); assert.deepEqual(received.images,[]);
 assert.equal(await fs.readFile(path.join(received.cwd,'.refract-attachments',attachments[0].token),'utf8'),'Important facts');
 assert.equal((await service.load(project.id)).messages[0].attachments[0].name,'notes.txt');
});

test('attachment requests include bounded UTF-8 previews only for selected text files',async t=>{
 const root=await fs.mkdtemp(path.join(os.tmpdir(),'refract-text-preview-'));t.after(()=>fs.rm(root,{recursive:true,force:true}));
 const store=new ProjectStore(path.join(root,'projects')),project=await store.create('Preview');
 const selected=path.join(root,'selected.txt'),large=path.join(root,'large.md'),second=path.join(root,'second.tex'),last=path.join(root,'last.csv'),invalid=path.join(root,'invalid.txt'),pdf=path.join(root,'paper.pdf'),unselected=path.join(root,'unselected.txt');
 await fs.writeFile(selected,'PREVIEW_SELECTED_α');await fs.writeFile(large,'x'.repeat(40000)+'BEYOND_PREVIEW');
 await fs.writeFile(invalid,Buffer.from([0xc3,0x28]));await fs.writeFile(pdf,'%PDF-DO_NOT_INLINE');await fs.writeFile(unselected,'UNSELECTED_SECRET');
 await fs.writeFile(second,'PREVIEW_TWO'+'y'.repeat(20000));await fs.writeFile(last,'PREVIEW_LAST'+'z'.repeat(20000));
 for(const provider of ['codex','opencode']) {
  let prompt='';const runner=async(_cwd,_session,value)=>{prompt=value;return {sessionId:'preview',text:'done'};};
  const service=new ChatService(store,runner,runner),attachments=await service.attach(project.id,[selected,large,invalid,pdf,second,last]);
  await service.send(project.id,'Read selected files','default',attachments,undefined,undefined,undefined,provider);
  assert.match(prompt,/PREVIEW_SELECTED_α/);assert.match(prompt,/"truncated":true/);assert.match(prompt,/only a prefix/);
  assert.match(prompt,/PREVIEW_TWO/);assert.doesNotMatch(prompt,/BEYOND_PREVIEW|UNSELECTED_SECRET|PDF-DO_NOT_INLINE|PREVIEW_LAST/);
  assert.ok(prompt.length<40000);for(const item of attachments)assert.ok(prompt.includes(item.token));
  assert.doesNotMatch(JSON.stringify(await service.load(project.id)),/PREVIEW_SELECTED_α/);
 }
});

test('chat storage rejects linked private directories and project roots',async t=>{
 const tempParent=await fs.realpath(os.tmpdir()),root=await fs.mkdtemp(path.join(tempParent,'refract-chat-links-'));
 t.after(async()=>{const resolved=await fs.realpath(root);if(path.dirname(resolved).toLowerCase()!==tempParent.toLowerCase() || !path.basename(resolved).startsWith('refract-chat-links-'))throw new Error('Unsafe cleanup path');await fs.rm(resolved,{recursive:true,force:true,maxRetries:5,retryDelay:200});});
 const store=new ProjectStore(path.join(root,'projects')),project=await store.create('Paper'),outside=path.join(root,'outside');
 await fs.mkdir(outside);await fs.writeFile(path.join(outside,'sentinel.txt'),'unchanged');
 const attachDir=path.join(store.projectDir(project.id),'.refract-attachments'),chatDir=path.join(store.projectDir(project.id),'.refract-chats');
 try {await fs.symlink(outside,attachDir,process.platform==='win32'?'junction':'dir');await fs.symlink(outside,chatDir,process.platform==='win32'?'junction':'dir');}
 catch(error){if(['EPERM','EACCES'].includes(error.code))return t.skip('Creating directory links is not permitted');throw error;}
 let ran=false;const service=new ChatService(store,async()=>{ran=true;return {sessionId:null,text:'unexpected'};});
 const token='aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa.txt';
 await assert.rejects(service.attach(project.id,[path.join(outside,'sentinel.txt')]),/Invalid private project directory/);
 await assert.rejects(service.send(project.id,'Read this','default',[{token,name:'note.txt'}]),/Invalid private project directory/);
 await assert.rejects(service.create(project.id),/Invalid private project directory/);
 await assert.rejects(service.list(project.id),/Invalid private project directory/);
 await assert.rejects(service.load(project.id,'bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb'),/Invalid private project directory/);
 assert.equal(ran,false);assert.equal(await fs.readFile(path.join(outside,'sentinel.txt'),'utf8'),'unchanged');
 const linkedId='cccccccc-cccc-cccc-cccc-cccccccccccc';await fs.symlink(outside,store.projectDir(linkedId),process.platform==='win32'?'junction':'dir');
 await assert.rejects(service.load(linkedId),/Invalid project directory/);await assert.rejects(service.list(linkedId),/Invalid project directory/);
});

test('chat files and attachments cannot be links',async t=>{
 const tempParent=await fs.realpath(os.tmpdir()),root=await fs.mkdtemp(path.join(tempParent,'refract-chat-file-links-'));
 t.after(async()=>{const resolved=await fs.realpath(root);if(path.dirname(resolved).toLowerCase()!==tempParent.toLowerCase() || !path.basename(resolved).startsWith('refract-chat-file-links-'))throw new Error('Unsafe cleanup path');await fs.rm(resolved,{recursive:true,force:true,maxRetries:5,retryDelay:200});});
 const store=new ProjectStore(path.join(root,'projects')),project=await store.create('Paper'),outside=path.join(root,'outside.txt');
 await fs.writeFile(outside,'sentinel');const chatFile=path.join(store.projectDir(project.id),'.refract-chat.json');
 try {await fs.symlink(outside,chatFile,'file');}catch(error){if(['EPERM','EACCES'].includes(error.code))return t.skip('Creating file links is not permitted');throw error;}
 const service=new ChatService(store,async()=>({sessionId:null,text:'unexpected'}));
 await assert.rejects(service.load(project.id),/Only regular private project files/);
 await assert.rejects(service.save(project.id,{sessionId:null,messages:[]}),/Only regular private project files/);
 await fs.unlink(chatFile);const dir=path.join(store.projectDir(project.id),'.refract-attachments');await fs.mkdir(dir);
 const token='dddddddd-dddd-dddd-dddd-dddddddddddd.txt';await fs.symlink(outside,path.join(dir,token),'file');
 await assert.rejects(service.send(project.id,'Read this','default',[{token,name:'note.txt'}]),/Only regular private project files/);
 assert.equal(await fs.readFile(outside,'utf8'),'sentinel');
});

test('chat files and attachments cannot be hard links',async t=>{
 const tempParent=await fs.realpath(os.tmpdir()),root=await fs.mkdtemp(path.join(tempParent,'refract-chat-hardlinks-'));
 t.after(async()=>{const resolved=await fs.realpath(root);if(path.dirname(resolved).toLowerCase()!==tempParent.toLowerCase() || !path.basename(resolved).startsWith('refract-chat-hardlinks-'))throw new Error('Unsafe cleanup path');await fs.rm(resolved,{recursive:true,force:true,maxRetries:5,retryDelay:200});});
 const store=new ProjectStore(path.join(root,'projects')),project=await store.create('Paper'),outside=path.join(root,'outside.txt');
 await fs.writeFile(outside,'sentinel');const chatFile=path.join(store.projectDir(project.id),'.refract-chat.json');
 try{await fs.link(outside,chatFile);}catch(error){if(['EPERM','EACCES','ENOTSUP'].includes(error.code))return t.skip('Creating hard links is not permitted');throw error;}
 const service=new ChatService(store,async()=>({sessionId:null,text:'unexpected'}));
 await assert.rejects(service.load(project.id),/Only regular private project files/);
 await assert.rejects(service.save(project.id,{sessionId:null,messages:[]}),/Only regular private project files/);
 await fs.unlink(chatFile);const dir=path.join(store.projectDir(project.id),'.refract-attachments');await fs.mkdir(dir);
 const token='eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee.txt';await fs.link(outside,path.join(dir,token));
 await assert.rejects(service.send(project.id,'Read this','default',[{token,name:'note.txt'}]),/Only regular private project files/);
 assert.equal(await fs.readFile(outside,'utf8'),'sentinel');
});

test('chat storage rejects non-file entries and preserves data when saving fails',async t=>{
 const tempParent=await fs.realpath(os.tmpdir()),root=await fs.mkdtemp(path.join(tempParent,'refract-chat-invalid-'));
 t.after(async()=>{const resolved=await fs.realpath(root);if(path.dirname(resolved).toLowerCase()!==tempParent.toLowerCase() || !path.basename(resolved).startsWith('refract-chat-invalid-'))throw new Error('Unsafe cleanup path');await fs.rm(resolved,{recursive:true,force:true,maxRetries:5,retryDelay:200});});
 const store=new ProjectStore(path.join(root,'projects')),project=await store.create('Paper'),service=new ChatService(store),projectDir=store.projectDir(project.id);
 const chatFile=path.join(projectDir,'.refract-chat.json');await fs.mkdir(chatFile);
 await assert.rejects(service.load(project.id),/Only regular private project files/);await assert.rejects(service.save(project.id,{}),/Only regular private project files/);
 await fs.rmdir(chatFile);await service.save(project.id,{sessionId:'previous',messages:[]});
 const rename=fs.rename;fs.rename=async()=>{throw new Error('simulated save failure');};
 try{await assert.rejects(service.save(project.id,{sessionId:'new',messages:[]}),/simulated save failure/);}finally{fs.rename=rename;}
 assert.equal((await service.load(project.id)).sessionId,'previous');
 assert.equal((await fs.readdir(projectDir)).some(name=>name.endsWith('.tmp')),false);
 const dir=path.join(projectDir,'.refract-attachments'),token='eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee.txt';await fs.mkdir(dir);await fs.mkdir(path.join(dir,token));
 await assert.rejects(service.send(project.id,'Read this','default',[{token,name:'note.txt'}]),/Only regular private project files/);
});

test('failed multi-file attachment removes only copies from that attempt',async t=>{
 const tempParent=await fs.realpath(os.tmpdir()),root=await fs.mkdtemp(path.join(tempParent,'refract-attach-fail-'));
 t.after(async()=>{const resolved=await fs.realpath(root);if(path.dirname(resolved).toLowerCase()!==tempParent.toLowerCase() || !path.basename(resolved).startsWith('refract-attach-fail-'))throw new Error('Unsafe cleanup path');await fs.rm(resolved,{recursive:true,force:true,maxRetries:5,retryDelay:200});});
 const store=new ProjectStore(path.join(root,'projects')),project=await store.create('Paper'),service=new ChatService(store);
 const first=path.join(root,'first.txt'),second=path.join(root,'second.txt'),bad=path.join(root,'bad.exe');
 await fs.writeFile(first,'first');await fs.writeFile(second,'second');await fs.writeFile(bad,'bad');
 const dir=path.join(store.projectDir(project.id),'.refract-attachments');
 await assert.rejects(service.attach(project.id,Array(11).fill(first)),/at most 10/);await assert.rejects(service.attach(project.id,[first,second],1),/at most 1/);await assert.rejects(fs.access(dir));
 await assert.rejects(service.attach(project.id,[first,bad]),/Unsupported file format/);await assert.rejects(fs.access(dir));
 await fs.mkdir(dir);await fs.writeFile(path.join(dir,'existing.txt'),'keep');
 const copy=service.copyAttachment.bind(service);let calls=0;service.copyAttachment=async (...args)=>{await copy(...args);if(++calls===2)throw new Error('simulated copy failure');};
 try{await assert.rejects(service.attach(project.id,[first,second]),/simulated copy failure/);}finally{service.copyAttachment=copy;}
 assert.deepEqual(await fs.readdir(dir),['existing.txt']);assert.equal(await fs.readFile(path.join(dir,'existing.txt'),'utf8'),'keep');
 assert.equal(await fs.readFile(first,'utf8'),'first');assert.equal(await fs.readFile(second,'utf8'),'second');
 const originalOpen=fs.open;let outputs=0,partial=false;
 fs.open=async (file,flags,...args)=>{const handle=await originalOpen(file,flags,...args);if(flags==='wx' && path.dirname(file)===dir && ++outputs===2){const write=handle.write.bind(handle);handle.write=async (buffer,offset,length,position)=>{if(partial)throw new Error('simulated disk failure');partial=true;return write(buffer,offset,Math.min(3,length),position);};}return handle;};
 try{await assert.rejects(service.attach(project.id,[first,second]),/simulated disk failure/);}finally{fs.open=originalOpen;}
 assert.equal(partial,true);assert.deepEqual(await fs.readdir(dir),['existing.txt']);assert.equal(await fs.readFile(first,'utf8'),'first');assert.equal(await fs.readFile(second,'utf8'),'second');
 const open=fs.open;let expanded=false,maxWritten=0;
 fs.open=async (file,flags,...args)=>{const handle=await open(file,flags,...args);if(file===second && flags==='r'){const read=handle.read.bind(handle);handle.read=async (...readArgs)=>{const result=await read(...readArgs);if(!expanded && result.bytesRead){expanded=true;await fs.truncate(second,20*1024*1024+1);}return result;};}if(flags==='wx' && path.dirname(file)===dir){const write=handle.write.bind(handle);let total=0;handle.write=async (...writeArgs)=>{const result=await write(...writeArgs);total+=result.bytesWritten;maxWritten=Math.max(maxWritten,total);return result;};}return handle;};
 try{await assert.rejects(service.attach(project.id,[first,second]),/File is too large/);}finally{fs.open=open;}
 assert.equal(expanded,true);assert.ok(maxWritten<=20*1024*1024);assert.deepEqual(await fs.readdir(dir),['existing.txt']);assert.equal(await fs.readFile(path.join(dir,'existing.txt'),'utf8'),'keep');
 const input=await fs.open(second,'r');const prefix=Buffer.alloc(6);try{await input.read(prefix,0,6,0);}finally{await input.close();}assert.equal(prefix.toString(),'second');
 await fs.truncate(second,20*1024*1024);const [atLimit]=await service.attach(project.id,[second]);assert.equal((await fs.stat(path.join(dir,atLimit.token))).size,20*1024*1024);
});

test('cancelled assistant responses are saved in conversation history',async t=>{
 const root=await fs.mkdtemp(path.join(os.tmpdir(),'refract-chat-cancel-'));t.after(()=>fs.rm(root,{recursive:true,force:true}));
 const store=new ProjectStore(root),project=await store.create('Paper'),controller=new AbortController();
 let notify;const started=new Promise<void>(resolve=>notify=resolve);
 const service=new ChatService(store,async (_cwd,_id,_prompt,_images,signal)=>{
  notify();await new Promise<void>(resolve=>signal.addEventListener('abort',()=>resolve(),{once:true}));
  throw new Error('Assistant operation cancelled.');
 });
 const pending=service.send(project.id,'stop please','default',[],controller.signal);await started;controller.abort();await pending;
 const chat=await new ChatService(store).load(project.id);assert.deepEqual(chat.messages.map(m=>m.role),['user','error']);
 assert.match(chat.messages[1].text,/cancelled/);
});

test('a session started before cancellation is retained and resumed',async t=>{
 const root=await fs.mkdtemp(path.join(os.tmpdir(),'refract-chat-session-'));t.after(()=>fs.rm(root,{recursive:true,force:true}));
 const store=new ProjectStore(root),project=await store.create('Paper');
 const stopped=new ChatService(store,async()=>{throw new CodexRunError('Assistant operation cancelled.','thread-from-interrupted-run');});
 const first=await stopped.send(project.id,'First');assert.equal(first.sessionId,'thread-from-interrupted-run');
 let resumed;const service=new ChatService(store,async (_cwd,id)=>{resumed=id;return {sessionId:id,text:'Resumed'};});
 const second=await service.send(project.id,'Continue');assert.equal(resumed,'thread-from-interrupted-run');
 assert.deepEqual(second.messages.map(m=>m.role),['user','error','user','assistant']);
});

test('cancellation terminates a background process', {timeout:15000}, async()=>{
 const {spawn}=require('node:child_process'),{once}=require('node:events');
 const child=spawn(process.execPath,['-e','setInterval(() => {}, 1000)'],{windowsHide:true,stdio:'ignore'});
 const closed=once(child,'close');await once(child,'spawn');
 assert.equal(await terminateProcessTree(child),true);await closed;
});

test('Windows cancellation terminates a descendant process', {timeout:15000, skip:process.platform!=='win32'}, async t=>{
 const {spawn,spawnSync}=require('node:child_process'),{once}=require('node:events');
 const child=spawn(process.execPath,[path.join(__dirname,'fixtures','process-tree-child.cjs')],{windowsHide:true,stdio:['ignore','pipe','ignore']});
 let descendantPid: number | undefined;
 t.after(()=>{
  if(descendantPid) spawnSync('taskkill.exe',['/pid',String(descendantPid),'/t','/f'],{windowsHide:true,stdio:'ignore'});
  if(child.exitCode===null) child.kill();
 });
 const closed=once(child,'close');
 const [data]=await once(child.stdout,'data');descendantPid=Number(String(data).trim());
 assert.ok(Number.isInteger(descendantPid) && descendantPid>0);
 assert.equal(await terminateProcessTree(child),true);await closed;
 assert.throws(()=>process.kill(descendantPid,0),{code:'ESRCH'});
 descendantPid=undefined;
});

test('cancellation falls back to terminating the direct child if tree termination fails', {timeout:15000}, async()=>{
 const {spawn}=require('node:child_process'),{once}=require('node:events');
 const child=spawn(process.execPath,['-e','setInterval(() => {}, 1000)'],{windowsHide:true,stdio:'ignore'});
 const closed=once(child,'close');await once(child,'spawn');
 assert.equal(await cancelProcess(child,async()=>false),true);await closed;
 assert.equal(await cancelProcess(child,async()=>false),false);
 const second=spawn(process.execPath,['-e','setInterval(() => {}, 1000)'],{windowsHide:true,stdio:'ignore'}),secondClosed=once(second,'close');await once(second,'spawn');
 assert.equal(await cancelProcess(second,async()=>{throw new Error('tree termination unavailable');}),true);await secondClosed;
});

test('new and resumed Codex sessions both request the project sandbox',()=>{
 const fresh=codexArguments('codex.js',null,['image.png']),resumed=codexArguments('codex.js','session-1',['image.png']);
 assert.deepEqual(fresh.slice(0,4),['codex.js','exec','--sandbox','workspace-write']);
 assert.deepEqual(resumed.slice(0,7),['codex.js','exec','--sandbox','workspace-write','--config','allow_login_shell=false','resume']);
 assert.deepEqual(resumed.slice(-4),['--image','image.png','session-1','-']);
});

test('Codex events are decoded across chunks, bound memory and report safe progress',()=>{
 const statuses=[],events=new CodexEventStream(null,status=>statuses.push(status));
 const data=Buffer.from([
  JSON.stringify({type:'thread.started',thread_id:'session-1'}),
  JSON.stringify({type:'turn.started'}),
  JSON.stringify({type:'item.started',item:{type:'command_execution',command:'private command'}}),
  JSON.stringify({type:'item.completed',item:{type:'command_execution'}}),
  JSON.stringify({type:'item.completed',item:{type:'agent_message',text:'Hello 🌍'}})
 ].join('\n'));
 for(let i=0;i<data.length;i+=7)events.write(data.subarray(i,i+7));events.end();
 assert.equal(events.thread,'session-1');assert.equal(events.message,'Hello 🌍');
 assert.deepEqual(statuses,['Thinking…','Running a command…','Thinking…']);
 const bounded=new CodexEventStream(null);bounded.write(Buffer.alloc(5*1024*1024,65));
 assert.equal(bounded.pending.length,0);assert.equal(bounded.oversized,true);
 bounded.write(Buffer.from('\n'+JSON.stringify({type:'item.completed',item:{type:'agent_message',text:'Recovered'}})+'\n'));bounded.end();
 assert.equal(bounded.message,'Recovered');
});

test('a started session is persisted while the assistant is still running', async t => {
 const root: string=await fs.mkdtemp(path.join(os.tmpdir(),'refract-live-session-'));t.after(()=>fs.rm(root,{recursive:true,force:true}));
 const store=new ProjectStore(root),project=await store.create('Paper');
 let release: () => void, persisted: () => void;
 const proceed=new Promise<void>(resolve=>release=resolve), saved=new Promise<void>(resolve=>persisted=resolve);
 const service=new ChatService(store,async (_cwd,_id,_prompt,_images,_signal,_progress,onSession: (id:string)=>void)=>{onSession('session-live');await proceed;return {sessionId:'session-live',text:'done'};});
 const save=service.save.bind(service);
 service.save=async (...args: any[])=>{await save(...args);if(args[1].sessionId==='session-live')persisted();};
 const running=service.send(project.id,'Start');await saved;
 const intermediate=await new ChatService(store).load(project.id);assert.equal(intermediate.sessionId,'session-live');assert.deepEqual(intermediate.messages.map(m=>m.role),['user']);
 release();const result=await running;assert.equal(result.messages.at(-1).text,'done');
});

test('session ID survives abrupt process termination and can be resumed', {timeout:15000}, async t => {
 const root: string=await fs.mkdtemp(path.join(os.tmpdir(),'refract-chat-crash-'));t.after(()=>fs.rm(root,{recursive:true,force:true}));
 const store=new ProjectStore(root),project=await store.create('Paper');
 const {spawn}=require('node:child_process'),{once}=require('node:events');
 const child=spawn(process.execPath,[path.join(__dirname,'fixtures','chat-child.cjs'),root,project.id],{windowsHide:true,stdio:['ignore','pipe','pipe']});
 t.after(()=>{if(child.exitCode===null && child.signalCode===null)child.kill();});
 const [data]: [Buffer]=await once(child.stdout,'data');assert.match(data.toString(),/saved/);
 const closed=once(child,'close');child.kill();await closed;
 let resumed: string|null=null;
 const service=new ChatService(store,async (_cwd,session)=>{resumed=session;return {sessionId:session,text:'resumed'};});
 const chat=await service.send(project.id,'Continue');
 assert.equal(resumed,'session-before-exit');assert.equal(chat.messages.at(-1).text,'resumed');
});

test('Codex overrides apply to new and resumed runs and reject invalid values', () => {
 const options = {model:'gpt-5.4',reasoningEffort:'high'};
 for (const session of [null,'session-1']) {
  const args = codexArguments('codex.js',session,[],options);
  assert.deepEqual(args.slice(4,7),['--model=gpt-5.4','--config','model_reasoning_effort="high"']);
  assert.equal(args.includes('resume'),session !== null);
 }
 assert.throws(()=>codexArguments('codex.js',null,[],{model:' bad '}),/Invalid Codex model/);
 assert.throws(()=>codexArguments('codex.js',null,[],{reasoningEffort:'high"\nfoo=1'}),/Invalid Codex reasoning effort/);
 for (const reasoningEffort of ['max', 'ultra'] as const) assert.ok(codexArguments('codex.js',null,[],{reasoningEffort}).includes(`model_reasoning_effort="${reasoningEffort}"`));
});

test('chat forwards selected options for new and resumed sessions', async t => {
 const root = await fs.mkdtemp(path.join(os.tmpdir(), 'refract-options-')); t.after(() => fs.rm(root,{recursive:true,force:true}));
 const store = new ProjectStore(root), project = await store.create('Options'), seen: unknown[] = [];
 const service = new ChatService(store, async (_cwd, session, _prompt, _images, _signal, _progress, _onSession, options) => { seen.push({session,options}); return {sessionId:'thread',text:'ok'}; });
 const options = {model:'gpt-5.4',reasoningEffort:'high' as const};
 await service.send(project.id,'first','default',[],undefined,undefined,options);
 await service.send(project.id,'second','default',[],undefined,undefined,options);
 assert.deepEqual(seen,[{session:null,options},{session:'thread',options}]);
 await assert.rejects(service.send(project.id,'invalid','default',[],undefined,undefined,{model:'bad\nmodel'}),/Invalid Codex model/);
 assert.equal((await service.load(project.id)).messages.length,4);
});

test('provider switching carries context and retries failed handoffs', async t => {
 const root = await fs.mkdtemp(path.join(os.tmpdir(), 'refract-provider-')); t.after(() => fs.rm(root,{recursive:true,force:true}));
 const store = new ProjectStore(root), project = await store.create('Paper');
 const calls: Array<{session:string|null; prompt:string}> = [];
 const codex = async (_cwd, session, prompt) => {calls.push({session,prompt}); return {sessionId:'codex-session',text:'codex reply'};};
 let fail = true;
 const opencode = async (_cwd, session, prompt) => {calls.push({session,prompt}); if(fail) {fail=false; throw new Error('failed');} return {sessionId:'open-session',text:'open reply'};};
 const service = new ChatService(store,codex,opencode);
 await service.send(project.id,'first','default',[],undefined,undefined,undefined,'codex');
 await service.send(project.id,'second','default',[],undefined,undefined,undefined,'opencode');
 await service.send(project.id,'retry','default',[],undefined,undefined,undefined,'opencode');
 assert.deepEqual((await service.load(project.id)).messages.find(m=>m.role==='handoff'),{role:'handoff',from:'codex',to:'opencode',status:'failed'});
 assert.equal(calls[1].session,null); assert.match(calls[1].prompt,/user: first/); assert.match(calls[1].prompt,/assistant: codex reply/);
 assert.equal(calls[2].session,null); assert.match(calls[2].prompt,/user: second/); assert.doesNotMatch(calls[2].prompt,/error: failed/);
 await service.send(project.id,'third','default',[],undefined,undefined,undefined,'opencode'); assert.equal(calls[3].session,'open-session'); assert.ok(calls[3].prompt.endsWith('\n\nthird'));
 await service.send(project.id,'back','default',[],undefined,undefined,undefined,'codex'); assert.equal(calls[4].session,null); assert.match(calls[4].prompt,/assistant: open reply/);
 const fresh = await service.create(project.id); await service.send(project.id,'new',fresh,[],undefined,undefined,undefined,'codex');
 assert.equal(calls[5].session,null); assert.ok(calls[5].prompt.endsWith('\n\nnew'));
 const legacy = await service.create(project.id); await service.save(project.id,{sessionId:'old-session',messages:[]},legacy);
 assert.equal((await service.load(project.id,legacy)).provider,'codex');
});

test('handoffs in both directions preserve conversation history beyond 48000 characters', async t => {
 const root = await fs.mkdtemp(path.join(os.tmpdir(), 'refract-history-')); t.after(() => fs.rm(root,{recursive:true,force:true}));
 const store = new ProjectStore(root), project = await store.create('Paper');
 for (const [source, target] of [['codex','opencode'], ['opencode','codex']] as const) {
  const received: string[] = [];
  const runner = async (_cwd, _session, prompt) => {received.push(prompt); return {sessionId:'session',text:'x'.repeat(50000)};};
  const service = new ChatService(store, runner, runner), chatId = await service.create(project.id);
  await service.send(project.id,'first',chatId,[],undefined,undefined,undefined,source);
  await service.send(project.id,'next',chatId,[],undefined,undefined,undefined,target);
  assert.ok(received[1].includes(`user: first\n\nassistant: ${'x'.repeat(50000)}`));
 }
});

test('OpenCode chat persists its provider and resumes its own session', async t => {
 const root = await fs.mkdtemp(path.join(os.tmpdir(), 'refract-opencode-chat-')); t.after(() => fs.rm(root,{recursive:true,force:true}));
 const store = new ProjectStore(root), project = await store.create('Paper'), sessions:Array<string|null>=[], selectedModels:string[]=[];
 const codex = async()=>{throw new Error('Codex must not run');};
 const opencode = async (_cwd, session, _prompt, _images, _signal, _progress, _onSession, options) => {sessions.push(session);selectedModels.push(options.model);return {sessionId:'ses_1',text:'ok'};};
 await new ChatService(store,codex,opencode).send(project.id,'first','default',[],undefined,undefined,{model:'opencode/model-10'},'opencode');
 const restarted=new ChatService(store,codex,opencode);
 await restarted.send(project.id,'second','default',[],undefined,undefined,{model:'opencode/model-2'},'opencode');
 assert.deepEqual(sessions,[null,'ses_1']); assert.deepEqual(selectedModels,['opencode/model-10','opencode/model-2']);
 const chat=await restarted.load(project.id);assert.equal(chat.provider,'opencode');assert.equal(chat.messages.length,4);
});

test('image previews use only validated private attachments', async t => {
 const root=await fs.mkdtemp(path.join(os.tmpdir(),'refract-preview-')); t.after(()=>fs.rm(root,{recursive:true,force:true}));
 const store=new ProjectStore(root), project=await store.create('Preview'), service=new ChatService(store);
 const bytes=Buffer.from('89504e470d0a1a0a0000000d49484452000000010000000108060000001f15c4890000000d4944415408996360000000020001e221bc330000000049454e44ae426082','hex');
 const item=await service.attachPastedImage(project.id,bytes);
 assert.equal(await service.imagePreview(project.id,item.token),`data:image/png;base64,${bytes.toString('base64')}`);
 await assert.rejects(service.imagePreview(project.id,'../../secret.png'),/Invalid/);
 const regularFile = service.regularFile.bind(service);
 service.regularFile = async (file:string) => {
  const info = await regularFile(file);
  await fs.truncate(file,20*1024*1024+1);
  return info;
 };
 await assert.rejects(service.imagePreview(project.id,item.token),/Image is too large/);
});

test('mixed supported attachments retain bytes and routing across a service restart', async t=>{
 const root=await fs.mkdtemp(path.join(os.tmpdir(),'refract-attachment-formats-'));t.after(()=>fs.rm(root,{recursive:true,force:true}));
 const store=new ProjectStore(path.join(root,'projects')),project=await store.create('Formats');
 const extensions=['png','jpg','jpeg','webp','pdf','txt','md','tex','bib','csv','docx'];
 const files:string[]=[];
 for(const extension of extensions){const file=path.join(root,`sample.${extension.toUpperCase()}`);await fs.writeFile(file,`fixture-${extension}`);files.push(file);}
 let received:{cwd:string;prompt:string;images:string[]}|undefined;
 const runner=async (cwd:string,_id:string|null,prompt:string,images:string[])=>{received={cwd,prompt,images};return {sessionId:'formats',text:'ok'};};
 const service=new ChatService(store,runner);
 const attachments=[...await service.attach(project.id,files.slice(0,10)),...await service.attach(project.id,files.slice(10))];
 // Send the ten-file maximum, then the remaining document in a resumed service.
 await service.send(project.id,'Inspect these','default',attachments.slice(0,10));
 assert.ok(received);assert.equal(received.images.length,4);
 for(let i=0;i<10;i++){
  const copied=path.join(received.cwd,'.refract-attachments',attachments[i].token);
  assert.equal(await fs.readFile(copied,'utf8'),`fixture-${extensions[i]}`);
  if(i<4)assert.ok(received.images.includes(copied));else assert.ok(received.prompt.includes(`.refract-attachments/${attachments[i].token}`));
 }
 const reopened=new ChatService(store,runner);await reopened.send(project.id,'Inspect the last file','default',attachments.slice(10));
 assert.ok(received.prompt.includes(`.refract-attachments/${attachments[10].token}`));
 const messages=(await reopened.load(project.id)).messages;
 assert.deepEqual(messages[0].attachments,attachments.slice(0,10));assert.deepEqual(messages[2].attachments,attachments.slice(10));
 await fs.unlink(path.join(store.projectDir(project.id),'.refract-attachments',attachments[10].token));
 await assert.rejects(reopened.send(project.id,'Missing file','default',attachments.slice(10)),/ENOENT/);
 assert.equal((await reopened.load(project.id)).messages.length,4);
});
test('speed overrides reach fresh and resumed Codex commands', () => {
 for (const session of [null,'session-1']) {
 for (const fastMode of [true,false]) {
 const args = codexArguments('codex.js',session,[],{fastMode});
 assert.ok(args.includes(`service_tier="${fastMode ? 'fast' : 'default'}"`));
 assert.ok(args.includes('features.fast_mode=true'));
 }
 assert.ok(!codexArguments('codex.js',session,[]).some((arg: string) => arg.startsWith('service_tier=')));
 }
 assert.throws(() => codexArguments('codex.js',null,[],{fastMode:'yes'}),/Invalid Codex speed/);
});
