import tests = require('node:test');
import assert = require('node:assert/strict');
import fs = require('node:fs/promises');
import os = require('node:os');
import path = require('node:path');
import runtime = require('../src/codex-runtime.cjs');
import caches = require('../src/source-cache.cjs');
import builds = require('../src/build-cache.cjs');
import metrics = require('../src/assistant-performance.cjs');
const {ProjectStore}=require('../src/project-store.cjs');
const {test}=tests;
async function root(t:tests.TestContext) {
  const parent=await fs.realpath(os.tmpdir()),directory=await fs.mkdtemp(path.join(parent,'refract-speed-'));
  t.after(async()=>{assert.equal(path.dirname(await fs.realpath(directory)),parent);await fs.rm(directory,{recursive:true,force:true,maxRetries:5,retryDelay:200});});return directory;
}
test('warm Codex retains the process, cancels without losing its session, and rejects crashes',async t=>{
  const directory=await root(t),client=new runtime.CodexRuntime(directory,process.execPath,[path.join(__dirname,'fixtures/codex-runtime.cjs')]);
  t.after(()=>client.close());
  const first=await client.run(null,'hello',[]),second=await client.run(first.sessionId,'hello',[]);
  assert.equal(first.text.split(':')[1],second.text.split(':')[1]);assert.ok(second.text.endsWith(':2'));
  const controller=new AbortController();
  await assert.rejects(client.run(first.sessionId,'wait',[],controller.signal,status=>{if(status.startsWith('Thinking'))controller.abort();}),/cancelled/);
  assert.equal((await client.run(first.sessionId,'hello',[])).sessionId,first.sessionId);
  await assert.rejects(client.run(first.sessionId,'crash',[]),/stopped/);
});
test('source cache detects in-place writes with restored mtime, replacements and eviction',async t=>{
  const directory=await root(t),file=path.join(directory,'source.tex'),cache=new caches.SourceCache(10);
  await fs.writeFile(file,'original');const before=await fs.stat(file),first=await cache.read(file);
  assert.equal((await cache.read(file)).sha256,first.sha256);
  await fs.writeFile(file,'modified');await fs.utimes(file,before.atime,before.mtime);
  assert.equal((await cache.read(file)).data.toString(),'modified');
  await fs.writeFile(file+'.tmp','replaced');await fs.rename(file+'.tmp',file);
  assert.equal((await cache.read(file)).data.toString(),'replaced');
  await fs.writeFile(file,'larger than the cache limit');assert.equal((await cache.read(file)).data.toString(),'larger than the cache limit');
});
test('build cache reuses only unchanged successful inputs, dependencies and outputs',async t=>{
  t.mock.timers.enable({apis:['Date'],now:new Date('2026-01-01T12:00:00Z')});
  const directory=await root(t),store=new ProjectStore(directory),project=await store.create('Cache'),cwd=store.projectDir(project.id),output=path.join(cwd,'.refract-build');
  const executable=path.join(directory,'engine.exe'),dependency=path.join(directory,'external.sty');await fs.writeFile(executable,'engine');await fs.writeFile(dependency,'package');
  let calls=0,fail=false;
  const cache=new builds.BuildCache(store,async()=>{
    calls++;await fs.mkdir(output,{recursive:true});
    for(const name of ['main.pdf','main.log','main.synctex.gz'])await fs.writeFile(path.join(output,name),name);
    await fs.writeFile(path.join(output,'main.fls'),`INPUT ${dependency}\nINPUT ${path.join(cwd,'main.tex')}\n`);
    return {ok:!fail,pdf:'fixture',log:'log',exitCode:fail?1:0};
  });
  const options={executable,rootFile:'main.tex'};
  await cache.run(project.id,cwd,options);
  // A conservative miss is valid while Windows settles freshly written file
  // metadata. Priming is bounded; a cache that never reuses still fails.
  let hit=await cache.run(project.id,cwd,options),attempts=1;
  while(!hit.cached && attempts++<3)hit=await cache.run(project.id,cwd,options);
  assert.equal(hit.cached,true);const primingCalls=calls;
  assert.equal((await cache.run(project.id,cwd,options)).cached,true);assert.equal(calls,primingCalls);
  await fs.writeFile(dependency,'updated');assert.equal((await cache.run(project.id,cwd,options)).cached,false);
  await store.write(project.id,'main.tex','changed source');assert.equal((await cache.run(project.id,cwd,options)).cached,false);
  await fs.writeFile(path.join(output,'main.pdf'),'replaced output');assert.equal((await cache.run(project.id,cwd,options)).cached,false);
  assert.equal((await cache.run(project.id,cwd,{...options,draftMode:true})).cached,false);
  assert.equal((await cache.run(project.id,cwd,{...options,draftMode:true},true)).cached,false);
  fail=true;await cache.run(project.id,cwd,options,true);assert.equal((await cache.run(project.id,cwd,options)).cached,false);
  const abort=new AbortController();abort.abort();assert.equal((await cache.run(project.id,cwd,{...options,signal:abort.signal})).exitCode,null);
});
test('performance diagnostics contain numeric timing and counts without request data',async()=>{
  const trace=new metrics.AssistantPerformance();await trace.measure('context',async()=>{});trace.count('contextCharacters',42);
  const report=trace.report();assert.equal(report.counts.contextCharacters,42);assert.ok(report.totalMs>=report.phasesMs.context!);
  assert.deepEqual(Object.keys(report).sort(),['counts','phasesMs','totalMs']);
});

test('real TeX builds are reused, and a changed input requires recompilation',{timeout:30000},async t=>{
  // Cache keys intentionally change at minute boundaries; hold only the
  // parent's clock stable while real compiler subprocesses and timers run.
  t.mock.timers.enable({apis:['Date'],now:new Date('2026-01-01T12:00:00Z')});
  const executable=path.join(process.env.REFRACT_TEX_BIN || 'missing-tex','pdflatex.exe');
  try{await fs.access(executable);}catch{return t.skip('External TeX is not configured');}
  const directory=await root(t),store=new ProjectStore(directory),project=await store.create('Real cache');
  const {compile}=require('../src/compiler.cjs'),cache=new builds.BuildCache(store,compile),cwd=store.projectDir(project.id),options={executable,rootFile:'main.tex'};
  const first=await cache.run(project.id,cwd,options);assert.equal(first.ok,true,first.log);
  const second=await cache.run(project.id,cwd,options);assert.equal(second.cached,true);assert.equal(second.pdf,first.pdf);
  await store.write(project.id,'main.tex','\\documentclass{article}\n\\begin{document}Changed\\end{document}');
  const changed=await cache.run(project.id,cwd,options);assert.equal(changed.cached,false);assert.equal(changed.ok,true,changed.log);
});

test('bounded handoff context keeps complete history available to a paging tool',async t=>{
  const directory=await root(t),store=new ProjectStore(directory),project=await store.create('Context');
  const {ChatService}=require('../src/chat.cjs'),{startAssistantTools}=require('../src/assistant-tools.cjs');
  let prompt='';const service=new ChatService(store,async(_cwd,_session,value)=>{prompt=value;return {sessionId:null,text:'done'};});
  await service.save(project.id,{provider:'opencode',sessionId:null,messages:[{role:'user',text:'EARLY_CONTEXT '+ 'x'.repeat(64000)},{role:'assistant',text:'LATEST_CONTEXT'}]});
  const bridge=await startAssistantTools(store,project.id,async()=>({}),async()=>({}),{history:()=>service.load(project.id)});
  try{
    await service.send(project.id,'Current user request','default',[],undefined,undefined,{tools:bridge.connection},'codex',bridge.instructions);
    assert.ok(prompt.length<36000);assert.match(prompt,/LATEST_CONTEXT/);assert.doesNotMatch(prompt,/EARLY_CONTEXT/);assert.match(prompt,/conversation_read/);
    const response=await fetch(bridge.connection.url,{method:'POST',headers:{Authorization:`Bearer ${bridge.connection.token}`,'Content-Type':'application/json',Accept:'application/json, text/event-stream'},body:JSON.stringify({jsonrpc:'2.0',id:1,method:'tools/call',params:{name:'conversation_read',arguments:{offset:0,limit:100}}})});
    const reply=await response.json() as {result:{content:Array<{text:string}>}};assert.match(reply.result.content[0].text,/EARLY_CONTEXT/);
    assert.match((await service.load(project.id)).messages[0].text,/EARLY_CONTEXT/);
  } finally{await bridge.close();}
});

 test('warm Codex applies speed settings to new and resumed threads',async t=>{
  const directory=await root(t),client=new runtime.CodexRuntime(directory,process.execPath,[path.join(__dirname,'fixtures/codex-runtime.cjs')]);
  try {
  const first=await client.run(null,'config',[],undefined,undefined,undefined,{fastMode:true});
  assert.equal(JSON.parse(first.text).service_tier,'fast');assert.equal(JSON.parse(first.text)['features.fast_mode'],true);
  const second=await client.run(first.sessionId,'config',[],undefined,undefined,undefined,{fastMode:false});
  assert.equal(JSON.parse(second.text).service_tier,'default');
  const inherited=await client.run(first.sessionId,'config',[]);
  assert.equal(JSON.parse(inherited.text).service_tier,undefined);
  } finally {await client.close();}
 });
