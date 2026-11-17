const {test} = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const os = require('node:os');
const path = require('node:path');
const {startAssistantBuildBridge,configuredBuild,lookupPackage} = require('../src/assistant-build-bridge.cjs');
const {compile} = require('../src/compiler.cjs');
const {ChatService} = require('../src/chat.cjs');
const {ProjectStore} = require('../src/project-store.cjs');

test('a build cancelled before it starts preserves previous outputs', async t => {
 const root=await fs.mkdtemp(path.join(os.tmpdir(),'refract-cancelled-build-'));
 t.after(()=>fs.rm(root,{recursive:true,force:true}));
 await require('../src/project-trust.cjs').grantTrust(root);
 const output=path.join(root,'.refract-build');await fs.mkdir(output);
 await fs.writeFile(path.join(output,'main.pdf'),'previous PDF');
 await fs.writeFile(path.join(output,'main.synctex.gz'),'previous SyncTeX');
 const controller=new AbortController();controller.abort();
 const result=await compile(root,{signal:controller.signal});
 assert.equal(result.ok,false);assert.match(result.log,/cancelled/);
 assert.equal(await fs.readFile(path.join(output,'main.pdf'),'utf8'),'previous PDF');
 assert.equal(await fs.readFile(path.join(output,'main.synctex.gz'),'utf8'),'previous SyncTeX');
});

test('bridge authenticates and serializes requests and expires after use', async () => {
 let release: () => void; const gate = new Promise<void>(r=>release=r);
 const bridge=await startAssistantBuildBridge(async()=>{await gate;return {id:'fresh'};},async name=>({name,available:false}));
 const url=bridge.instructions.match(/http:\/\/127\.0\.0\.1:\d+/)[0];
 const token=bridge.instructions.match(/Bearer ([0-9a-f]+)/)[1];
 const headers={Authorization:`Bearer ${token}`};
 assert.equal((await fetch(url+'/build',{method:'POST'})).status,403);
 assert.equal((await fetch(url+'/package?name=../x.sty',{method:'POST',headers})).status,400);
 assert.deepEqual(await (await fetch(url+'/package?name=missing.sty',{method:'POST',headers})).json(),{name:'missing.sty',available:false});
 const pending=fetch(url+'/build',{method:'POST',headers});
 await new Promise(resolve=>setTimeout(resolve,50));
 assert.equal((await fetch(url+'/build',{method:'POST',headers})).status,409);
 release(); assert.deepEqual(await (await pending).json(),{id:'fresh'});
 await bridge.close(); await assert.rejects(fetch(url+'/build',{method:'POST',headers}));
});

test('both providers receive the current build capability without saving it to chat',async t=>{
 const root=await fs.mkdtemp(path.join(os.tmpdir(),'refract-assistant-bridge-'));t.after(()=>fs.rm(root,{recursive:true,force:true}));
 const store=new ProjectStore(root),project=await store.create('test');const prompts:string[]=[];
 const runner=async (_cwd,_session,prompt)=>{prompts.push(prompt);return {sessionId:null,text:'done'};};
 const chat=new ChatService(store,runner,runner);
 for(const provider of ['codex','opencode']) await chat.send(project.id,'fix','default',[],undefined,undefined,undefined,provider,'test capability');
 assert.equal(prompts.length,2);for(const prompt of prompts)assert.match(prompt,/test capability/);
 assert.doesNotMatch(JSON.stringify(await chat.load(project.id)),/test capability/);
});

test('configured engine and root produce fresh package failure and successful PDF', {timeout:90000}, async t=>{
 const bin=path.resolve(process.env.REFRACT_TEX_BIN || 'missing-tex');const executable=path.join(bin,'xelatex.exe');
 try {await fs.access(executable);}catch{return t.skip('External TeX is not configured');}
 const parent=await fs.mkdtemp(path.join(os.tmpdir(),'refract-package-'));t.after(()=>fs.rm(parent,{recursive:true,force:true}));
 const store=new ProjectStore(parent), project=await store.create('Paper'), root=store.projectDir(project.id);
 await store.setEngine(project.id,'xelatex');const meta=await store.metadata(project.id);meta.rootFile='paper.tex';await fs.writeFile(path.join(root,'.refract.json'),JSON.stringify(meta));
 const options=await configuredBuild(store,project.id,bin);assert.equal(options.engine,'xelatex');assert.equal(options.rootFile,'paper.tex');
 assert.equal((await lookupPackage('article.cls',root,bin)).available,true);assert.equal((await lookupPackage('refract-package-does-not-exist.sty',root,bin)).available,false);
 await fs.writeFile(path.join(root,'local-example.sty'),'');assert.equal((await lookupPackage('local-example.sty',root,bin)).available,true);
 await fs.writeFile(path.join(root,'paper.tex'),'\\documentclass{article}\n\\usepackage{refract-package-does-not-exist}\n\\begin{document}Test\\end{document}');
 await fs.mkdir(path.join(root,'.refract-build'));await fs.writeFile(path.join(root,'.refract-build','main.pdf'),'stale');await fs.writeFile(path.join(root,'.refract-build','main.log'),'stale');
 const failed=await compile(root,options);
 assert.equal(failed.ok,false);assert.notEqual(failed.exitCode,0);assert.match(failed.log,/refract-package-does-not-exist/);await assert.rejects(fs.access(path.join(root,'.refract-build','main.pdf')));
 await fs.writeFile(path.join(root,'paper.tex'),'\\documentclass{article}\n\\begin{document}Fixed\\end{document}');
 const success=await compile(root,options);assert.equal(success.ok,true,success.log);assert.equal(success.exitCode,0);assert.match(success.log,/XeTeX/);assert.equal(Buffer.from(success.pdf,'base64').subarray(0,5).toString(),'%PDF-');
});

test('cancelling an assistant build reaches the compiler and bridge waits for cleanup', async()=>{
 const controller=new AbortController(); let started: () => void; const ready=new Promise<void>(r=>started=r);
 const bridge=await startAssistantBuildBridge(()=>new Promise(resolve=>{started();controller.signal.addEventListener('abort',()=>resolve({ok:false,exitCode:null}),{once:true});}),async()=>({}));
 const url=bridge.instructions.match(/http:\/\/127\.0\.0\.1:\d+/)[0];const token=bridge.instructions.match(/Bearer ([0-9a-f]+)/)[1];
 const response=fetch(url+'/build',{method:'POST',headers:{Authorization:`Bearer ${token}`}});await ready;controller.abort();
 assert.deepEqual(await (await response).json(),{ok:false,exitCode:null});await bridge.close();
});

test('both assistant providers can request configured rebuilds after a package repair', {timeout:90000}, async t=>{
 const bin=path.resolve(process.env.REFRACT_TEX_BIN || 'missing-tex');
 try {await fs.access(path.join(bin,'xelatex.exe'));}catch{return t.skip('External TeX is not configured');}
 const root=await fs.mkdtemp(path.join(os.tmpdir(),'refract-provider-build-'));t.after(()=>fs.rm(root,{recursive:true,force:true}));
 const store=new ProjectStore(root),project=await store.create('test'),dir=store.projectDir(project.id);
 await store.setEngine(project.id,'xelatex');const metadata=await store.metadata(project.id);metadata.rootFile='paper.tex';await fs.writeFile(path.join(dir,'.refract.json'),JSON.stringify(metadata));
 let counter=0;
 const bridge=await startAssistantBuildBridge(async()=>{const options=await configuredBuild(store,project.id,bin);const result=await compile(dir,options);return {id:++counter,ok:result.ok,exitCode:result.exitCode,log:result.log,pdf:result.ok};},name=>lookupPackage(name,dir,bin));
 try {
  const runner=async (_cwd,_session,prompt)=>{
   const url=prompt.match(/http:\/\/127\.0\.0\.1:\d+/)[0],token=prompt.match(/Bearer ([0-9a-f]+)/)[1];
   const build=async()=>await (await fetch(url+'/build',{method:'POST',headers:{Authorization:`Bearer ${token}`}})).json();
   const failure=await build();assert.equal(failure.ok,false);assert.match(failure.log,/refract-package-does-not-exist/);
   await fs.writeFile(path.join(dir,'paper.tex'),'\\documentclass{article}\n\\begin{document}Fixed\\end{document}');
   const success=await build();assert.equal(success.ok,true,success.log);assert.notEqual(success.id,failure.id);assert.equal(success.pdf,true);
   return {sessionId:null,text:'fixed'};
  };
  const chat=new ChatService(store,runner,runner);
  for(const provider of ['codex','opencode']){
   await fs.writeFile(path.join(dir,'paper.tex'),'\\documentclass{article}\n\\usepackage{refract-package-does-not-exist}\n\\begin{document}Test\\end{document}');
   const result=await chat.send(project.id,'fix','default',[],undefined,undefined,undefined,provider,bridge.instructions);assert.equal(result.messages.at(-1).text,'fixed');
  }
 } finally {await bridge.close();}
});

test('documented PowerShell build command works in a fresh shell', {skip:process.platform!=='win32'},async()=>{
 const {execFile}=require('node:child_process');
 const bridge=await startAssistantBuildBridge(async()=>({id:'fresh',ok:true}),async()=>({available:true}));
 try {
  const command=bridge.instructions.split('\n')[1];
  const output=await new Promise<string>((resolve,reject)=>execFile('powershell.exe',['-NoProfile','-NonInteractive','-Command',command],{windowsHide:true},(error,stdout,stderr)=>error?reject(new Error(stderr)):resolve(stdout)));
  assert.deepEqual(JSON.parse(output),{id:'fresh',ok:true});
 } finally {await bridge.close();}
});

test('Codex restricted Windows sandbox can reach the bridge', {timeout:60000,skip:process.platform!=='win32' || process.env.REFRACT_SANDBOX_BRIDGE_TEST!=='1'},async t=>{
 const {execFile}=require('node:child_process');const root=await fs.mkdtemp(path.join(os.tmpdir(),'refract-sandbox-'));
 t.after(async()=>{const resolved=await fs.realpath(root),parent=await fs.realpath(os.tmpdir());if(path.dirname(resolved).toLowerCase()!==parent.toLowerCase() || !path.basename(resolved).startsWith('refract-sandbox-'))throw new Error('Unsafe cleanup path');await fs.rm(resolved,{recursive:true,force:true,maxRetries:5,retryDelay:200});});
 let called=false;const bridge=await startAssistantBuildBridge(async()=>{called=true;return {ok:true};},async()=>({}));
 try {
  const command=bridge.instructions.split('\n')[1];const script=path.join(process.env.APPDATA || '','npm','node_modules','@openai','codex','bin','codex.js');
  const output=await new Promise<string>((resolve,reject)=>execFile(process.execPath,[script,...(process.env.REFRACT_TEST_UNELEVATED_SANDBOX==='1' ? ['-c','windows.sandbox="unelevated"'] : []),'sandbox','--include-managed-config','-P',':workspace','-C',root,'--','powershell.exe','-NoProfile','-NonInteractive','-Command',command],{windowsHide:true,timeout:45000},(error,stdout,stderr)=>error?reject(new Error(stderr.replace(/Bearer [0-9a-f]+/g,'Bearer [redacted]'))):resolve(stdout)));
  assert.equal(called,true);assert.deepEqual(JSON.parse(output),{ok:true});
 } finally {await bridge.close();}
});
