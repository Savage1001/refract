const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const os = require('node:os');
const path = require('node:path');
const { ProjectStore } = require('../src/project-store.cjs');
test('projects persist and paths cannot escape their project', async t => {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), 'refract-'));
  t.after(() => fs.rm(root, { recursive: true, force: true }));
  const store = new ProjectStore(root); await store.init();
  const p = await store.create('Paper');
  assert.equal((await store.list())[0].name, 'Paper');
  assert.match(await store.read(p.id, 'main.tex'), /\\documentclass/);
  await store.write(p.id, 'chapters/a.tex', 'Hello');
  assert.equal(await store.read(p.id, 'chapters/a.tex'), 'Hello');
  await assert.rejects(store.write(p.id, '../outside', 'bad'));
});

test('failed project rename preserves metadata and removes its temporary file', async t => {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), 'refract-rename-'));
  t.after(() => fs.rm(root, {recursive:true, force:true}));
  const store = new ProjectStore(root), project = await store.create('Paper');
  const metadata = path.join(store.projectDir(project.id), '.refract.json'), rename = fs.rename;
  fs.rename = async (from, to) => { if (to === metadata) throw new Error('simulated rename failure'); return rename(from, to); };
  try { await assert.rejects(store.renameProject(project.id, 'Revised'), /simulated rename failure/); }
  finally { fs.rename = rename; }
  assert.equal((await store.metadata(project.id)).name, 'Paper');
  assert.deepEqual((await fs.readdir(store.projectDir(project.id))).sort(), ['.refract.json', 'main.tex']);
  assert.equal((await store.renameProject(project.id, 'Revised')).name, 'Revised');
});

test('project listing survives files and folders becoming inaccessible during edit scans', async t => {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), 'refract-list-'));
  t.after(() => fs.rm(root, {recursive:true, force:true}));
  const store = new ProjectStore(root), project = await store.create('Paper');
  const dir = store.projectDir(project.id), blocked = path.join(dir, 'blocked'), missing = path.join(dir, 'gone.tex');
  await fs.mkdir(blocked); await fs.writeFile(missing, 'content');
  const readdir = fs.readdir, lstat = fs.lstat;
  fs.readdir = async (...args) => { if (args[0] === blocked) throw new Error('Access denied'); return readdir(...args); };
  fs.lstat = async (...args) => { if (args[0] === missing) throw new Error('File disappeared'); return lstat(...args); };
  try {
    const listed = await store.list();
    assert.equal(listed.length, 1);
    assert.equal(listed[0].id, project.id);
    assert.ok(listed[0].edited);
  } finally { fs.readdir = readdir; fs.lstat = lstat; }
});

test('imported project assets retain bytes and never replace existing files', async t => {
 const root = await fs.mkdtemp(path.join(os.tmpdir(),'refract-files-')); t.after(()=>fs.rm(root,{recursive:true,force:true}));
 const store = new ProjectStore(path.join(root,'projects')); const project = await store.create('Paper');
 const asset = path.join(root,'figure.png'), data = Buffer.from([0,255,42,1]); await fs.writeFile(asset,data);
 assert.deepEqual(await store.importFiles(project.id,[asset]),['figure.png']);
 assert.deepEqual(await fs.readFile(store.filePath(project.id,'figure.png')),data);
 await assert.rejects(store.importFiles(project.id,[asset]),{code:'EEXIST'});
 assert.deepEqual(await fs.readFile(store.filePath(project.id,'figure.png')),data);
});

test('folders appear in the tree even before they contain files', async t => {
 const root = await fs.mkdtemp(path.join(os.tmpdir(),'refract-tree-')); t.after(()=>fs.rm(root,{recursive:true,force:true}));
 const store = new ProjectStore(root), project = await store.create('Paper');
 await store.createFolder(project.id,'figures');
 assert.ok((await store.tree(project.id)).some(node => node.path === 'figures' && node.children.length === 0));
 await store.createFile(project.id,'figures/caption.tex');
 assert.equal((await store.tree(project.id)).find(node => node.path === 'figures').children[0].path,'figures/caption.tex');
});

test('folder upload copies nested data and does not overwrite existing folders', async t => {
 const root = await fs.mkdtemp(path.join(os.tmpdir(),'refract-folder-')); t.after(()=>fs.rm(root,{recursive:true,force:true}));
 const store = new ProjectStore(path.join(root,'projects')), project = await store.create('Paper');
 const source = path.join(root,'figures'); await fs.mkdir(path.join(source,'nested'),{recursive:true});
 const content = Buffer.from([0,255,7]); await fs.writeFile(path.join(source,'nested','figure.png'),content);
 await store.importFolder(project.id,source);
 assert.deepEqual(await fs.readFile(store.filePath(project.id,'figures/nested/figure.png')),content);
 await assert.rejects(store.importFolder(project.id,source),{code:'EEXIST'});
 assert.deepEqual(await fs.readFile(store.filePath(project.id,'figures/nested/figure.png')),content);
 await assert.rejects(store.importFolder(project.id,store.projectDir(project.id)),/outside this project/);
});

test('file rename preserves root selection and never overwrites; delete keeps a recovery copy', async t => {
 const root = await fs.mkdtemp(path.join(os.tmpdir(),'refract-file-actions-')); t.after(()=>fs.rm(root,{recursive:true,force:true}));
 const store = new ProjectStore(root), p = await store.create('Paper');
 const original = await store.read(p.id,'main.tex'); await store.createFile(p.id,'other.tex');
 await assert.rejects(store.renameFile(p.id,'main.tex','other.tex'),{code:'EEXIST'});
 assert.equal(await store.read(p.id,'main.tex'),original);
 const meta = await store.renameFile(p.id,'main.tex','paper.tex');
 assert.equal(meta.rootFile,'paper.tex'); assert.equal((await store.metadata(p.id)).rootFile,'paper.tex');
 await store.trashFile(p.id,'paper.tex'); await assert.rejects(store.read(p.id,'paper.tex'),{code:'ENOENT'});
 const trash = await fs.readdir(path.join(store.projectDir(p.id),'.refract-trash'));
 assert.equal(await fs.readFile(path.join(store.projectDir(p.id),'.refract-trash',trash.find(name=>!name.endsWith('.json'))),'utf8'),original);
});

test('deleted files are recoverable without overwriting another file',async t=>{
 const root=await fs.mkdtemp(path.join(os.tmpdir(),'refract-trash-'));t.after(()=>fs.rm(root,{recursive:true,force:true}));
 const store=new ProjectStore(root),p=await store.create('Paper');
 await store.write(p.id,'nested/asset.dat',Buffer.from([0,255,3])); await store.trashFile(p.id,'nested/asset.dat');
 const [item]=await new ProjectStore(root).listTrash(p.id); assert.equal(item.name,'nested/asset.dat');
 await store.write(p.id,'nested/asset.dat','new data'); await assert.rejects(store.restoreTrash(p.id,item.id),{code:'EEXIST'});
 assert.equal(await store.read(p.id,'nested/asset.dat'),'new data'); assert.equal((await store.listTrash(p.id)).length,1);
 await fs.unlink(store.filePath(p.id,'nested/asset.dat')); await fs.rmdir(store.filePath(p.id,'nested'));
 assert.equal(await store.restoreTrash(p.id,item.id),'nested/asset.dat');
 assert.deepEqual(await fs.readFile(store.filePath(p.id,'nested/asset.dat')),Buffer.from([0,255,3]));
 assert.deepEqual(await store.listTrash(p.id),[]); await assert.rejects(store.restoreTrash(p.id,'../bad'));
});

test('duplicate preserves project files and root metadata independently', async t => {
 const root = await fs.mkdtemp(path.join(os.tmpdir(),'refract-copy-')); t.after(()=>fs.rm(root,{recursive:true,force:true}));
 const store = new ProjectStore(root), p = await store.create('Paper');
 await store.renameFile(p.id,'main.tex','paper.tex'); await store.createFolder(p.id,'figures');
 await store.write(p.id,'figures/plot.tex','Original');
 const copy = await store.duplicate(p.id);
 assert.notEqual(copy.id,p.id); assert.equal(copy.rootFile,'paper.tex');
 assert.equal(await store.read(copy.id,'figures/plot.tex'),'Original');
 await store.write(copy.id,'figures/plot.tex','Copy'); assert.equal(await store.read(p.id,'figures/plot.tex'),'Original');
});

test('project search finds matching lines and skips binary assets', async t => {
 const root = await fs.mkdtemp(path.join(os.tmpdir(),'refract-search-')); t.after(()=>fs.rm(root,{recursive:true,force:true}));
 const store = new ProjectStore(root), p = await store.create('Paper');
 await store.write(p.id,'chapter.tex','First line\nImportant finding\nLast line');
 await fs.writeFile(store.filePath(p.id,'binary.dat'), Buffer.from([0,73,77,80]));
 const matches = await store.search(p.id,'FINDING');
 assert.deepEqual(matches.map(({file,line,column})=>({file,line,column})),[{file:'chapter.tex',line:2,column:10}]);
 assert.deepEqual(await store.search(p.id,'IMP'),[{file:'chapter.tex',line:2,column:0,text:'Important finding'}]);
 await store.write(p.id,'unicode.tex','İ A.b axb\n𐐀 FINDING');
 await fs.writeFile(store.filePath(p.id,'invalid.txt'),Buffer.from([0xff,0x46,0x49,0x4e,0x44]));
 assert.deepEqual(await store.search(p.id,'a.b'),[{file:'unicode.tex',line:1,column:2,text:'İ A.b axb'}]);
 assert.deepEqual((await store.search(p.id,'finding')).find(item=>item.file==='unicode.tex'),{file:'unicode.tex',line:2,column:3,text:'𐐀 FINDING'});
 assert.equal((await store.search(p.id,'find')).some(item=>item.file==='invalid.txt'),false);
});

test('a failed rename preserves the original file and root metadata',async t=>{
 const root=await fs.mkdtemp(path.join(os.tmpdir(),'refract-rename-rollback-'));t.after(()=>fs.rm(root,{recursive:true,force:true}));
 const store=new ProjectStore(root),p=await store.create('Paper'),original=await store.read(p.id,'main.tex');
 const originalUnlink=fs.unlink, source=store.filePath(p.id,'main.tex'); let injected=false;
 fs.unlink=async file=>{if(file===source && !injected){injected=true;throw Object.assign(new Error('simulated file lock'),{code:'EBUSY'});}return originalUnlink(file);};
 try {await assert.rejects(store.renameFile(p.id,'main.tex','paper.tex'),/simulated file lock/);} finally {fs.unlink=originalUnlink;}
 assert.equal(await store.read(p.id,'main.tex'),original);
 await assert.rejects(fs.access(store.filePath(p.id,'paper.tex')),{code:'ENOENT'});
 assert.equal((await store.metadata(p.id)).rootFile,undefined);
});

test('duplicated comments and history are independent while sessions and trash stay private',async t=>{
 const root=await fs.mkdtemp(path.join(os.tmpdir(),'refract-copy-history-'));t.after(()=>fs.rm(root,{recursive:true,force:true}));
 const {Comments}=require('../src/comments.cjs'),{Versions}=require('../src/versions.cjs');
 const store=new ProjectStore(root),p=await store.create('Paper'),comments=new Comments(store),versions=new Versions(store);
 const item=await comments.add(p.id,'main.tex','Introduction','Review');await comments.reply(p.id,item.id,'Reply');
 const snapshot=await versions.snapshot(p.id);await store.write(p.id,'main.tex','Changed');
 await fs.writeFile(path.join(store.projectDir(p.id),'.refract-chat.json'),'private');
 await store.createFile(p.id,'deleted.txt');await store.trashFile(p.id,'deleted.txt');
 const copy=await store.duplicate(p.id);
 assert.equal((await comments.list(copy.id))[0].replies[0].text,'Reply');
 assert.equal((await versions.list(copy.id))[0].id,snapshot.id);
 assert.match((await versions.readFile(copy.id,snapshot.id,'main.tex')).toString(),/Introduction/);
 await comments.resolve(copy.id,item.id,true);assert.equal((await comments.list(p.id))[0].resolved,false);
 await versions.snapshot(copy.id);assert.equal((await versions.list(p.id)).length,1);
 await assert.rejects(fs.access(path.join(store.projectDir(copy.id),'.refract-chat.json')));
 assert.deepEqual(await store.listTrash(copy.id),[]);
});

test('failed history copy removes the incomplete duplicate and permits retry',async t=>{
 const root=await fs.mkdtemp(path.join(os.tmpdir(),'refract-copy-failure-'));t.after(()=>fs.rm(root,{recursive:true,force:true}));
 const {Comments}=require('../src/comments.cjs'),{Versions}=require('../src/versions.cjs');
 const store=new ProjectStore(root),p=await store.create('Paper'),comments=new Comments(store),versions=new Versions(store);
 const item=await comments.add(p.id,'main.tex','Introduction','Review');const version=await versions.snapshot(p.id);
 const copyFile=fs.copyFile;let injected=false;
 fs.copyFile=async(from,to,...args)=>{if(String(from).endsWith(version.id+'.zip')){injected=true;throw new Error('simulated copy failure');}return copyFile(from,to,...args);};
 try {await assert.rejects(store.duplicate(p.id),/simulated copy failure/);} finally {fs.copyFile=copyFile;}
 assert.equal(injected,true);assert.deepEqual((await store.list()).map(project=>project.id),[p.id]);
 assert.equal((await new Comments(store).list(p.id))[0].id,item.id);
 assert.equal((await new Versions(store).list(p.id))[0].id,version.id);
 const copy=await store.duplicate(p.id);
 assert.equal((await comments.list(copy.id))[0].id,item.id);
 assert.equal((await versions.readFile(copy.id,version.id,'main.tex')).toString(),await store.read(p.id,'main.tex'));
});

test('LaTeX engine settings are validated, persistent and copied',async t=>{
 const root=await fs.mkdtemp(path.join(os.tmpdir(),'refract-engine-'));t.after(()=>fs.rm(root,{recursive:true,force:true}));
 const store=new ProjectStore(root),p=await store.create('Paper');
 assert.equal((await store.metadata(p.id)).engine,undefined);
 await store.setEngine(p.id,'xelatex');assert.equal((await new ProjectStore(root).metadata(p.id)).engine,'xelatex');
 await assert.rejects(store.setEngine(p.id,'../evil'),/Unsupported LaTeX engine/);
 assert.equal((await store.metadata(p.id)).engine,'xelatex');
 const copy=await store.duplicate(p.id);assert.equal(copy.engine,'xelatex');
 await store.setEngine(copy.id,'lualatex');assert.equal((await store.metadata(p.id)).engine,'xelatex');
});

test('nested destinations support creation and uploads without following links',async t=>{
 const root=await fs.mkdtemp(path.join(os.tmpdir(),'refract-destination-'));t.after(()=>fs.rm(root,{recursive:true,force:true}));
 const store=new ProjectStore(path.join(root,'projects')),p=await store.create('Paper');
 await store.createFolder(p.id,'chapters/parts');await store.createFile(p.id,'chapters/parts/notes.tex');
 const source=path.join(root,'image.png');await fs.writeFile(source,Buffer.from([0,1,255]));
 assert.deepEqual(await store.importFiles(p.id,[source],'chapters/parts'),['chapters/parts/image.png']);
 await assert.rejects(store.importFiles(p.id,[source],'chapters/parts'),{code:'EEXIST'});
 const other=path.join(root,'other');await fs.mkdir(other);await fs.writeFile(path.join(other,'first.txt'),'first');await fs.writeFile(path.join(other,'image.png'),'collision');
 await assert.rejects(store.importFiles(p.id,[path.join(other,'first.txt'),path.join(other,'image.png')],'chapters/parts'),/Upload stopped after copying chapters\/parts\/first.txt/);
 assert.equal(await store.read(p.id,'chapters/parts/first.txt'),'first');
 const folder=path.join(root,'figures');await fs.mkdir(folder);await fs.writeFile(path.join(folder,'plot.dat'),'data');
 assert.equal(await store.importFolder(p.id,folder,'chapters/parts'),'chapters/parts/figures');
 assert.equal(await store.read(p.id,'chapters/parts/figures/plot.dat'),'data');
 await assert.rejects(store.importFiles(p.id,[source],'../outside'));
 await assert.rejects(store.createFile(p.id,'chapters/.refract-hidden'));
 const external=path.join(root,'outside');await fs.mkdir(external);
 try {await fs.symlink(external,store.filePath(p.id,'linked'),process.platform==='win32'?'junction':'dir');}
 catch(error) {if(!['EPERM','EACCES'].includes(error.code)) throw error;return;}
 await assert.rejects(store.importFiles(p.id,[source],'linked'),/Invalid destination/);
 await assert.rejects(store.createFile(p.id,'linked/new.txt'),/Invalid destination/);
 await assert.rejects(fs.access(path.join(external,'new.txt')));
});

test('file reads and renames reject linked project folders',async t=>{
 const root: string=await fs.mkdtemp(path.join(os.tmpdir(),'refract-file-link-'));t.after(()=>fs.rm(root,{recursive:true,force:true}));
 const store=new ProjectStore(path.join(root,'projects')),p=await store.create('Paper'),outside=path.join(root,'outside');
 await fs.mkdir(outside);await fs.writeFile(path.join(outside,'secret.txt'),'outside');
 try {await fs.symlink(outside,store.filePath(p.id,'linked'),process.platform==='win32'?'junction':'dir');}
 catch(error) {if(['EPERM','EACCES'].includes(error.code))return t.skip('Creating directory links is not permitted');throw error;}
 await assert.rejects(store.read(p.id,'linked/secret.txt'),/Invalid destination/);
 await assert.rejects(store.existingFile(p.id,'linked/secret.txt'),/Invalid destination/);
 await assert.rejects(store.renameFile(p.id,'linked/secret.txt','moved.txt'),/Invalid destination/);
 await assert.rejects(store.renameFile(p.id,'main.tex','linked/moved.tex'),/Invalid destination/);
 assert.equal(await fs.readFile(path.join(outside,'secret.txt'),'utf8'),'outside');
 await assert.rejects(fs.access(path.join(outside,'moved.tex')));
 assert.match(await store.read(p.id,'main.tex'),/documentclass/);
});

test('folder moves preserve nested data, hidden files and the root document',async t=>{
 const root=await fs.mkdtemp(path.join(os.tmpdir(),'refract-move-'));t.after(()=>fs.rm(root,{recursive:true,force:true}));
 const store=new ProjectStore(root),p=await store.create('Paper');await store.createFolder(p.id,'chapters/deep');
 await store.renameFile(p.id,'main.tex','chapters/deep/main.tex');await fs.writeFile(store.filePath(p.id,'chapters/deep/image.bin'),Buffer.from([0,255,7]));
 await fs.writeFile(store.filePath(p.id,'chapters/.notes'),'hidden');await store.createFolder(p.id,'chapters/empty');
 await assert.rejects(store.moveFolder(p.id,'chapters','chapters/deep/inside'),/outside the folder/);
 await store.createFolder(p.id,'existing');await assert.rejects(store.moveFolder(p.id,'chapters','existing'),{code:'EEXIST'});
 const meta=await store.moveFolder(p.id,'chapters','drafts');assert.equal(meta.rootFile,'drafts/deep/main.tex');
 assert.deepEqual(await fs.readFile(store.filePath(p.id,'drafts/deep/image.bin')),Buffer.from([0,255,7]));
 assert.equal(await fs.readFile(store.filePath(p.id,'drafts/.notes'),'utf8'),'hidden');assert.equal((await fs.stat(store.filePath(p.id,'drafts/empty'))).isDirectory(),true);
 await assert.rejects(fs.access(store.filePath(p.id,'chapters')));
});

test('a failed folder move recovers files and root metadata',async t=>{
 const root=await fs.mkdtemp(path.join(os.tmpdir(),'refract-move-fail-'));t.after(()=>fs.rm(root,{recursive:true,force:true}));
 const store=new ProjectStore(root),p=await store.create('Paper');await store.createFolder(p.id,'old');await store.renameFile(p.id,'main.tex','old/main.tex');
 await store.write(p.id,'old/other.txt','keep');const target=store.filePath(p.id,'old/other.txt'), unlink=fs.unlink;let injected=false;
 fs.unlink=async name=>{if(name===target && !injected){injected=true;throw new Error('simulated lock');}return unlink(name);};
 try {await assert.rejects(store.moveFolder(p.id,'old','new'),/simulated lock/);} finally {fs.unlink=unlink;}
 assert.equal(await store.read(p.id,'old/other.txt'),'keep');assert.match(await store.read(p.id,'old/main.tex'),/documentclass/);
 assert.equal((await store.metadata(p.id)).rootFile,'old/main.tex');await assert.rejects(fs.access(store.filePath(p.id,'new')));
});

test('folder moves reject linked entries and linked destinations',async t=>{
 const root=await fs.mkdtemp(path.join(os.tmpdir(),'refract-move-link-'));t.after(()=>fs.rm(root,{recursive:true,force:true}));
 const store=new ProjectStore(path.join(root,'projects')),p=await store.create('Paper'),outside=path.join(root,'outside');await fs.mkdir(outside);await fs.writeFile(path.join(outside,'keep.txt'),'outside');
 await store.createFolder(p.id,'source');await store.write(p.id,'source/file.txt','inside');
 try {await fs.symlink(outside,store.filePath(p.id,'source/link'),process.platform==='win32'?'junction':'dir');await fs.symlink(outside,store.filePath(p.id,'linked'),process.platform==='win32'?'junction':'dir');}
 catch(error){if(['EPERM','EACCES'].includes(error.code))return t.skip('Creating directory links is not permitted');throw error;}
 await assert.rejects(store.moveFolder(p.id,'source','target'),/links cannot be moved/);
 await fs.unlink(store.filePath(p.id,'source/link'));
 await assert.rejects(store.moveFolder(p.id,'source','linked/target'),/Invalid destination/);
 assert.equal(await store.read(p.id,'source/file.txt'),'inside');assert.equal(await fs.readFile(path.join(outside,'keep.txt'),'utf8'),'outside');
});

test('folder move cleans up when destination preparation fails',async t=>{
 const root=await fs.mkdtemp(path.join(os.tmpdir(),'refract-move-prepare-'));t.after(()=>fs.rm(root,{recursive:true,force:true}));
 const store=new ProjectStore(root),p=await store.create('Paper');await store.createFolder(p.id,'old/deep');await store.write(p.id,'old/deep/file.txt','safe');
 const mkdir=fs.mkdir, failing=store.filePath(p.id,'new/deep');let injected=false;
 fs.mkdir=async (name,options)=>{if(name===failing && !injected){injected=true;throw new Error('simulated folder error');}return mkdir(name,options);};
 try {await assert.rejects(store.moveFolder(p.id,'old','new'),/simulated folder error/);} finally {fs.mkdir=mkdir;}
 assert.equal(await store.read(p.id,'old/deep/file.txt'),'safe');await assert.rejects(fs.access(store.filePath(p.id,'new')));
});

test('deleted folders restore nested, hidden and binary files without overwriting',async t=>{
 const root=await fs.mkdtemp(path.join(os.tmpdir(),'refract-trash-folder-'));t.after(()=>fs.rm(root,{recursive:true,force:true}));
 const store=new ProjectStore(root),p=await store.create('Paper');await store.createFolder(p.id,'parent/chapters/empty');
 await assert.rejects(store.trashFolder(p.id,''),/Invalid file path/);await assert.rejects(store.trashFolder(p.id,'../outside'));
 await store.renameFile(p.id,'main.tex','parent/chapters/main.tex');
 await fs.writeFile(store.filePath(p.id,'parent/chapters/.hidden'),Buffer.from([0,255,4]));
 await store.trashFolder(p.id,'parent/chapters');
 const [item]=await new ProjectStore(root).listTrash(p.id);assert.equal(item.kind,'folder');assert.equal(item.name,'parent/chapters');
 assert.equal((await store.metadata(p.id)).rootFile,'parent/chapters/main.tex');
 await store.createFolder(p.id,'parent/chapters');await assert.rejects(store.restoreTrash(p.id,item.id),{code:'EEXIST'});
 assert.equal((await store.listTrash(p.id)).length,1);await fs.rmdir(store.filePath(p.id,'parent/chapters'));await fs.rmdir(store.filePath(p.id,'parent'));
 assert.equal(await store.restoreTrash(p.id,item.id),'parent/chapters');
 assert.match(await store.read(p.id,'parent/chapters/main.tex'),/documentclass/);
 assert.deepEqual(await fs.readFile(store.filePath(p.id,'parent/chapters/.hidden')),Buffer.from([0,255,4]));
 assert.equal((await fs.stat(store.filePath(p.id,'parent/chapters/empty'))).isDirectory(),true);assert.deepEqual(await store.listTrash(p.id),[]);
});

test('a failed folder restoration leaves the deleted folder recoverable',async t=>{
 const root=await fs.mkdtemp(path.join(os.tmpdir(),'refract-trash-folder-fail-'));t.after(()=>fs.rm(root,{recursive:true,force:true}));
 const store=new ProjectStore(root),p=await store.create('Paper');await store.createFolder(p.id,'folder');await store.write(p.id,'folder/a.txt','a');await store.write(p.id,'folder/z.txt','z');
 await store.trashFolder(p.id,'folder');const [item]=await store.listTrash(p.id);
 const source=path.join(store.projectDir(p.id),'.refract-trash',item.id,'z.txt'),unlink=fs.unlink;let injected=false;
 fs.unlink=async file=>{if(file===source && !injected){injected=true;throw new Error('simulated lock');}return unlink(file);};
 try {await assert.rejects(store.restoreTrash(p.id,item.id),/simulated lock/);} finally {fs.unlink=unlink;}
 await assert.rejects(fs.access(store.filePath(p.id,'folder')));assert.equal((await store.listTrash(p.id)).length,1);
 await store.restoreTrash(p.id,item.id);assert.equal(await store.read(p.id,'folder/a.txt'),'a');assert.equal(await store.read(p.id,'folder/z.txt'),'z');
});

test('folder deletion refuses linked contents',async t=>{
 const root=await fs.mkdtemp(path.join(os.tmpdir(),'refract-trash-folder-link-'));t.after(()=>fs.rm(root,{recursive:true,force:true}));
 const store=new ProjectStore(path.join(root,'projects')),p=await store.create('Paper'),outside=path.join(root,'outside');await fs.mkdir(outside);await fs.writeFile(path.join(outside,'keep'),'safe');await store.createFolder(p.id,'folder');
 try {await fs.symlink(outside,store.filePath(p.id,'folder/link'),process.platform==='win32'?'junction':'dir');} catch(error) {if(['EPERM','EACCES'].includes(error.code))return t.skip('Creating directory links is not permitted');throw error;}
 await assert.rejects(store.trashFolder(p.id,'folder'),/links cannot be deleted/);assert.equal(await fs.readFile(path.join(outside,'keep'),'utf8'),'safe');assert.deepEqual(await store.listTrash(p.id),[]);
});

test('deleted projects retain private data and restore without overwriting',async t=>{
 const root=await fs.mkdtemp(path.join(os.tmpdir(),'refract-project-trash-'));t.after(()=>fs.rm(root,{recursive:true,force:true}));
 const {Comments}=require('../src/comments.cjs'),{Versions}=require('../src/versions.cjs');
 const store=new ProjectStore(root),p=await store.create('Paper'),comments=new Comments(store),versions=new Versions(store);
 const item=await comments.add(p.id,'main.tex','Introduction','Review');await comments.reply(p.id,item.id,'Reply');
 const version=await versions.snapshot(p.id);await store.setEngine(p.id,'xelatex');
 await fs.writeFile(path.join(store.projectDir(p.id),'.refract-chat.json'),'chat data');
 await store.write(p.id,'image.bin',Buffer.from([0,255,8]));await store.trashFile(p.id,'image.bin');
 await store.trashProject(p.id);assert.deepEqual(await store.list(),[]);
 const [record]=await new ProjectStore(root).listDeletedProjects();assert.equal(record.projectId,p.id);assert.equal(record.name,'Paper');
 const occupied=store.projectDir(p.id);await fs.mkdir(occupied);await fs.writeFile(path.join(occupied,'keep'),'other project');
 await assert.rejects(store.restoreProject(record.id),{code:'EEXIST'});assert.equal(await fs.readFile(path.join(occupied,'keep'),'utf8'),'other project');
 assert.equal((await store.listDeletedProjects()).length,1);await fs.unlink(path.join(occupied,'keep'));await fs.rmdir(occupied);
 const restored=await store.restoreProject(record.id);assert.equal(restored.id,p.id);assert.equal(restored.engine,'xelatex');
 assert.equal((await comments.list(p.id))[0].replies[0].text,'Reply');assert.equal((await versions.list(p.id))[0].id,version.id);
 assert.equal(await fs.readFile(path.join(store.projectDir(p.id),'.refract-chat.json'),'utf8'),'chat data');
 const [trashedFile]=await store.listTrash(p.id);await store.restoreTrash(p.id,trashedFile.id);
 assert.deepEqual(await fs.readFile(store.filePath(p.id,'image.bin')),Buffer.from([0,255,8]));
 assert.deepEqual(await store.listDeletedProjects(),[]);await assert.rejects(store.restoreProject('../bad'));
});

test('project deletion rejects linked project directories',async t=>{
 const root=await fs.mkdtemp(path.join(os.tmpdir(),'refract-project-link-'));t.after(()=>fs.rm(root,{recursive:true,force:true}));
 const store=new ProjectStore(path.join(root,'projects'));await store.init();const outside=path.join(root,'outside');await fs.mkdir(outside);await fs.writeFile(path.join(outside,'keep'),'safe');
 const id='aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa';try {await fs.symlink(outside,store.projectDir(id),process.platform==='win32'?'junction':'dir');} catch(error){if(['EPERM','EACCES'].includes(error.code))return t.skip('Creating directory links is not permitted');throw error;}
 await assert.rejects(store.trashProject(id),/Invalid project directory/);assert.equal(await fs.readFile(path.join(outside,'keep'),'utf8'),'safe');
});

test('failed project deletion leaves the project and its files intact',async t=>{
 const root=await fs.mkdtemp(path.join(os.tmpdir(),'refract-project-delete-fail-'));t.after(()=>fs.rm(root,{recursive:true,force:true}));
 const store=new ProjectStore(root),p=await store.create('Paper'),source=store.projectDir(p.id),rename=fs.rename;let injected=false;
 fs.rename=async (from,to)=>{if(from===source && !injected){injected=true;throw new Error('simulated project lock');}return rename(from,to);};
 try {await assert.rejects(store.trashProject(p.id),/simulated project lock/);} finally {fs.rename=rename;}
 assert.match(await store.read(p.id,'main.tex'),/documentclass/);assert.equal((await store.list())[0].id,p.id);assert.deepEqual(await store.listDeletedProjects(),[]);
});

test('writes cannot recreate a deleted project or follow linked folders',async t=>{
 const root=await fs.mkdtemp(path.join(os.tmpdir(),'refract-deleted-write-'));t.after(()=>fs.rm(root,{recursive:true,force:true}));
 const store=new ProjectStore(path.join(root,'projects')),p=await store.create('Paper');await store.trashProject(p.id);
 await assert.rejects(store.write(p.id,'main.tex','late edit'),{code:'ENOENT'});await assert.rejects(fs.access(store.projectDir(p.id)));
 const [item]=await store.listDeletedProjects();await store.restoreProject(item.id);assert.doesNotMatch(await store.read(p.id,'main.tex'),/late edit/);
 const outside=path.join(root,'outside');await fs.mkdir(outside);try{await fs.symlink(outside,store.filePath(p.id,'link'),process.platform==='win32'?'junction':'dir');}catch(error){if(['EPERM','EACCES'].includes(error.code))return t.skip('Creating directory links is not permitted');throw error;}
 await assert.rejects(store.write(p.id,'link/file.txt','unsafe'),/Invalid destination/);await assert.rejects(fs.access(path.join(outside,'file.txt')));
 await assert.rejects(store.write(p.id,'.refract.json','unsafe'),/Invalid file name/);
});

test('project deletion retries a transient Windows lock without losing data',async t=>{
 if(process.platform!=='win32') return t.skip('Windows lock behavior');
 const root=await fs.mkdtemp(path.join(os.tmpdir(),'refract-project-retry-'));t.after(()=>fs.rm(root,{recursive:true,force:true}));
 const store=new ProjectStore(root),p=await store.create('Paper'),source=store.projectDir(p.id),rename=fs.rename;let attempts=0;
 fs.rename=async (from,to)=>{if(from===source && attempts++===0)throw Object.assign(new Error('temporary lock'),{code:'EPERM'});return rename(from,to);};
 try {await store.trashProject(p.id);} finally {fs.rename=rename;}
 assert.ok(attempts>=2);const [item]=await store.listDeletedProjects();await store.restoreProject(item.id);assert.match(await store.read(p.id,'main.tex'),/documentclass/);
});

test('saving retries a temporary Windows lock and preserves data on permanent failure',async t=>{
 if(process.platform!=='win32') return t.skip('Windows lock behavior');
 const root:string=await fs.mkdtemp(path.join(os.tmpdir(),'refract-save-retry-'));
 t.after(async()=>{const resolved=await fs.realpath(root),parent=await fs.realpath(os.tmpdir());if(path.dirname(resolved).toLowerCase()!==parent.toLowerCase() || !path.basename(resolved).startsWith('refract-save-retry-'))throw new Error('Unsafe cleanup path');await fs.rm(resolved,{recursive:true,force:true});});
 const store=new ProjectStore(root),p=await store.create('Paper'),target=store.filePath(p.id,'main.tex'),rename=fs.rename;let attempts=0;
 fs.rename=async (from,to)=>{if(to===target && attempts++<2)throw Object.assign(new Error('temporary lock'),{code:'EPERM'});return rename(from,to);};
 try {await store.write(p.id,'main.tex','saved after retry');} finally {fs.rename=rename;}
 assert.equal(attempts,3);assert.equal(await store.read(p.id,'main.tex'),'saved after retry');
 fs.rename=async (from,to)=>{if(to===target)throw Object.assign(new Error('permanent lock'),{code:'EPERM'});return rename(from,to);};
 try {await assert.rejects(store.write(p.id,'main.tex','unsaved'),/permanent lock/);} finally {fs.rename=rename;}
 assert.equal(await store.read(p.id,'main.tex'),'saved after retry');assert.deepEqual((await fs.readdir(store.projectDir(p.id))).filter(name=>name.endsWith('.tmp')),[]);
});

test('saving does not overwrite a file replaced during a Windows lock',async t=>{
 if(process.platform!=='win32') return t.skip('Windows lock behavior');
 const root:string=await fs.mkdtemp(path.join(os.tmpdir(),'refract-save-replaced-'));
 t.after(async()=>{const resolved=await fs.realpath(root),parent=await fs.realpath(os.tmpdir());if(path.dirname(resolved).toLowerCase()!==parent.toLowerCase() || !path.basename(resolved).startsWith('refract-save-replaced-'))throw new Error('Unsafe cleanup path');await fs.rm(resolved,{recursive:true,force:true});});
 const store=new ProjectStore(root),p=await store.create('Paper'),target=store.filePath(p.id,'main.tex'),replacement=path.join(store.projectDir(p.id),'replacement.tmp'),rename=fs.rename;await fs.writeFile(replacement,'external change');
 fs.rename=async (from,to)=>{if(to===target){await rename(replacement,target);throw Object.assign(new Error('temporary lock'),{code:'EPERM'});}return rename(from,to);};
 try {await assert.rejects(store.write(p.id,'main.tex','pending edit'),/temporary lock/);} finally {fs.rename=rename;}
 assert.equal(await store.read(p.id,'main.tex'),'external change');
});

test('saving preserves a replacement of its temporary file during a lock',async t=>{
 if(process.platform!=='win32') return t.skip('Windows lock behavior');
 const root:string=await fs.mkdtemp(path.join(os.tmpdir(),'refract-save-temp-'));
 t.after(async()=>{const resolved=await fs.realpath(root),parent=await fs.realpath(os.tmpdir());if(path.dirname(resolved).toLowerCase()!==parent.toLowerCase() || !path.basename(resolved).startsWith('refract-save-temp-'))throw new Error('Unsafe cleanup path');await fs.rm(resolved,{recursive:true,force:true});});
 const store=new ProjectStore(root),p=await store.create('Paper'),target=store.filePath(p.id,'main.tex'),replacement=path.join(store.projectDir(p.id),'replacement.tmp'),rename=fs.rename;await fs.writeFile(replacement,'external temporary data');let staged='';
 fs.rename=async (from,to)=>{if(to===target){staged=from;await rename(replacement,from);throw Object.assign(new Error('temporary lock'),{code:'EPERM'});}return rename(from,to);};
 try {await assert.rejects(store.write(p.id,'main.tex','pending edit'),/temporary lock/);} finally {fs.rename=rename;}
 assert.equal(await fs.readFile(staged,'utf8'),'external temporary data');assert.match(await store.read(p.id,'main.tex'),/documentclass/);
});

test('saving does not overwrite an in-place edit made during a Windows lock',async t=>{
 if(process.platform!=='win32') return t.skip('Windows lock behavior');
 const root:string=await fs.mkdtemp(path.join(os.tmpdir(),'refract-save-edited-'));
 t.after(async()=>{const resolved=await fs.realpath(root),parent=await fs.realpath(os.tmpdir());if(path.dirname(resolved).toLowerCase()!==parent.toLowerCase() || !path.basename(resolved).startsWith('refract-save-edited-'))throw new Error('Unsafe cleanup path');await fs.rm(resolved,{recursive:true,force:true});});
 const store=new ProjectStore(root),p=await store.create('Paper'),target=store.filePath(p.id,'main.tex'),rename=fs.rename;
 fs.rename=async (from,to)=>{if(to===target){await fs.writeFile(target,'external edit');throw Object.assign(new Error('temporary lock'),{code:'EPERM'});}return rename(from,to);};
 try {await assert.rejects(store.write(p.id,'main.tex','pending edit'),/temporary lock/);} finally {fs.rename=rename;}
 assert.equal(await store.read(p.id,'main.tex'),'external edit');
});

test('deleted projects expire after 30 days and retain their deletion date before expiry',async t=>{
 const root=await fs.mkdtemp(path.join(os.tmpdir(),'refract-project-expiry-'));t.after(()=>fs.rm(root,{recursive:true,force:true}));
 const store=new ProjectStore(root), old=await store.create('Old'), recent=await store.create('Recent');
 await store.trashProject(old.id);await store.trashProject(recent.id);
 const items=await store.listDeletedProjects(), oldItem=items.find(item=>item.projectId===old.id), recentItem=items.find(item=>item.projectId===recent.id);
 const trash=path.join(root,'.refract-project-trash'), recordPath=path.join(trash,oldItem.id+'.json');
 const record=JSON.parse(await fs.readFile(recordPath,'utf8'));record.deleted=new Date(Date.now()-30*24*60*60*1000).toISOString();await fs.writeFile(recordPath,JSON.stringify(record));
 const remaining=await store.listDeletedProjects();assert.equal(remaining.length,1);assert.equal(remaining[0].deleted,recentItem.deleted);
 await assert.rejects(fs.access(path.join(trash,oldItem.id)),{code:'ENOENT'});await assert.rejects(fs.access(recordPath),{code:'ENOENT'});
 await assert.rejects(store.restoreProject(oldItem.id),/not found/);assert.equal((await store.restoreProject(recentItem.id)).id,recent.id);
});

test('duplication rejects hard-linked private data and removes the partial copy', async t => {
 const root = await fs.mkdtemp(path.join(os.tmpdir(), 'refract-duplicate-links-'));
 t.after(() => fs.rm(root, {recursive:true, force:true}));
 const store = new ProjectStore(root), project = await store.create('Paper');
 const outside = path.join(root, 'outside.json'); await fs.writeFile(outside, 'private');
 const comments = path.join(store.projectDir(project.id), '.refract-comments.json');
 await fs.link(outside, comments);
 await assert.rejects(store.duplicate(project.id), /unsupported link or file/);
 assert.equal(await fs.readFile(outside, 'utf8'), 'private');
 assert.deepEqual((await store.list()).map(item => item.id), [project.id]);
 await fs.unlink(comments);
 const history = path.join(store.projectDir(project.id), '.refract-versions'); await fs.mkdir(history);
 await fs.link(outside, path.join(history, '00000000-0000-0000-0000-000000000000.json'));
 await assert.rejects(store.duplicate(project.id), /unsupported link or file/);
 assert.deepEqual((await store.list()).map(item => item.id), [project.id]);
});
