const {test}=require('node:test');const assert=require('node:assert/strict');
const fs=require('node:fs/promises'),os=require('node:os'),path=require('node:path'),crypto=require('node:crypto');
const {ProjectStore}=require('../src/project-store.cjs');const {ChatService}=require('../src/chat.cjs');
const {startAssistantBuildBridge,configuredBuild,lookupPackage}=require('../src/assistant-build-bridge.cjs');const {compile}=require('../src/compiler.cjs');
test('real OpenCode repairs and rebuilds a missing package fixture',{timeout:240000,skip:process.platform!=='win32'||process.env.REFRACT_REAL_OPENCODE_BUILD_TEST!=='1'},async t=>{
 const root=await fs.mkdtemp(path.join(os.tmpdir(),'refract-real-build-'));
 t.after(async()=>{const resolved=await fs.realpath(root),parent=await fs.realpath(os.tmpdir());if(path.dirname(resolved).toLowerCase()!==parent.toLowerCase()||!path.basename(resolved).startsWith('refract-real-build-'))throw new Error('Unsafe cleanup path');await fs.rm(resolved,{recursive:true,force:true,maxRetries:5,retryDelay:200});});
 const bin=require('../src/tex-toolchain.cjs').texBin();await fs.access(path.join(bin,'xelatex.exe'));
 const store=new ProjectStore(root),project=await store.create('Test'),dir=store.projectDir(project.id);await store.setEngine(project.id,'xelatex');const metadata=await store.metadata(project.id);metadata.rootFile='paper.tex';await fs.writeFile(path.join(dir,'.refract.json'),JSON.stringify(metadata));
 await fs.writeFile(path.join(dir,'paper.tex'),'\\documentclass{article}\n\\usepackage{refract-fake-test-package}\n\\begin{document}Hello.\\end{document}\n');
 const results:Array<{ok:boolean;exitCode:number|null;log:string}>=[];
 const bridge=await startAssistantBuildBridge(async()=>{const result=await compile(dir,await configuredBuild(store,project.id,bin));results.push({ok:result.ok,exitCode:result.exitCode,log:result.log});return {id:crypto.randomUUID(),ok:result.ok,exitCode:result.exitCode,log:result.log,pdfPath:result.ok?path.join(dir,'.refract-build','main.pdf'):null};},name=>lookupPackage(name,dir,bin));
 const controller=new AbortController();const timer=setTimeout(()=>controller.abort(),180000);
 try {
  const chat=await new ChatService(store).send(project.id,'This is a disposable test fixture. Build it using the provided interface. If the intentionally nonexistent refract-fake-test-package is the cause, remove only that test package declaration, then rebuild and report the result.','default',[],controller.signal,undefined,undefined,'opencode',bridge.instructions);
  assert.equal(chat.messages.at(-1).role,'assistant',chat.messages.at(-1).text);assert.ok(results.some(r=>!r.ok&&/refract-fake-test-package/.test(r.log)),'Expected a fresh missing-package build');assert.ok(results.at(-1)?.ok,'Expected a successful rebuild');assert.equal((await fs.readFile(path.join(dir,'.refract-build','main.pdf'))).subarray(0,5).toString(),'%PDF-');
 } finally {clearTimeout(timer);await bridge.close();}
});
