const {test}=require('node:test'),assert=require('node:assert/strict'),fs=require('node:fs/promises'),path=require('node:path'),os=require('node:os');
const {ProjectStore}=require('../src/project-store.cjs');const {Versions}=require('../src/versions.cjs');
test('failed snapshot metadata write leaves history readable and permits retry',async t=>{
 const root=await fs.mkdtemp(path.join(os.tmpdir(),'refract-version-write-'));t.after(()=>fs.rm(root,{recursive:true,force:true}));
 const store=new ProjectStore(root),p=await store.create('Paper'),versions=new Versions(store);
 const first=await versions.snapshot(p.id);await store.write(p.id,'main.tex','Changed');
 const writeFile=fs.writeFile;
 try {
  fs.writeFile=async (file,...args)=>{
   if (String(file).endsWith('.json.tmp')) {await writeFile(file,'{',...args.slice(1));throw new Error('simulated interrupted write');}
   return writeFile(file,...args);
  };
  await assert.rejects(versions.snapshot(p.id),/simulated interrupted write/);
 } finally {fs.writeFile=writeFile;}
 assert.deepEqual((await versions.list(p.id)).map(v=>v.id),[first.id]);
 assert.deepEqual((await fs.readdir(versions.directory(p.id))).sort(),[first.id+'.json',first.id+'.zip'].sort());
 const second=await versions.snapshot(p.id);
 assert.equal((await versions.readFile(p.id,second.id,'main.tex')).toString(),'Changed');
});
test('versions preserve earlier files across edits and skip duplicates',async t=>{
 const root=await fs.mkdtemp(path.join(os.tmpdir(),'refract-version-'));t.after(()=>fs.rm(root,{recursive:true,force:true}));
 const store=new ProjectStore(root),p=await store.create('Paper'),versions=new Versions(store);
 const first=await versions.snapshot(p.id), original=await store.read(p.id,'main.tex');
 assert.equal((await versions.snapshot(p.id)).id,first.id);
 await store.write(p.id,'main.tex','Changed');const second=await versions.snapshot(p.id);
 assert.notEqual(second.id,first.id);assert.equal((await versions.list(p.id)).length,2);
 assert.equal((await versions.readFile(p.id,first.id,'main.tex')).toString(),original);
 await assert.rejects(versions.readFile(p.id,'../bad','main.tex'));
});

test('version history refuses linked directories and preserves their contents',async t=>{
 const root: string=await fs.mkdtemp(path.join(os.tmpdir(),'refract-version-link-'));
 t.after(async()=>{const resolved=await fs.realpath(root),parent=await fs.realpath(os.tmpdir());if(path.dirname(resolved).toLowerCase()!==parent.toLowerCase() || !path.basename(resolved).startsWith('refract-version-link-'))throw new Error('Unsafe cleanup path');await fs.rm(resolved,{recursive:true,force:true});});
 const store=new ProjectStore(path.join(root,'projects')),p=await store.create('Paper'),versions=new Versions(store);
 const outside=path.join(root,'outside'),link=versions.directory(p.id);await fs.mkdir(outside);await fs.writeFile(path.join(outside,'keep.txt'),'untouched');
 try {await fs.symlink(outside,link,process.platform==='win32'?'junction':'dir');} catch(error) {if(['EPERM','EACCES'].includes(error.code))return t.skip('Creating directory links is not permitted');throw error;}
 try {
  await assert.rejects(versions.list(p.id),/Invalid version history directory/);
  await assert.rejects(versions.snapshot(p.id),/Invalid version history directory/);
  assert.deepEqual(await fs.readdir(outside),['keep.txt']);assert.equal(await fs.readFile(path.join(outside,'keep.txt'),'utf8'),'untouched');
 } finally {await fs.unlink(link);}
});

test('version history refuses linked metadata and archives',async t=>{
 const root: string=await fs.mkdtemp(path.join(os.tmpdir(),'refract-version-hardlink-'));t.after(async()=>{const resolved=await fs.realpath(root),parent=await fs.realpath(os.tmpdir());if(path.dirname(resolved).toLowerCase()!==parent.toLowerCase() || !path.basename(resolved).startsWith('refract-version-hardlink-'))throw new Error('Unsafe cleanup path');await fs.rm(resolved,{recursive:true,force:true});});
 const store=new ProjectStore(path.join(root,'projects')),p=await store.create('Paper'),versions=new Versions(store),version=await versions.snapshot(p.id),dir=versions.directory(p.id);
 const metadata=path.join(dir,version.id+'.json'),archive=path.join(dir,version.id+'.zip'),outsideMetadata=path.join(root,'metadata.json'),outsideArchive=path.join(root,'archive.zip');
 await fs.link(metadata,outsideMetadata);
 await assert.rejects(versions.list(p.id),/Invalid version history file/);
 await fs.unlink(outsideMetadata);await fs.link(archive,outsideArchive);
 await assert.rejects(versions.readFile(p.id,version.id,'main.tex'),/Invalid version history file/);
 await assert.rejects(versions.restore(p.id,version.id),/Invalid version history file/);
 assert.ok((await fs.stat(outsideArchive)).size>0);
});

test('comparison and project restore preserve bytes, folders, root selection and recovery',async t=>{
 const root=await fs.mkdtemp(path.join(os.tmpdir(),'refract-restore-'));t.after(()=>fs.rm(root,{recursive:true,force:true}));
 const store=new ProjectStore(root),p=await store.create('Paper'),versions=new Versions(store);
 const project=store.projectDir(p.id), meta=path.join(project,'.refract.json');
 await fs.mkdir(path.join(project,'empty','nested'),{recursive:true});
 await fs.mkdir(path.join(project,'images')); await fs.writeFile(path.join(project,'images','figure.bin'),Buffer.from([0,255,9]));
 await store.write(p.id,'other.tex','Old text');
 const oldMeta=await store.metadata(p.id); oldMeta.rootFile='other.tex'; await fs.writeFile(meta,JSON.stringify(oldMeta));
 const first=await versions.snapshot(p.id);
 await fs.rm(path.join(project,'empty'),{recursive:true}); await fs.unlink(path.join(project,'images','figure.bin'));
 await store.write(p.id,'other.tex','New text'); await store.write(p.id,'added.tex','Added');
 const changedMeta=await store.metadata(p.id); changedMeta.rootFile='main.tex'; await fs.writeFile(meta,JSON.stringify(changedMeta));
 await fs.writeFile(path.join(project,'.private-data'),'keep');
 const changes=await versions.compare(p.id,first.id);
 assert.deepEqual(changes.filter(x=>!x.name.endsWith('/') && x.name!=='Root document').map(x=>[x.name,x.status]),[['added.tex','added'],['images/figure.bin','deleted'],['other.tex','changed']]);
 assert.equal(changes.find(x=>x.name==='other.tex').previous,'Old text');
 assert.equal(changes.find(x=>x.name==='Root document').previous,'other.tex');
 assert.ok(changes.some(x=>x.name==='empty/nested/' && x.status==='folder deleted'));
 const result=await versions.restore(p.id,first.id);
 assert.equal(await store.read(p.id,'other.tex'),'Old text');
 assert.deepEqual(await fs.readFile(path.join(project,'images','figure.bin')),Buffer.from([0,255,9]));
 assert.equal((await fs.stat(path.join(project,'empty','nested'))).isDirectory(),true);
 await assert.rejects(fs.access(path.join(project,'added.tex')));
 assert.equal((await store.metadata(p.id)).rootFile,'other.tex');
 assert.equal(await fs.readFile(path.join(project,'.private-data'),'utf8'),'keep');
 await versions.restore(p.id,result.recoveryId);
 assert.equal(await store.read(p.id,'other.tex'),'New text');
 assert.equal(await store.read(p.id,'added.tex'),'Added');
 assert.equal((await store.metadata(p.id)).rootFile,'main.tex');
});

test('larger multi-folder snapshots restore and recover all file bytes', {timeout:180000}, async t=>{
 const root:string=await fs.mkdtemp(path.join(os.tmpdir(),'refract-large-version-'));
 t.after(async()=>{const resolved=await fs.realpath(root),parent=await fs.realpath(os.tmpdir());if(path.dirname(resolved).toLowerCase()!==parent.toLowerCase() || !path.basename(resolved).startsWith('refract-large-version-'))throw new Error('Unsafe cleanup path');await fs.rm(resolved,{recursive:true,force:true});});
 const store=new ProjectStore(root),p=await store.create('Paper'),versions=new Versions(store),project=store.projectDir(p.id),crypto=require('node:crypto');
 const originals=new Map<string,Buffer>();
 for(let folder=0;folder<50;folder++) {
  await fs.mkdir(path.join(project,`chapter-${folder}`,'empty'),{recursive:true});
  for(let file=0;file<20;file++) {const name=`chapter-${folder}/asset-${file}.bin`,bytes=crypto.randomBytes(16384);originals.set(name,bytes);await fs.writeFile(path.join(project,...name.split('/')),bytes);}
 }
 const old=await versions.snapshot(p.id);
 for(let folder=0;folder<50;folder++) {
  for(let file=0;file<10;file++) await fs.writeFile(path.join(project,`chapter-${folder}`,`asset-${file}.bin`),Buffer.from(`changed ${folder}:${file}`));
  for(let file=10;file<15;file++) await fs.unlink(path.join(project,`chapter-${folder}`,`asset-${file}.bin`));
 }
 const result=await versions.restore(p.id,old.id);
 for(const [name,bytes] of originals) assert.deepEqual(await fs.readFile(path.join(project,...name.split('/'))),bytes,name);
 for(let folder=0;folder<50;folder++) assert.equal((await fs.stat(path.join(project,`chapter-${folder}`,'empty'))).isDirectory(),true);
 await versions.restore(p.id,result.recoveryId);
 for(let folder=0;folder<50;folder++) {
  for(let file=0;file<10;file++) assert.equal(await fs.readFile(path.join(project,`chapter-${folder}`,`asset-${file}.bin`),'utf8'),`changed ${folder}:${file}`);
  for(let file=10;file<15;file++) await assert.rejects(fs.access(path.join(project,`chapter-${folder}`,`asset-${file}.bin`)),{code:'ENOENT'});
  for(let file=15;file<20;file++) assert.deepEqual(await fs.readFile(path.join(project,`chapter-${folder}`,`asset-${file}.bin`)),originals.get(`chapter-${folder}/asset-${file}.bin`));
 }
});

test('restoration handles file-folder changes and refuses to remove hidden data',async t=>{
 const root=await fs.mkdtemp(path.join(os.tmpdir(),'refract-conflicts-'));t.after(()=>fs.rm(root,{recursive:true,force:true}));
 const store=new ProjectStore(root),p=await store.create('Paper'),versions=new Versions(store),project=store.projectDir(p.id);
 await store.write(p.id,'item','original'); const original=await versions.snapshot(p.id);
 await fs.unlink(path.join(project,'item')); await fs.mkdir(path.join(project,'item')); await fs.writeFile(path.join(project,'item','nested'),'new');
 const later=await versions.snapshot(p.id);
 await versions.restore(p.id,original.id); assert.equal(await store.read(p.id,'item'),'original');
 await versions.restore(p.id,later.id); assert.equal(await store.read(p.id,'item/nested'),'new');
 await fs.writeFile(path.join(project,'item','.keep'),'private');
 await assert.rejects(versions.restore(p.id,original.id),/Hidden data or a link/);
 assert.equal(await fs.readFile(path.join(project,'item','.keep'),'utf8'),'private');
 assert.equal(await store.read(p.id,'item/nested'),'new');
});

test('older snapshots without folder or root metadata remain restorable',async t=>{
 const root=await fs.mkdtemp(path.join(os.tmpdir(),'refract-oldversion-'));t.after(()=>fs.rm(root,{recursive:true,force:true}));
 const store=new ProjectStore(root),p=await store.create('Paper'),versions=new Versions(store);
 const version=await versions.snapshot(p.id), file=path.join(versions.directory(p.id),version.id+'.json');
 delete version.directories; delete version.rootFile; await fs.writeFile(file,JSON.stringify(version));
 await store.write(p.id,'main.tex','changed'); await fs.mkdir(path.join(store.projectDir(p.id),'keep-empty'));
 await versions.restore(p.id,version.id);
 assert.match(await store.read(p.id,'main.tex'),/documentclass/);
 assert.equal((await fs.stat(path.join(store.projectDir(p.id),'keep-empty'))).isDirectory(),true);
});

test('a failed restore rolls back earlier writes and retains the recovery snapshot',async t=>{
 const root=await fs.mkdtemp(path.join(os.tmpdir(),'refract-rollback-'));t.after(()=>fs.rm(root,{recursive:true,force:true}));
 const store=new ProjectStore(root),p=await store.create('Paper'),versions=new Versions(store),project=store.projectDir(p.id);
 await store.write(p.id,'a.txt','old a'); await store.write(p.id,'z.txt','old z'); const original=await versions.snapshot(p.id);
 await store.write(p.id,'a.txt','new a'); await store.write(p.id,'z.txt','new z');
 const rename=fs.rename; let injected=false;
 fs.rename=async (from,to)=>{ if (!injected && to===path.join(project,'z.txt')) { injected=true; throw new Error('simulated write failure'); } return rename(from,to); };
 try { await assert.rejects(versions.restore(p.id,original.id),/previous project was recovered/); } finally { fs.rename=rename; }
 assert.equal(await store.read(p.id,'a.txt'),'new a'); assert.equal(await store.read(p.id,'z.txt'),'new z');
 assert.ok((await versions.list(p.id)).length>=2);
});

test('restore cleanup retries a transient file lock after checking the stage again',async t=>{
 const root:string=await fs.mkdtemp(path.join(os.tmpdir(),'refract-stage-retry-'));
 t.after(async()=>{const resolved=await fs.realpath(root),parent=await fs.realpath(os.tmpdir());if(path.dirname(resolved).toLowerCase()!==parent.toLowerCase() || !path.basename(resolved).startsWith('refract-stage-retry-'))throw new Error('Unsafe cleanup path');await fs.rm(resolved,{recursive:true,force:true});});
 const store=new ProjectStore(root),p=await store.create('Paper'),versions=new Versions(store),project=store.projectDir(p.id);
 await store.write(p.id,'item','old file');const old=await versions.snapshot(p.id);
 await fs.unlink(path.join(project,'item'));await store.write(p.id,'item/nested.txt','new nested');
 const unlink=fs.unlink;let failed=false;
 fs.unlink=async file=>{if(!failed && path.basename(file)==='nested.txt' && file.includes(`${path.sep}.refract-restore-`)){failed=true;throw Object.assign(new Error('temporary lock'),{code:'EPERM'});}return unlink(file);};
 let result;try {result=await versions.restore(p.id,old.id);} finally {fs.unlink=unlink;}
 assert.equal(failed,true);assert.equal(await store.read(p.id,'item'),'old file');
 assert.equal(result.cleanupPending,0);
 assert.deepEqual((await fs.readdir(project)).filter(name=>/^\.refract-restore-[0-9a-f-]{36}$/.test(name)),[]);
});

test('retained restore stages are retried after reopening',async t=>{
 const root:string=await fs.mkdtemp(path.join(os.tmpdir(),'refract-stage-later-'));
 t.after(()=>fs.rm(root,{recursive:true,force:true}));
 const store=new ProjectStore(root),p=await store.create('Paper'),versions=new Versions(store),project=store.projectDir(p.id);
 await store.write(p.id,'item','old file');const old=await versions.snapshot(p.id);
 await fs.unlink(path.join(project,'item'));await store.write(p.id,'item/nested.txt','new nested');
 const unlink=fs.unlink;
 fs.unlink=async file=>{if(path.basename(file)==='nested.txt' && file.includes(`${path.sep}.refract-restore-`))throw Object.assign(new Error('locked'),{code:'EPERM'});return unlink(file);};
 let result;try {result=await versions.restore(p.id,old.id);} finally {fs.unlink=unlink;}
 assert.equal(result.cleanupPending,1);
 assert.equal(await new Versions(store).retainedStages(p.id),0);
 assert.equal(await store.read(p.id,'item'),'old file');
 assert.equal((await fs.readdir(project)).filter(name=>name.startsWith('.refract-cleanup-')).length,0);
});

test('restore cleanup leaves staged data changed before a retry',async t=>{
 const root:string=await fs.mkdtemp(path.join(os.tmpdir(),'refract-stage-recheck-'));
 t.after(async()=>{const resolved=await fs.realpath(root),parent=await fs.realpath(os.tmpdir());if(path.dirname(resolved).toLowerCase()!==parent.toLowerCase() || !path.basename(resolved).startsWith('refract-stage-recheck-'))throw new Error('Unsafe cleanup path');await fs.rm(resolved,{recursive:true,force:true});});
 const store=new ProjectStore(root),p=await store.create('Paper'),versions=new Versions(store),project=store.projectDir(p.id);
 await store.write(p.id,'item','old file');const old=await versions.snapshot(p.id);
 await fs.unlink(path.join(project,'item'));await store.write(p.id,'item/nested.txt','new nested');
 const unlink=fs.unlink;let changed='';
 fs.unlink=async file=>{if(!changed && path.basename(file)==='nested.txt' && file.includes(`${path.sep}.refract-restore-`)){changed=file;await fs.writeFile(file,'changed staged data');throw Object.assign(new Error('temporary lock'),{code:'EPERM'});}return unlink(file);};
 let result;try {result=await versions.restore(p.id,old.id);} finally {fs.unlink=unlink;}
 assert.equal(result.cleanupPending,1);
 assert.ok(changed);assert.equal(await fs.readFile(changed,'utf8'),'changed staged data');assert.equal(await store.read(p.id,'item'),'old file');
 assert.equal(await new Versions(store).retainedStages(p.id),1);
});

test('an interrupted restore retains its recovery record and can recover on restart', async t => {
 const root: string = await fs.mkdtemp(path.join(os.tmpdir(),'refract-interrupted-')); t.after(()=>fs.rm(root,{recursive:true,force:true}));
 const store = new ProjectStore(root), p = await store.create('Paper'), versions = new Versions(store), project = store.projectDir(p.id);
 await store.write(p.id,'a.txt','old a'); await store.write(p.id,'z.txt','old z'); const original = await versions.snapshot(p.id);
 await store.write(p.id,'a.txt','new a'); await store.write(p.id,'z.txt','new z');
 const rename = fs.rename;
 fs.rename = async (from: string, to: string) => { if (to === path.join(project,'z.txt')) throw new Error('persistent failure'); return rename(from,to); };
 try { await assert.rejects(versions.restore(p.id,original.id),/Recovery also failed/); } finally { fs.rename = rename; }
 const journal = path.join(project,'.refract-restore-journal.json'); await fs.access(journal);
 const restarted = new Versions(store); assert.equal(await restarted.recoverInterrupted(p.id),true);
 assert.equal(await store.read(p.id,'a.txt'),'new a'); assert.equal(await store.read(p.id,'z.txt'),'new z');
 await assert.rejects(fs.access(journal),{code:'ENOENT'}); assert.equal(await restarted.recoverInterrupted(p.id),false);
});

test('legacy recovery journals restore without deleting unrecorded stages',async t=>{
 const root:string=await fs.mkdtemp(path.join(os.tmpdir(),'refract-legacy-journal-'));
 t.after(async()=>{const resolved=await fs.realpath(root),parent=await fs.realpath(os.tmpdir());if(path.dirname(resolved).toLowerCase()!==parent.toLowerCase() || !path.basename(resolved).startsWith('refract-legacy-journal-'))throw new Error('Unsafe cleanup path');await fs.rm(resolved,{recursive:true,force:true});});
 const store=new ProjectStore(root),p=await store.create('Paper'),versions=new Versions(store),project=store.projectDir(p.id);
 await store.write(p.id,'main.tex','before restore');const recovery=await versions.snapshot(p.id);
 const unowned=path.join(project,'.refract-restore-22222222-2222-2222-2222-222222222222');await fs.mkdir(unowned);await fs.writeFile(path.join(unowned,'keep'),'preserved');
 await fs.writeFile(path.join(project,'.refract-restore-journal.json'),JSON.stringify({recoveryId:recovery.id}));await store.write(p.id,'main.tex','partially restored');
 assert.equal(await versions.recoverInterrupted(p.id),true);assert.equal(await store.read(p.id,'main.tex'),'before restore');assert.equal(await fs.readFile(path.join(unowned,'keep'),'utf8'),'preserved');
});

test('a process terminated during restore is recovered on next startup', async t => {
 const root: string = await fs.mkdtemp(path.join(os.tmpdir(),'refract-killed-restore-')); t.after(()=>fs.rm(root,{recursive:true,force:true}));
 const store = new ProjectStore(root), p = await store.create('Paper'), versions = new Versions(store);
 await store.write(p.id,'a.txt','old a'); await store.write(p.id,'z.txt','old z'); const old = await versions.snapshot(p.id);
 await store.write(p.id,'a.txt','new a'); await store.write(p.id,'z.txt','new z');
 const {spawn} = require('node:child_process');
 const result: number = await new Promise((resolve,reject) => {
  const child = spawn(process.execPath,[path.join(__dirname,'fixtures','restore-child.cjs'),root,p.id,old.id],{windowsHide:true,stdio:'ignore'});
  child.on('error',reject); child.on('close',resolve);
 });
 assert.equal(result,73); assert.equal(await store.read(p.id,'a.txt'),'old a');
 assert.equal(await new Versions(store).recoverInterrupted(p.id),true);
 assert.equal(await store.read(p.id,'a.txt'),'new a'); assert.equal(await store.read(p.id,'z.txt'),'new z');
});

test('interrupted staging, folder replacement, file deletion and metadata updates recover the previous project', {timeout:30000}, async t=>{
 const root:string=await fs.mkdtemp(path.join(os.tmpdir(),'refract-restore-points-'));t.after(()=>fs.rm(root,{recursive:true,force:true}));
 const store=new ProjectStore(root),{spawn}=require('node:child_process');
 for(const point of ['journal','mkdir','stage-write','blocker','delete','metadata','cleanup']) {
  const p=await store.create(point),versions=new Versions(store),project=store.projectDir(p.id);
  await store.write(p.id,'item','old file');const old=await versions.snapshot(p.id);
  await fs.unlink(path.join(project,'item'));await store.write(p.id,'item/nested.txt','new nested');
  await store.write(p.id,'added.txt','new file');await store.write(p.id,'other.tex','new root');
  const meta=await store.metadata(p.id);meta.rootFile='other.tex';await fs.writeFile(path.join(project,'.refract.json'),JSON.stringify(meta));
  let stderr='';const code:number=await new Promise((resolve,reject)=>{const child=spawn(process.execPath,[path.join(__dirname,'fixtures','restore-child.cjs'),root,p.id,old.id,point],{windowsHide:true,stdio:['ignore','ignore','pipe']});child.stderr.on('data',chunk=>{stderr+=chunk.toString();});child.on('error',reject);child.on('close',resolve);});
  assert.equal(code,73,`did not reach ${point}: ${stderr}`);
  assert.equal(await new Versions(store).recoverInterrupted(p.id),true);
  assert.equal(await store.read(p.id,'item/nested.txt'),'new nested');assert.equal(await store.read(p.id,'added.txt'),'new file');
  assert.equal((await store.metadata(p.id)).rootFile,'other.tex');
  await assert.rejects(fs.access(path.join(project,'.refract-restore-journal.json')),{code:'ENOENT'});
  const stages=(await fs.readdir(project)).filter(name=>/^\.refract-restore-[0-9a-f-]{36}$/.test(name));
  if(point==='mkdir') {assert.equal(stages.length,1);assert.deepEqual(await fs.readdir(path.join(project,stages[0])),[]);} else assert.deepEqual(stages,[]);
 }
});

test('recovery preserves unowned stages and changed or unexpected staged data', {timeout:30000}, async t=>{
 const root:string=await fs.mkdtemp(path.join(os.tmpdir(),'refract-stage-preserve-'));
 t.after(async()=>{const resolved=await fs.realpath(root),parent=await fs.realpath(os.tmpdir());if(path.dirname(resolved).toLowerCase()!==parent.toLowerCase() || !path.basename(resolved).startsWith('refract-stage-preserve-'))throw new Error('Unsafe cleanup path');await fs.rm(resolved,{recursive:true,force:true});});
 const store=new ProjectStore(root),{spawn}=require('node:child_process');
 for(const change of ['unknown','modified','concurrent','marker']) {
  const p=await store.create(change),versions=new Versions(store),project=store.projectDir(p.id);
  await store.write(p.id,'item','old file');const old=await versions.snapshot(p.id);
  await fs.unlink(path.join(project,'item'));await store.write(p.id,'item/nested.txt','new nested');
  const code:number=await new Promise((resolve,reject)=>{const child=spawn(process.execPath,[path.join(__dirname,'fixtures','restore-child.cjs'),root,p.id,old.id,'blocker'],{windowsHide:true,stdio:'ignore'});child.on('error',reject);child.on('close',resolve);});
  assert.equal(code,73);
  const journal=JSON.parse(await fs.readFile(path.join(project,'.refract-restore-journal.json'),'utf8'));
  const stage=path.join(project,`.refract-restore-${journal.stages[0].id}`),blocked=path.join(stage,`blocked-${journal.stages[0].files.length}`);
  const preserved=change==='unknown'?path.join(stage,'unexpected.txt'):change==='marker'?path.join(stage,'.refract-stage-owner'):path.join(blocked,'nested.txt');if(change!=='concurrent')await fs.writeFile(preserved,change);
  const unowned=path.join(project,'.refract-restore-11111111-1111-1111-1111-111111111111');await fs.mkdir(unowned);await fs.writeFile(path.join(unowned,'keep'),'unowned');
  const readFile=fs.readFile;let changed=false;
  if(change==='concurrent')fs.readFile=async (file,...args)=>{const result=await readFile(file,...args);if(file===preserved && !changed){changed=true;await fs.writeFile(file,change);}return result;};
  try {assert.equal(await new Versions(store).recoverInterrupted(p.id),true);} finally {fs.readFile=readFile;}
  assert.equal(await store.read(p.id,'item/nested.txt'),'new nested');
  assert.equal(await fs.readFile(preserved,'utf8'),change);assert.equal(await fs.readFile(path.join(unowned,'keep'),'utf8'),'unowned');
 }
});

test('recovery does not follow a replaced staging directory link', {timeout:30000}, async t=>{
 const root:string=await fs.mkdtemp(path.join(os.tmpdir(),'refract-stage-link-'));
 t.after(async()=>{const resolved=await fs.realpath(root),parent=await fs.realpath(os.tmpdir());if(path.dirname(resolved).toLowerCase()!==parent.toLowerCase() || !path.basename(resolved).startsWith('refract-stage-link-'))throw new Error('Unsafe cleanup path');await fs.rm(resolved,{recursive:true,force:true});});
 const store=new ProjectStore(path.join(root,'projects')),p=await store.create('Paper'),versions=new Versions(store),project=store.projectDir(p.id),{spawn}=require('node:child_process');
 await store.write(p.id,'item','old file');const old=await versions.snapshot(p.id);
 await fs.unlink(path.join(project,'item'));await store.write(p.id,'item/nested.txt','new nested');
 const code:number=await new Promise((resolve,reject)=>{const child=spawn(process.execPath,[path.join(__dirname,'fixtures','restore-child.cjs'),store.root,p.id,old.id,'blocker'],{windowsHide:true,stdio:'ignore'});child.on('error',reject);child.on('close',resolve);});assert.equal(code,73);
 const journal=JSON.parse(await fs.readFile(path.join(project,'.refract-restore-journal.json'),'utf8'));
 const stage=path.join(project,`.refract-restore-${journal.stages[0].id}`),saved=path.join(root,'saved-stage'),outside=path.join(root,'outside');
 await fs.rename(stage,saved);await fs.mkdir(outside);await fs.writeFile(path.join(outside,'keep'),'untouched');
 try {await fs.symlink(outside,stage,process.platform==='win32'?'junction':'dir');}catch(error){if(['EPERM','EACCES'].includes(error.code))return t.skip('Creating directory links is not permitted');throw error;}
 try {assert.equal(await new Versions(store).recoverInterrupted(p.id),true);assert.equal(await store.read(p.id,'item/nested.txt'),'new nested');assert.equal(await fs.readFile(path.join(outside,'keep'),'utf8'),'untouched');}
 finally {await fs.unlink(stage);}
});


test('snapshot and comparison reject a folder replaced by a link after enumeration',async t=>{
 const root:string=await fs.mkdtemp(path.join(os.tmpdir(),'refract-version-swap-'));
 t.after(()=>fs.rm(root,{recursive:true,force:true}));
 const store=new ProjectStore(path.join(root,'projects')),p=await store.create('Paper'),versions=new Versions(store);
 const project=store.projectDir(p.id), folder=path.join(project,'chapter'), outside=path.join(root,'outside');
 await fs.mkdir(folder);await fs.mkdir(outside);await fs.writeFile(path.join(folder,'text.txt'),'inside');await fs.writeFile(path.join(outside,'text.txt'),'outside');
 const version=await versions.snapshot(p.id), originalFiles=store.files.bind(store);
 store.files=async id=>{const names=await originalFiles(id);await fs.rename(folder,folder+'-saved');try {await fs.symlink(outside,folder,process.platform==='win32'?'junction':'dir');} catch(error) {await fs.rename(folder+'-saved',folder);throw error;}return names;};
 try {
  try {await assert.rejects(versions.compare(p.id,version.id),/Invalid destination folder/);} catch(error) {if(['EPERM','EACCES'].includes(error.code))return t.skip('Creating directory links is not permitted');throw error;}
  await fs.unlink(folder);await fs.rename(folder+'-saved',folder);
  await assert.rejects(versions.snapshot(p.id),/Invalid destination folder/);
 } finally {await fs.unlink(folder).catch(()=>{});await fs.rename(folder+'-saved',folder).catch(()=>{});}
});
