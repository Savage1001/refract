import testModule = require('node:test');
import assert = require('node:assert/strict');
import fs = require('node:fs/promises');
import os = require('node:os');
import path = require('node:path');
import playwright = require('playwright');
const {test} = testModule, {_electron:electron} = playwright;
declare const store: any;

test('desktop wires direct tools into both providers, saves compiler changes and snapshots edits', {timeout:60000}, async t => {
  const parent = await fs.realpath(os.tmpdir()), data = await fs.mkdtemp(path.join(parent,'refract-tools-ui-'));
  const env: NodeJS.ProcessEnv = {...process.env,REFRACT_DATA_DIR:data,REFRACT_TEST_HIDDEN:'1'}; delete env.ELECTRON_RUN_AS_NODE;
  const app = await electron.launch({args:[path.resolve(__dirname,'../..')],env});
  const compilerAvailable=await fs.access(path.join(process.env.REFRACT_TEX_BIN || 'missing-tex','pdflatex.exe')).then(()=>true,()=>false);
  t.after(async()=>{await app.close();assert.equal(path.dirname(await fs.realpath(data)),parent);await fs.rm(data,{recursive:true,force:true,maxRetries:5,retryDelay:200});});
  await app.evaluate(({app},compilerAvailable) => {
    const req = (process.getBuiltinModule('module') as any).createRequire(app.getAppPath()+'\\package.json');
    const {ChatService} = req('./build/src/chat.cjs');
    const original = ChatService.prototype.send;
    ChatService.prototype.send = function(...args) {
      const runner = async (_cwd,_session,prompt,_images,_signal,_progress,_persist,options) => {
        if (!options.tools || !prompt.includes('project_edit')) throw new Error('Missing direct tool capability');
        if (!prompt.includes('Choose the fastest LaTeX engine that fully meets')) throw new Error('Missing engine selection guidance');
        const connection = options.tools;
        const call = async (name,arguments_) => {
          const response = await fetch(connection.url,{method:'POST',headers:{Authorization:`Bearer ${connection.token}`,'Content-Type':'application/json',Accept:'application/json, text/event-stream'},body:JSON.stringify({jsonrpc:'2.0',id:1,method:'tools/call',params:{name,arguments:arguments_}})});
          const reply = await response.json() as {result?:{isError?:boolean;content:Array<{text:string}>};error?:unknown};
          if (!response.ok || reply.error || reply.result?.isError) throw new Error(JSON.stringify(reply));
          return JSON.parse(reply.result.content[0].text);
        };
        const read = await call('project_read',{path:'main.tex'});
        await call('project_edit',{path:'main.tex',sha256:read.sha256,oldText:read.content.includes('Introduction') ? 'Introduction' : 'First confirmed edit',newText:read.content.includes('Introduction') ? 'First confirmed edit' : 'Second confirmed edit'});
        const engine = read.content.includes('Introduction') ? 'xelatex' : 'lualatex';
        const previousEngine = engine === 'xelatex' ? 'pdflatex' : 'xelatex';
        if (!prompt.includes(`start of this turn is ${previousEngine}`)) throw new Error('Wrong initial engine in prompt');
        if (args[7] === 'codex' && !options.developerInstructions?.includes(`start of this turn is ${previousEngine}`)) throw new Error('Missing Codex developer instructions');
        const change = await call('project_set_engine', {engine});
        if (change.engine !== engine || change.previousEngine !== previousEngine || change.changed !== true) throw new Error('Expected the project engine to change');
        const repeated = await call('project_set_engine', {engine});
        if (repeated.changed !== false) throw new Error('Selecting the same engine should be a no-op');
        if (compilerAvailable) {
          const build = await call('build', {});
          if (!build.ok || build.engine !== engine) throw new Error(`Build did not succeed with ${engine}: ${build.log}`);
        }
        (globalThis as any).__lastToolConnection=connection;
        return {sessionId:null,text:'Confirmed edit'};
      };
      this.runner=runner;this.openCodeRunner=runner;
      return original.apply(this,args);
    };
  },compilerAvailable);
  const page = await app.firstWindow();
  await page.evaluate(() => store.codexAccountSelect('cli'));
  await page.locator('#new').click(); await page.locator('#source').waitFor({state:'visible'});
  const id: string = await page.evaluate(async()=>{const [project]=await store.list();return project.id;});
  await page.locator('#tools').click();
  await page.locator('#tool-info').click();
  let finalBuildMinute = 0;
  for (const provider of ['codex','opencode']) {
    finalBuildMinute = Math.floor(Date.now() / 60000);
    const chat = await page.evaluate(({id,provider})=>store.chatSend(id,'Edit the section','default',[],provider),{id,provider});
    assert.equal(chat.messages.at(-1).text,'Confirmed edit');
    const engine = provider === 'codex' ? 'xelatex' : 'lualatex';
    await page.waitForFunction(engine => document.querySelector<HTMLSelectElement>('[aria-label="LaTeX engine"]')?.value === engine, engine);
    const metadata = JSON.parse(await fs.readFile(path.join(data,'projects',id,'.refract.json'),'utf8'));
    assert.equal(metadata.engine, engine);
    const connection = await app.evaluate(()=>(globalThis as any).__lastToolConnection);
    await assert.rejects(fetch(connection.url,{method:'POST',headers:{Authorization:`Bearer ${connection.token}`}}));
    assert.ok(!JSON.stringify(chat).includes(connection.token));
  }
  assert.match(await fs.readFile(path.join(data,'projects',id,'main.tex'),'utf8'),/Second confirmed edit/);
  const versions = await page.evaluate(id=>store.versionList(id),id);
  assert.ok(versions.length>=3,'Before and after snapshots were not captured');
  const performance=await page.evaluate(id=>store.assistantPerformance(id),id);
  assert.ok(performance.totalMs>0);assert.equal(performance.counts.toolCalls,compilerAvailable ? 5 : 4);
  assert.ok(performance.phasesMs.snapshotBefore>=0);assert.ok(performance.phasesMs.snapshotAfter>=0);
  if(compilerAvailable) {
    let automatic = await page.evaluate(id => store.compile(id), id);
    assert.equal(automatic.ok, true);
    // A minute boundary legitimately invalidates the assistant's build cache entry.
    if (!automatic.cached && Math.floor(Date.now() / 60000) !== finalBuildMinute) {
      automatic = await page.evaluate(id => store.compile(id), id);
      assert.equal(automatic.ok, true);
    }
    assert.equal(automatic.cached, true);
    const forced=await page.evaluate(id=>store.compile(id,true),id);assert.equal(forced.ok,true);assert.equal(forced.cached,false);
  }
});
