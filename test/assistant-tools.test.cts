import nodeTest = require('node:test');
import assert = require('node:assert/strict');
import fs = require('node:fs/promises');
import os = require('node:os');
import path = require('node:path');
import clientSdk = require('@modelcontextprotocol/sdk/client/index.js');
import transportSdk = require('@modelcontextprotocol/sdk/client/streamableHttp.js');
import assistantTools = require('../src/assistant-tools.cjs');
const {test} = nodeTest, {Client} = clientSdk, {StreamableHTTPClientTransport} = transportSdk, {startAssistantTools} = assistantTools;
const {ProjectStore} = require('../src/project-store.cjs');
const {codexArguments} = require('../src/chat.cjs');
const {openCodeEnvironment} = require('../src/opencode.cjs');

test('MCP tools verify edits, reject stale/ambiguous edits, and expire on close', async t => {
  const parent = await fs.realpath(os.tmpdir());
  const root = await fs.mkdtemp(path.join(parent, 'refract-tools-'));
  t.after(async () => {assert.equal(path.dirname(await fs.realpath(root)), parent); await fs.rm(root, {recursive:true, force:true});});
  const store = new ProjectStore(root), project = await store.create('Tools');
  const original = '\ufeffFirst α\r\nRepeated\r\nRepeated\r\n';
  await store.write(project.id, 'main.tex', original);
  let release: () => void;
  const gate = new Promise<void>(resolve => release = resolve);
  const bridge = await startAssistantTools(store, project.id, async () => {await gate; return {ok:true};}, async name => ({name, available:true}));
  t.after(() => bridge.close());
  const client = new Client({name:'test', version:'1.0.0'});
  await client.connect(new StreamableHTTPClientTransport(new URL(bridge.connection.url), {requestInit:{headers:{Authorization:`Bearer ${bridge.connection.token}`}}}));
  const call = async (name: string, args: Record<string, unknown> = {}) => client.callTool({name, arguments:args});
  const value = (response: Awaited<ReturnType<typeof call>>) => JSON.parse((response.content as Array<{text:string}>)[0].text);
  assert.equal((await fetch(bridge.connection.url, {method:'POST'})).status, 403);
  assert.equal((await fetch(bridge.connection.url, {method:'POST',headers:{Authorization:`Bearer ${bridge.connection.token}`,Origin:'http://untrusted.example'}})).status, 403);
  assert.equal((await client.listTools()).tools.length, 7);
  assert.deepEqual(value(await call('project_list')), {files:['main.tex']});
  const read = value(await call('project_read', {path:'main.tex'}));
  assert.equal(read.content, original);
  const excerpt=value(await call('project_read',{path:'main.tex',offset:2,limit:4}));assert.equal(excerpt.content,original.slice(2,6));assert.equal(excerpt.sha256,read.sha256);assert.equal(excerpt.nextOffset,6);
  await store.write(project.id,'second.tex','Second file');
  const batch=value(await call('project_read_many',{paths:['main.tex','second.tex','missing.tex']}));
  assert.equal(batch[0].content,original);assert.equal(batch[1].content,'Second file');assert.ok(batch[2].error);
  const edited=value(await call('project_edit_many',{edits:[{path:'second.tex',sha256:batch[1].sha256,oldText:'Second file',newText:'Edited file'},{path:'main.tex',sha256:read.sha256,oldText:'not found',newText:'no'}]}));
  assert.equal(edited[0].applied,true);assert.equal(edited[1].applied,false);
  for (const oldText of ['missing', 'Repeated']) assert.equal((await call('project_edit', {path:'main.tex', sha256:read.sha256, oldText, newText:'changed'})).isError, true);
  for (const newText of ['nul\0', '\ud800']) assert.equal((await call('project_edit', {path:'main.tex',sha256:read.sha256,oldText:'First α',newText})).isError,true);
  assert.equal(await store.read(project.id,'main.tex'),original);
  const changed = value(await call('project_edit', {path:'main.tex',sha256:read.sha256,oldText:'First α',newText:'Second β'}));
  assert.equal(changed.applied, true);
  assert.equal(await store.read(project.id, 'main.tex'), original.replace('First α', 'Second β'));
  assert.equal((await call('project_edit', {path:'main.tex',sha256:read.sha256,oldText:'Second β',newText:'lost update'})).isError, true);
  const concurrent = {path:'main.tex',sha256:changed.sha256,oldText:'Second β',newText:'Third γ'};
  const edits = await Promise.all([call('project_edit',concurrent),call('project_edit',concurrent)]);
  assert.equal(edits.filter(edit => !edit.isError).length, 1);
  for (const name of ['../outside.tex','.refract.json','a/.private/file','C:/outside.tex']) assert.equal((await call('project_read', {path:name})).isError, true);
  await fs.link(path.join(store.projectDir(project.id),'main.tex'), path.join(root,'outside.tex'));
  assert.equal((await call('project_read', {path:'main.tex'})).isError, true);
  await fs.unlink(path.join(root,'outside.tex'));
  await store.write(project.id, 'binary.bin', 'nul\x00data');
  assert.equal((await call('project_read', {path:'binary.bin'})).isError, true);
  await fs.writeFile(path.join(store.projectDir(project.id),'invalid.tex'),Buffer.from([0xff]));
  assert.equal((await call('project_read', {path:'invalid.tex'})).isError, true);
  await store.write(project.id,'large.tex','x'.repeat(2*1024*1024+1));
  assert.equal((await call('project_read', {path:'large.tex'})).isError, true);
  await fs.symlink(root,path.join(store.projectDir(project.id),'linked'),'junction');
  assert.equal((await call('project_read', {path:'linked/outside.tex'})).isError, true);
  await fs.unlink(path.join(store.projectDir(project.id),'linked'));
  assert.equal((await call('package_lookup', {name:'../bad.sty'})).isError, true);
  assert.deepEqual(value(await call('package_lookup', {name:'amsmath.sty'})), {name:'amsmath.sty',available:true});
  const pending = call('build');
  // A ping confirms that a second request can be served during a build.
  await client.ping();
  release(); assert.deepEqual(value(await pending), {ok:true});
  await client.close(); await bridge.close();
  await assert.rejects(fetch(bridge.connection.url, {method:'POST'}));
});

test('targeted reads and guarded replacements preserve all unrelated bytes', async t => {
  const parent = await fs.realpath(os.tmpdir());
  const root = await fs.mkdtemp(path.join(parent, 'refract-guarded-edits-'));
  t.after(async () => {
    assert.equal(path.dirname(await fs.realpath(root)), parent);
    await fs.rm(root, {recursive: true, force: true});
  });
  const store = new ProjectStore(root);
  const {id} = await store.create('Guarded edits');
  const file = 'main.tex';
  const prefix = '\ufeff% unchanged audit note\r\n' + '% audit\r\n'.repeat(3000);
  const original = prefix + 'Control\r\nPending.\r\nTreatment\r\nPending.\r\nVoltage: 7 mV. \u{1F4A1}\r\n';
  await store.write(id, file, original);
  const bridge = await startAssistantTools(store, id, async () => ({ok: true}), async () => ({}));
  const client = new Client({name: 'guarded-edits-test', version: '1.0.0'});
  t.after(async () => {await client.close(); await bridge.close();});
  await client.connect(new StreamableHTTPClientTransport(new URL(bridge.connection.url), {
    requestInit: {headers: {Authorization: `Bearer ${bridge.connection.token}`}},
  }));
  const call = (name: string, args: Record<string, unknown>) => client.callTool({name, arguments: args});
  const value = (response: Awaited<ReturnType<typeof call>>) => JSON.parse((response.content as Array<{text: string}>)[0].text);
  const first = value(await call('project_read', {path: file, find: 'Pending.'}));
  assert.ok(first.matchOffset > 16000 && first.content.length <= 600);
  const second = value(await call('project_read', {path: file, find: 'Pending.', offset: first.nextOffset}));
  assert.equal(second.sha256, first.sha256);
  assert.equal(second.matchOffset, original.lastIndexOf('Pending.'));
  assert.equal(second.nextOffset, null);
  assert.equal(value(await call('project_read', {path: file, find: 'unknown'})).matchOffset, null);
  const edit = {path: file, oldText: 'Treatment\nPending.', newText: 'Treatment\nApproved.'};
  for (const additional of [{oldText: 'Missing', newText: 'Present'}, {oldText: 'Treatment', newText: 'Test'}]) {
    assert.equal((await call('project_edit', {...edit, additional: [additional]})).isError, true);
    assert.equal(await store.read(id, file), original);
  }
  assert.equal((await call('project_edit', {path: file, oldText: 'Pending.', newText: 'Approved.'})).isError, true);
  assert.equal((await call('project_edit', {path: file, oldText: '\uD83D', newText: ''})).isError, true);
  assert.equal(await store.read(id, file), original);
  const response = await call('project_edit', {...edit, additional: [{oldText: '7 mV', newText: '9 mV'}]});
  assert.equal(value(response).applied, true);
  assert.equal(await store.read(id, file), original.replace('Treatment\r\nPending.', 'Treatment\r\nApproved.').replace('7 mV', '9 mV'));
  assert.equal((await call('project_edit', {path: file, sha256: second.sha256, oldText: '9 mV', newText: '11 mV'})).isError, true);
});

test('closing waits for a running tool before releasing the snapshot boundary', async t => {
  const parent = await fs.realpath(os.tmpdir()), root = await fs.mkdtemp(path.join(parent,'refract-tool-close-'));
  t.after(async()=>{assert.equal(path.dirname(await fs.realpath(root)),parent);await fs.rm(root,{recursive:true,force:true});});
  const store = new ProjectStore(root), project = await store.create('Close');
  let release: () => void, started: () => void;
  const gate = new Promise<void>(resolve => release=resolve), ready = new Promise<void>(resolve => started=resolve);
  const bridge = await startAssistantTools(store,project.id,async()=>{started();await gate;return {ok:true};},async()=>({}));
  const client = new Client({name:'close-test',version:'1.0.0'});
  await client.connect(new StreamableHTTPClientTransport(new URL(bridge.connection.url),{requestInit:{headers:{Authorization:`Bearer ${bridge.connection.token}`}}}));
  const building = client.callTool({name:'build',arguments:{}}); await ready;
  let closed = false; const closing = bridge.close().then(()=>{closed=true;});
  await new Promise(resolve=>setImmediate(resolve));assert.equal(closed,false);
  release(); assert.ok(!(await building).isError); await closing; await client.close();
});

test('the compiler tool accepts only supported engines for the current project', async t => {
  const parent = await fs.realpath(os.tmpdir()), root = await fs.mkdtemp(path.join(parent, 'refract-engine-tool-'));
  t.after(async () => {assert.equal(path.dirname(await fs.realpath(root)), parent); await fs.rm(root, {recursive:true, force:true});});
  const store = new ProjectStore(root), project = await store.create('Engine tool');
  const bridge = await startAssistantTools(store, project.id,
    async () => ({engine: (await store.metadata(project.id)).engine}), async () => ({}), {
      initialEngine: 'pdflatex',
      setEngine: async engine => {
        await store.setEngine(project.id, engine);
        return {engine, previousEngine: 'pdflatex', changed: true};
      }
    });
  t.after(() => bridge.close());
  const client = new Client({name: 'engine-test', version: '1.0.0'});
  await client.connect(new StreamableHTTPClientTransport(new URL(bridge.connection.url), {
    requestInit: {headers: {Authorization: `Bearer ${bridge.connection.token}`}}
  }));
  t.after(() => client.close());
  const change = (args: Record<string, unknown>) => client.callTool({name: 'project_set_engine', arguments: args});
  assert.match(bridge.instructions, /project_set_engine/);
  assert.match(bridge.instructions, /start of this turn is pdflatex/);
  assert.equal((await change({engine: '../pdflatex.exe'})).isError, true);
  assert.equal((await change({engine: 'xelatex', projectId: 'another-project'})).isError, true);
  assert.equal((await store.metadata(project.id)).engine, undefined);
  assert.equal((await change({engine: 'xelatex'})).isError, undefined);
  const build = await client.callTool({name: 'build', arguments: {}});
  assert.deepEqual(build.content, [{type: 'text', text: '{"engine":"xelatex"}'}]);
});

test('provider configuration is per turn and preserves existing OpenCode settings', () => {
  const tools = {url:'http://127.0.0.1:12345/mcp',token:'test-secret'};
  for (const session of [null, 'existing-session']) {
    const args = codexArguments('codex.js',session,[],{tools});
    assert.ok(args.includes('allow_login_shell=false'));
    assert.ok(args.includes(`mcp_servers.refract.url="${tools.url}"`));
    assert.ok(args.includes('workspace-write')); assert.ok(!args.join(' ').includes(tools.token));
  }
  const base = {OPENCODE_CONFIG_CONTENT:JSON.stringify({model:'existing',mcp:{other:{type:'remote',url:'https://example.com'}}})};
  const config = JSON.parse(openCodeEnvironment(tools,base).OPENCODE_CONFIG_CONTENT);
  assert.equal(config.model,'existing'); assert.equal(config.mcp.other.url,'https://example.com');
  assert.equal(config.mcp.refract.headers.Authorization,`Bearer ${tools.token}`);
  assert.equal(JSON.parse(base.OPENCODE_CONFIG_CONTENT).mcp.refract,undefined);
});

test('real providers use direct tools and reconnect a resumed session to a new capability', {timeout:240000,skip:process.env.REFRACT_REAL_TOOLS_TEST !== '1'}, async t => {
  const {runCodex} = require('../src/chat.cjs');
  const {runOpenCode} = require('../src/opencode.cjs');
  const parent = await fs.realpath(os.tmpdir()), root = await fs.mkdtemp(path.join(parent,'refract-real-tools-'));
  t.after(async()=>{await require('../src/codex-runtime.cjs').closeCodexRuntimes();assert.equal(path.dirname(await fs.realpath(root)),parent); await fs.rm(root,{recursive:true,force:true,maxRetries:5,retryDelay:200});});
  const store = new ProjectStore(root), project = await store.create('Real tools');
  const runtime = require('../src/codex-runtime.cjs');
  t.after(()=>runtime.closeCodexRuntimes());
  const runner = process.env.REFRACT_REAL_TOOLS_PROVIDER === 'opencode' ? runOpenCode : process.env.REFRACT_REAL_TOOLS_PROVIDER === 'codex-warm' ? runtime.runWarmCodex : runCodex;
  let session: string | null = null;
  for (let turn = 0; turn < 2; turn++) {
    let builds = 0;
    const bridge = await startAssistantTools(store,project.id,async()=>{builds++;return {ok:true,marker:'REFRACT_TOOL_BUILD'};},async()=>({available:true}));
    const controller = new AbortController(), timer = setTimeout(()=>controller.abort(),100000);
    try {
      const oldText = turn ? 'REFRACT_DIRECT_FIRST' : '\\section{Introduction}', newText = turn ? 'REFRACT_DIRECT_SECOND' : 'REFRACT_DIRECT_FIRST';
      const reply = await runner(store.projectDir(project.id),session,`${bridge.instructions}\nUse only the refract MCP tools, no shell. Read main.tex with project_read, replace the exact string ${JSON.stringify(oldText)} with ${JSON.stringify(newText)} using project_edit and its read hash. Call the refract build tool once. Reply with its marker and the edit result.`,[],controller.signal,undefined,undefined,{tools:bridge.connection});
      assert.ok(reply.sessionId); if (session) assert.equal(reply.sessionId,session); session = reply.sessionId;
      assert.match(await store.read(project.id,'main.tex'),new RegExp(newText),reply.text);
      assert.equal(builds,1); assert.match(reply.text,/REFRACT_TOOL_BUILD/);
    } finally {clearTimeout(timer); await bridge.close();}
  }
});
