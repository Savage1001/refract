const {test}=require('node:test');const assert=require('node:assert/strict');
const fs=require('node:fs/promises'),os=require('node:os'),path=require('node:path'),crypto=require('node:crypto');
const {ProjectStore}=require('../src/project-store.cjs');const {ChatService}=require('../src/chat.cjs');
const {runWarmCodex,closeCodexRuntimes}=require('../src/codex-runtime.cjs');
const {configuredBuild,lookupPackage}=require('../src/assistant-build-bridge.cjs');const {startAssistantTools}=require('../src/assistant-tools.cjs');const {compile}=require('../src/compiler.cjs');
test('real Codex repairs and rebuilds a missing package fixture',{timeout:240000,skip:process.platform!=='win32'||process.env.REFRACT_REAL_BUILD_BRIDGE_TEST!=='1'},async t=>{
 const root=await fs.mkdtemp(path.join(os.tmpdir(),'refract-real-build-'));
 t.after(async()=>{await closeCodexRuntimes();const resolved=await fs.realpath(root),parent=await fs.realpath(os.tmpdir());if(path.dirname(resolved).toLowerCase()!==parent.toLowerCase()||!path.basename(resolved).startsWith('refract-real-build-'))throw new Error('Unsafe cleanup path');await fs.rm(resolved,{recursive:true,force:true,maxRetries:5,retryDelay:200});});
 const bin=require('../src/tex-toolchain.cjs').texBin();await fs.access(path.join(bin,'xelatex.exe'));
 const store=new ProjectStore(root),project=await store.create('Test'),dir=store.projectDir(project.id);await store.setEngine(project.id,'xelatex');const metadata=await store.metadata(project.id);metadata.rootFile='paper.tex';await fs.writeFile(path.join(dir,'.refract.json'),JSON.stringify(metadata));
 await fs.writeFile(path.join(dir,'paper.tex'),'\\documentclass{article}\n\\usepackage{refract-fake-test-package}\n\\begin{document}Hello.\\end{document}\n');
 const initial=await compile(dir,await configuredBuild(store,project.id,bin));
 assert.equal(initial.ok,false);assert.match(initial.log,/refract-fake-test-package/);
 const results:Array<{ok:boolean;exitCode:number|null;log:string}>=[];
 const bridge=await startAssistantTools(store,project.id,async()=>{const settings=await configuredBuild(store,project.id,bin),result=await compile(dir,settings);results.push({ok:result.ok,exitCode:result.exitCode,log:result.log});return {id:crypto.randomUUID(),rootFile:settings.rootFile,engine:settings.engine,ok:result.ok,exitCode:result.exitCode,log:result.log,pdfPath:result.ok?path.join(dir,'.refract-build','main.pdf'):null};},name=>lookupPackage(name,dir,bin));
 const controller=new AbortController();const timer=setTimeout(()=>controller.abort(),180000);
 try {
  const chat=await new ChatService(store,runWarmCodex).send(project.id,'This is a disposable test fixture. The configured root paper.tex has just failed compilation because refract-fake-test-package is intentionally nonexistent. Use the refract project tools to remove only that package declaration from paper.tex, then call the refract build tool and report its result.','default',[],controller.signal,undefined,{tools:bridge.connection},'codex',bridge.instructions);
  assert.equal(chat.messages.at(-1).role,'assistant',chat.messages.at(-1).text);assert.ok(results.length>0,'Expected the provider to call the configured build tool');assert.ok(results.at(-1)?.ok,'Expected a successful rebuild');assert.equal((await fs.readFile(path.join(dir,'.refract-build','main.pdf'))).subarray(0,5).toString(),'%PDF-');
 } finally {clearTimeout(timer);await bridge.close();}
});
