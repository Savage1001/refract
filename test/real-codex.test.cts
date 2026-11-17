const {test}=require('node:test'),assert=require('node:assert/strict');
const fs=require('node:fs/promises'),os=require('node:os'),path=require('node:path');
const {ProjectStore}=require('../src/project-store.cjs'),{ChatService}=require('../src/chat.cjs');
const {runWarmCodex,closeCodexRuntimes}=require('../src/codex-runtime.cjs');

function solidRedPng(): Buffer {
 const zlib=require('node:zlib'),width=64,height=64,header=Buffer.alloc(13);
 header.writeUInt32BE(width,0);header.writeUInt32BE(height,4);header[8]=8;header[9]=2;
 const pixels=Buffer.alloc((1+width*3)*height);
 for(let y=0;y<height;y++)for(let x=0;x<width;x++)pixels[y*(1+width*3)+1+x*3]=255;
 const chunk=(type:string,data:Buffer)=>{
  const label=Buffer.from(type),size=Buffer.alloc(4),checksum=Buffer.alloc(4);size.writeUInt32BE(data.length);
  let crc=0xffffffff;for(const byte of Buffer.concat([label,data])){crc^=byte;for(let i=0;i<8;i++)crc=(crc>>>1)^((crc&1) ? 0xedb88320 : 0);}
  checksum.writeUInt32BE((crc^0xffffffff)>>>0);return Buffer.concat([size,label,data,checksum]);
 };
 return Buffer.concat([Buffer.from('89504e470d0a1a0a','hex'),chunk('IHDR',header),chunk('IDAT',zlib.deflateSync(pixels)),chunk('IEND',Buffer.alloc(0))]);
}

async function testRoot(t) {
 const tempParent=await fs.realpath(os.tmpdir()),root=await fs.mkdtemp(path.join(tempParent,'refract-codex-test-'));
 t.after(async()=>{
  await closeCodexRuntimes();
  const resolved=await fs.realpath(root);
  if(path.dirname(resolved).toLowerCase()!==tempParent.toLowerCase() || !path.basename(resolved).startsWith('refract-codex-test-')) throw new Error(`Unsafe cleanup path: ${resolved}`);
  await fs.rm(resolved,{recursive:true,force:true,maxRetries:5,retryDelay:200});
 });
 return root;
}

test('real Codex conversations resume across service restarts', {timeout:240000,skip:process.env.REFRACT_REAL_CODEX_TEST!=='1'},async t=>{
 const root=await testRoot(t);
 const store=new ProjectStore(root),project=await store.create('Integration test');
 const statuses=[];
 const first=await new ChatService(store,runWarmCodex).send(project.id,'Reply with the single word REFRACTREADY. Do not change any files.','default',[],undefined,status=>statuses.push(status));
 assert.ok(first.sessionId,'Codex did not return a session ID');
 assert.equal(first.messages.at(-1).role,'assistant',first.messages.at(-1).text);
 assert.match(first.messages.at(-1).text,/REFRACTREADY/i);
 assert.ok(statuses.length>0,'Codex did not report progress');
 await closeCodexRuntimes();
 const second=await new ChatService(store,runWarmCodex).send(project.id,'Reply with the single word REFRACTAGAIN. Do not change any files.');
 assert.equal(second.sessionId,first.sessionId);
 assert.equal(second.messages.at(-1).role,'assistant',second.messages.at(-1).text);
 assert.match(second.messages.at(-1).text,/REFRACTAGAIN/i);
 assert.equal((await new ChatService(store,runWarmCodex).load(project.id)).messages.length,4);
});

test('a real interrupted Codex session can be resumed', {timeout:120000,skip:process.env.REFRACT_REAL_CODEX_TEST!=='1'},async t=>{
 const root=await testRoot(t),store=new ProjectStore(root),project=await store.create('Cancellation test'),controller=new AbortController();
 const timer=setTimeout(()=>controller.abort(),30000);
 let chat;try{chat=await new ChatService(store,runWarmCodex).send(project.id,'Consider a detailed proof of the prime number theorem. Do not modify files.','default',[],controller.signal,status=>{if(status==='Thinking…')controller.abort();});}finally{clearTimeout(timer);}
 assert.equal(chat.messages.at(-1).role,'error');assert.match(chat.messages.at(-1).text,/cancelled/i);assert.ok(chat.sessionId);
 const saved=await new ChatService(store,runWarmCodex).load(project.id);assert.equal(saved.sessionId,chat.sessionId);
 const resumed=await new ChatService(store,runWarmCodex).send(project.id,'Reply with the single word REFRACTRESUMED. Do not modify files.');
 assert.equal(resumed.sessionId,chat.sessionId);assert.equal(resumed.messages.at(-1).role,'assistant',resumed.messages.at(-1).text);
 assert.match(resumed.messages.at(-1).text,/REFRACTRESUMED/i);
});

test('real Codex can use a document and an image in the same request', {timeout:180000,skip:process.env.REFRACT_REAL_CODEX_TEST!=='1'},async t=>{
 const root=await testRoot(t),store=new ProjectStore(path.join(root,'projects')),project=await store.create('Attachment test');
 const textFile=path.join(root,'note.txt'),imageFile=path.join(root,'red.png');
 await fs.writeFile(textFile,'The identifier is REFRACTMIXED.');
 await fs.writeFile(imageFile,solidRedPng());
 const service=new ChatService(store,runWarmCodex),attachments=await service.attach(project.id,[textFile,imageFile]);
 const controller=new AbortController(),timer=setTimeout(()=>controller.abort(),120000);
 let chat;try{chat=await service.send(project.id,'Read the attached text file and inspect the attached image. Reply with the identifier and the image color. Do not modify files.','default',attachments,controller.signal);}finally{clearTimeout(timer);}
 assert.equal(chat.messages.at(-1).role,'assistant',chat.messages.at(-1).text);
 assert.match(chat.messages.at(-1).text,/REFRACTMIXED/i);assert.match(chat.messages.at(-1).text,/red/i);
 assert.equal((await new ChatService(store,runWarmCodex).load(project.id)).messages[0].attachments.length,2);
 const resumedController = new AbortController(), resumedTimer = setTimeout(()=>resumedController.abort(),60000);
 let resumed;
 try { resumed = await new ChatService(store,runWarmCodex).send(project.id,'What identifier and image color did I provide in the previous message? Do not modify files.','default',[],resumedController.signal); }
 finally { clearTimeout(resumedTimer); }
 assert.equal(resumed.sessionId,chat.sessionId);
 assert.equal(resumed.messages.at(-1).role,'assistant',resumed.messages.at(-1).text);
 assert.match(resumed.messages.at(-1).text,/REFRACTMIXED/i); assert.match(resumed.messages.at(-1).text,/red/i);
});

test('real Codex can read an attached text document', {timeout:90000,skip:process.env.REFRACT_REAL_CODEX_TEST!=='1'},async t=>{
 const root=await testRoot(t),store=new ProjectStore(path.join(root,'projects')),project=await store.create('Document test');
 const file=path.join(root,'note.txt');await fs.writeFile(file,'The identifier is REFRACTDOCUMENT.');
 const service=new ChatService(store,runWarmCodex),attachments=await service.attach(project.id,[file]),controller=new AbortController();
 const timer=setTimeout(()=>controller.abort(),60000);
 let chat;try{chat=await service.send(project.id,'Read the attached text file and report its identifier. Do not modify files.','default',attachments,controller.signal);}finally{clearTimeout(timer);}
 assert.equal(chat.messages.at(-1).role,'assistant',chat.messages.at(-1).text);
 assert.match(chat.messages.at(-1).text,/REFRACTDOCUMENT/i);
});
