const {test} = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const os = require('node:os');
const path = require('node:path');
const Zip = require('adm-zip');
const {ProjectStore} = require('../src/project-store.cjs');
const {importProject, exportProject} = require('../src/archive.cjs');

test('detects roots after escaped percent signs but ignores commented declarations', async () => {
 const root = await fs.mkdtemp(path.join(os.tmpdir(), 'refract-archive-comments-'));
 try {
  const archive = path.join(root, 'project.zip'), zip = new Zip();
  zip.addFile('paper.tex', Buffer.from('\\% text \\documentclass{article}\n'));
  zip.addFile('chapter.tex', Buffer.from('% \\documentclass{book}\n'));
  zip.writeZip(archive);
  const project = await importProject(new ProjectStore(path.join(root, 'projects')), archive, 'Paper');
  assert.equal(project.rootFile, 'paper.tex');
 } finally { await fs.rm(root, {recursive:true, force:true}); }
});
test('zip export and import preserve text and binary files', async t => {
 const root = await fs.mkdtemp(path.join(os.tmpdir(),'refract-archive-')); t.after(()=>fs.rm(root,{recursive:true,force:true}));
 const store = new ProjectStore(path.join(root,'projects')), p = await store.create('Paper');
 await fs.mkdir(path.join(store.projectDir(p.id),'images')); await fs.writeFile(path.join(store.projectDir(p.id),'images','figure.png'),Buffer.from([0,1,255,2]));
 await fs.mkdir(path.join(store.projectDir(p.id),'empty','nested'), {recursive:true});
 await fs.writeFile(path.join(store.projectDir(p.id),'.refract-comments.json'),'private comments');
 await fs.mkdir(path.join(store.projectDir(p.id),'.refract-versions')); await fs.writeFile(path.join(store.projectDir(p.id),'.refract-versions','history.zip'),'private history');
 const archive = path.join(root,'paper.zip'); await exportProject(store,p.id,archive);
 const exported = new Zip(archive).getEntries().map(entry=>entry.entryName);
 assert.ok(exported.every(name=>!name.split('/').some(part=>part.startsWith('.refract'))));
 const copy = await importProject(store,archive,'Copy');
 assert.equal(await store.read(copy.id,'main.tex'),await store.read(p.id,'main.tex'));
 assert.deepEqual(await fs.readFile(store.filePath(copy.id,'images/figure.png')),Buffer.from([0,1,255,2]));
 assert.equal((await fs.stat(store.filePath(copy.id,'empty/nested'))).isDirectory(),true);
});

test('archive export refuses linked project and file paths',async t=>{
 const root:string=await fs.mkdtemp(path.join(os.tmpdir(),'refract-export-links-'));
 t.after(async()=>{const resolved=await fs.realpath(root),parent=await fs.realpath(os.tmpdir());if(path.dirname(resolved).toLowerCase()!==parent.toLowerCase() || !path.basename(resolved).startsWith('refract-export-links-'))throw new Error('Unsafe cleanup path');await fs.rm(resolved,{recursive:true,force:true});});
 const store=new ProjectStore(path.join(root,'projects')),p=await store.create('Paper'),project=store.projectDir(p.id),outside=path.join(root,'outside'),archive=path.join(root,'output.zip');
 await fs.mkdir(outside);await fs.writeFile(path.join(outside,'secret.txt'),'private');
 try {await fs.symlink(outside,path.join(project,'linked'),process.platform==='win32'?'junction':'dir');}catch(error){if(['EPERM','EACCES'].includes(error.code))return t.skip('Creating directory links is not permitted');throw error;}
 const files=store.files;store.files=async()=>['linked/secret.txt'];
 try {await assert.rejects(exportProject(store,p.id,archive),/Invalid destination folder/);await assert.rejects(fs.access(archive),{code:'ENOENT'});}
 finally {store.files=files;await fs.unlink(path.join(project,'linked'));}
 const saved=path.join(root,'saved-project');await fs.rename(project,saved);await fs.symlink(outside,project,process.platform==='win32'?'junction':'dir');
 try {await assert.rejects(exportProject(store,p.id,archive),/Invalid project directory/);assert.equal(await fs.readFile(path.join(outside,'secret.txt'),'utf8'),'private');}
 finally {await fs.unlink(project);await fs.rename(saved,project);}
});
test('unsafe archives are rejected', async t => {
 const root = await fs.mkdtemp(path.join(os.tmpdir(),'refract-badzip-')); t.after(()=>fs.rm(root,{recursive:true,force:true}));
 const zip = new Zip(); zip.addFile('xx/escape',Buffer.from('x')); zip.addFile('main.tex',Buffer.from('x'));
 const file = path.join(root,'bad.zip'); const bytes = zip.toBuffer(); let offset = 0; while ((offset = bytes.indexOf('xx/escape', offset)) !== -1) { bytes.write('../escape', offset); offset += 9; } await fs.writeFile(file, bytes);
 await assert.rejects(importProject(new ProjectStore(path.join(root,'projects')),file,'Bad'),/Invalid path/);
});

test('Windows-invalid and conflicting archive paths are rejected before project creation', async t => {
 const root:string=await fs.mkdtemp(path.join(os.tmpdir(),'refract-archive-paths-')); t.after(()=>fs.rm(root,{recursive:true,force:true}));
 const store=new ProjectStore(path.join(root,'projects')); await store.init();
 for (const names of [['CON.txt'],['folder./file.txt'],['bad?.txt'],['Folder/a.txt','folder/b.txt'],['item','item/child.txt']]) {
  const zip=new Zip(); zip.addFile('main.tex',Buffer.from('\\documentclass{article}'));
  for (const name of names) zip.addFile(name,Buffer.from('data'));
  const archive=path.join(root,'bad.zip'); zip.writeZip(archive);
  await assert.rejects(importProject(store,archive,'Bad'),/Invalid path|conflicting paths/);
  assert.deepEqual(await store.list(),[]);
 }
});

test('oversized archives and inconsistent stored sizes are rejected before creating a project',async t=>{
 const root:string=await fs.mkdtemp(path.join(os.tmpdir(),'refract-archive-limits-'));t.after(()=>fs.rm(root,{recursive:true,force:true}));
 const store=new ProjectStore(path.join(root,'projects'));await store.init();
 const zip=new Zip();zip.addFile('main.tex',Buffer.from('\\documentclass{article}'));zip.getEntry('main.tex').header.method=0;
 const archive=path.join(root,'project.zip'),bytes=zip.toBuffer(),central=bytes.indexOf(Buffer.from([0x50,0x4b,0x01,0x02]));
 assert.ok(central>=0);bytes.writeUInt32LE(1,central+24);await fs.writeFile(archive,bytes);
 await assert.rejects(importProject(store,archive,'Bad'),/Invalid stored file size/);
 const stat=fs.stat;fs.stat=async file=>file===archive ? {isFile:()=>true,size:500*1024*1024+1} : stat(file);
 try {await assert.rejects(importProject(store,archive,'Too large'),/archive is too large/);} finally {fs.stat=stat;}
 assert.deepEqual(await store.list(),[]);
});

test('rejects a deflated entry whose compressed data is declared empty', async t => {
 const root: string = await fs.mkdtemp(path.join(os.tmpdir(), 'refract-archive-truncated-'));
 t.after(() => fs.rm(root, {recursive: true, force: true}));
 const zip = new Zip();
 zip.addFile('main.tex', Buffer.from('\\documentclass{article}'));
 zip.addFile('asset.bin', Buffer.alloc(1024, 42));
 const bytes: Buffer = zip.toBuffer();
 let offset = 0;
 while ((offset = bytes.indexOf(Buffer.from([0x50, 0x4b, 0x01, 0x02]), offset)) !== -1) {
  const nameLength = bytes.readUInt16LE(offset + 28);
  const name = bytes.subarray(offset + 46, offset + 46 + nameLength).toString();
  if (name === 'asset.bin') { bytes.writeUInt32LE(0, offset + 20); break; }
  offset += 46 + nameLength + bytes.readUInt16LE(offset + 30) + bytes.readUInt16LE(offset + 32);
 }
 assert.notEqual(offset, -1);
 const archive = path.join(root, 'bad.zip'); await fs.writeFile(archive, bytes);
 const store = new ProjectStore(path.join(root, 'projects'));
 await assert.rejects(importProject(store, archive, 'Bad'), /Invalid file data/);
 assert.deepEqual(await store.list(), []);
});

test('a failed extraction removes its partial project without affecting existing projects',async t=>{
 const root:string=await fs.mkdtemp(path.join(os.tmpdir(),'refract-archive-cleanup-'));t.after(()=>fs.rm(root,{recursive:true,force:true}));
 const store=new ProjectStore(path.join(root,'projects')),existing=await store.create('Existing');
 const zip=new Zip();zip.addFile('main.tex',Buffer.from('\\documentclass{article}'));zip.addFile('folder',Buffer.from('not a directory'));zip.addFile('folder/child.txt',Buffer.from('child'));
 const archive=path.join(root,'conflict.zip');zip.writeZip(archive);
 await assert.rejects(importProject(store,archive,'Bad'));
 assert.deepEqual((await store.list()).map(project=>project.id),[existing.id]);
 assert.match(await store.read(existing.id,'main.tex'),/documentclass/);
 assert.deepEqual((await fs.readdir(store.root)).sort(),[existing.id]);
});

test('imports and compiles a project with a non-default root filename', async t => {
 const root = await fs.mkdtemp(path.join(os.tmpdir(),'refract-root-')); t.after(()=>fs.rm(root,{recursive:true,force:true}));
 const zip = new Zip(); zip.addFile('paper.tex',Buffer.from('\\documentclass{article}\n\\begin{document}\nHello\n\\end{document}\n'));
 zip.addFile('chapter.tex',Buffer.from('Chapter')); const archive = path.join(root,'paper.zip'); zip.writeZip(archive);
 const store = new ProjectStore(path.join(root,'projects')), project = await importProject(store,archive,'Paper');
 assert.equal(project.rootFile,'paper.tex'); assert.equal((await store.metadata(project.id)).rootFile,'paper.tex');
 const exe = path.join(process.env.REFRACT_TEX_BIN || 'missing-tex','pdflatex.exe');
 try { await fs.access(exe); } catch { return t.skip('External TeX is not configured'); }
 const {compile} = require('../src/compiler.cjs'); const result = await compile(store.projectDir(project.id),{rootFile:project.rootFile});
 assert.equal(result.ok,true,result.log); assert.equal(Buffer.from(result.pdf,'base64').subarray(0,5).toString(),'%PDF-');
});

test('wrapped archives retain empty folders without importing wrapper', async t => {
 const root = await fs.mkdtemp(path.join(os.tmpdir(),'refract-folders-')); t.after(()=>fs.rm(root,{recursive:true,force:true}));
 const zip = new Zip(); zip.addFile('project/main.tex',Buffer.from('\\documentclass{article}')); zip.addFile('project/empty/nested/',Buffer.alloc(0));
 const archive=path.join(root,'project.zip'); zip.writeZip(archive);
 const store=new ProjectStore(path.join(root,'projects')); const project=await importProject(store,archive,'Paper');
 assert.equal((await fs.stat(store.filePath(project.id,'empty/nested'))).isDirectory(),true);
 assert.equal(await store.read(project.id,'main.tex'),'\\documentclass{article}');
});

test('multiple root documents can be selected or cancelled safely', async t => {
 const root=await fs.mkdtemp(path.join(os.tmpdir(),'refract-roots-')); t.after(()=>fs.rm(root,{recursive:true,force:true}));
 const zip=new Zip(); zip.addFile('main.tex',Buffer.from('\\documentclass{article}')); zip.addFile('other.tex',Buffer.from('\\documentclass{book}'));
 const archive=path.join(root,'project.zip'); zip.writeZip(archive);
 const store=new ProjectStore(path.join(root,'projects')); await store.init();
 assert.equal(await importProject(store,archive,'Cancel',async () => null),null);
 assert.deepEqual(await store.list(),[]);
 await assert.rejects(importProject(store,archive,'Invalid',async () => '../outside.tex'),/Invalid LaTeX root selection/);
 const project=await importProject(store,archive,'Selected',async choices => { assert.deepEqual(choices.sort(),['main.tex','other.tex']); return 'other.tex'; });
 assert.equal((await store.metadata(project.id)).rootFile,'other.tex');
});


test('imports each project folder independently, including nested and empty folders', async t => {
 const root=await fs.mkdtemp(path.join(os.tmpdir(),'refract-all-')); t.after(()=>fs.rm(root,{recursive:true,force:true}));
 const zip=new Zip();
 zip.addFile('bundle/One/main.tex',Buffer.from('\\documentclass{article}'));
 zip.addFile('bundle/One/assets/image.png',Buffer.from([1,2,3]));
 zip.addFile('bundle/One/empty/',Buffer.alloc(0));
 zip.addFile('bundle/Two/nested/paper.tex',Buffer.from('\\documentclass{book}'));
 zip.addFile('bundle/Two/other.txt',Buffer.from('two'));
 const archive=path.join(root,'project.zip'); zip.writeZip(archive);
 const store=new ProjectStore(path.join(root,'projects'));
 await importProject(store,archive,'Bundle',async choices => choices);
 const projects=await store.list(); assert.equal(projects.length,2);
 const one=projects.find(p=>p.name==='One'), two=projects.find(p=>p.name==='Two');
 assert.equal(one.rootFile,'main.tex'); assert.equal(two.rootFile,'nested/paper.tex');
 assert.deepEqual((await store.files(one.id)).sort(),['assets/image.png','main.tex']);
 assert.deepEqual((await store.files(two.id)).sort(),['nested/paper.tex','other.txt']);
 assert.equal((await fs.stat(store.filePath(one.id,'empty'))).isDirectory(),true);
 const single=await importProject(store,archive,'Bundle',async () => 'Two/nested/paper.tex');
 assert.deepEqual((await store.files(single.id)).sort(),['nested/paper.tex','other.txt']);
});

test('rejects importing all roots when project boundaries are ambiguous', async t => {
 const root=await fs.mkdtemp(path.join(os.tmpdir(),'refract-ambiguous-')); t.after(()=>fs.rm(root,{recursive:true,force:true}));
 const zip=new Zip(); zip.addFile('one.tex',Buffer.from('\\documentclass{article}')); zip.addFile('two.tex',Buffer.from('\\documentclass{book}'));
 const archive=path.join(root,'project.zip'); zip.writeZip(archive);
 const store=new ProjectStore(path.join(root,'projects')); await store.init();
 await assert.rejects(importProject(store,archive,'Bundle',async choices=>choices),/Cannot separate projects/);
 assert.deepEqual(await store.list(),[]);
});
