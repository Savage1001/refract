import nodeTest = require('node:test');
const { test } = nodeTest;
import assert = require('node:assert/strict');
import fs = require('node:fs/promises');
import os = require('node:os');
import path = require('node:path');
import fsSync = require('node:fs');
import crypto = require('node:crypto');
import setup = require('../scripts/install-dependencies.cjs');
import toolchain = require('../src/tex-toolchain.cjs');

test('managed toolchain requires all tools and custom configuration takes priority', async t => {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), 'refract-dependency-test-'));
  t.after(async () => { await fs.rm(root, { recursive: true, force: true }); });
  const tex = path.join(root, 'Refract/dependencies/TinyTeX');
  const bin = path.join(tex, 'bin/windows');
  await fs.mkdir(bin, { recursive: true });
  assert.equal(await setup.complete(tex), false);
  for (const name of ['pdflatex', 'xelatex', 'lualatex', 'kpsewhich', 'synctex', 'bibtex', 'biber']) await fs.writeFile(path.join(bin, `${name}.exe`), 'fixture');
  assert.equal(await setup.complete(tex), true);
  assert.equal(toolchain.texBin({ LOCALAPPDATA: root }), bin);
  assert.equal(toolchain.texBin({ LOCALAPPDATA: root, REFRACT_TEX_BIN: root }), root);
  await fs.unlink(path.join(bin, 'biber.exe'));
  assert.equal(await setup.complete(tex), false);
  assert.throws(() => toolchain.texBin({LOCALAPPDATA:root}), /installer/);
});

async function fixture(t: nodeTest.TestContext): Promise<string> {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), 'refract-dependency-setup-'));
  t.after(async () => { await fs.rm(root, {recursive:true,force:true,maxRetries:5,retryDelay:200}); });
  return root;
}
const tools = ['pdflatex', 'xelatex', 'lualatex', 'kpsewhich', 'synctex', 'bibtex', 'biber'];
function unpackFixture(archive:string, destination:string):void {
  const bin = path.join(destination, archive.endsWith('.zip') ? 'TinyTeX/bin/windows' : 'bin/windows');
  fsSync.mkdirSync(bin, {recursive:true});
  for (const tool of archive.endsWith('.zip') ? tools.filter(name=>name!=='synctex') : ['synctex']) fsSync.writeFileSync(path.join(bin, `${tool}.exe`), 'fixture');
}

test('fresh setup installs all required tools and preserves an incomplete previous installation', async t => {
  const root=await fixture(t), dependencies=path.join(root,'Refract/dependencies'), target=path.join(dependencies,'TinyTeX');
  await fs.mkdir(target,{recursive:true});await fs.writeFile(path.join(target,'keep.txt'),'previous');
  const downloads:string[]=[];
  await setup.install({localAppData:root,download:async(url)=>{downloads.push(url);},extract:unpackFixture});
  assert.equal(downloads.length,2);assert.equal(await setup.complete(target),true);
  assert.equal(toolchain.texBin({LOCALAPPDATA:root}),path.join(target,'bin/windows'));
  const entries=await fs.readdir(dependencies),previous=entries.find(name=>name.startsWith('previous-'));
  assert.ok(previous);assert.equal(await fs.readFile(path.join(dependencies,previous,'keep.txt'),'utf8'),'previous');
  assert.equal(entries.some(name=>name.startsWith('setup-')),false);
  await setup.install({localAppData:root,download:async()=>{assert.fail('complete installation must be reused');}});
});

test('download failures preserve existing data, clean staging and permit retry', async t => {
  const root=await fixture(t), dependencies=path.join(root,'Refract/dependencies'),target=path.join(dependencies,'TinyTeX');
  await fs.mkdir(target,{recursive:true});await fs.writeFile(path.join(target,'keep.txt'),'previous');
  await assert.rejects(setup.install({localAppData:root,download:async()=>{throw new Error('offline');}}),/offline/);
  assert.deepEqual(await fs.readdir(dependencies),['TinyTeX']);
  assert.equal(await fs.readFile(path.join(target,'keep.txt'),'utf8'),'previous');
  await setup.install({localAppData:root,download:async()=>{},extract:unpackFixture});
  assert.equal(await setup.complete(target),true);
});

test('failed extraction never publishes a partial toolchain', async t => {
  const root=await fixture(t);
  await assert.rejects(setup.install({localAppData:root,download:async()=>{},extract:()=>{throw new Error('bad archive');}}),/bad archive/);
  assert.deepEqual(await fs.readdir(path.join(root,'Refract/dependencies')),[]);
});

test('missing tools after extraction reject setup and clean staging', async t => {
  const root=await fixture(t);
  await assert.rejects(setup.install({localAppData:root,download:async()=>{},extract:()=>{}}),/missing required tools/);
  assert.deepEqual(await fs.readdir(path.join(root,'Refract/dependencies')),[]);
});

test('downloads verify SHA256 before writing an archive', async t => {
  const root=await fixture(t),file=path.join(root,'archive'),bytes=Buffer.from('verified fixture');
  t.mock.method(globalThis,'fetch',async()=>new Response(bytes));
  await assert.rejects(setup.download('https://example.invalid/archive','0'.repeat(64),file),/Checksum mismatch/);
  await assert.rejects(fs.access(file));
  await setup.download('https://example.invalid/archive',crypto.createHash('sha256').update(bytes).digest('hex'),file);
  assert.deepEqual(await fs.readFile(file),bytes);
});

test('HTTP errors cannot produce a successful dependency setup', async t => {
  const root=await fixture(t);
  t.mock.method(globalThis,'fetch',async()=>new Response('unavailable',{status:503}));
  await assert.rejects(setup.download('https://example.invalid/archive','unused',path.join(root,'archive')),/Download failed: 503/);
});

test('setup rejects relative per-user data directories', async () => {
  await assert.rejects(setup.install({localAppData:'relative'}),/absolute directory/);
});
