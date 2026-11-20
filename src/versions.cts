const fs = require('node:fs/promises');
const path = require('node:path');
const crypto = require('node:crypto');
const Zip = require('adm-zip');
const {SourceCache,mapLimited}:typeof import('./source-cache.cjs') = require('./source-cache.cjs');

type RestoreStage = { id: string; owner: string; targetId: string; beforeId: string; files: string[]; blockers: string[] };
type RestoreRecord = { recoveryId: string; stages?: RestoreStage[] };
type SnapshotData = { version: {id: string; rootFile?: string}; files: Map<string, Buffer>; directories: string[] | null };

class Versions {
 store: any;
 private readonly sourceCache = new SourceCache();
 private readonly archives = new Map<string,{bytes:Buffer;hashes:Map<string,string>}>();
 private archiveBytes=0;
 constructor(store) { this.store = store; }
 sourceStatistics() {return this.sourceCache.statistics();}
 directory(id) { return path.join(this.store.projectDir(id), '.refract-versions'); }
 async checkedDirectory(id: string, create = false): Promise<string> {
  const root = await this.store.destinationDir(id), dir = path.join(root, '.refract-versions');
  try { const info = await fs.lstat(dir); if (info.isSymbolicLink() || !info.isDirectory()) throw new Error('Invalid version history directory'); }
  catch (error) {
   if (error.code !== 'ENOENT' || !create) throw error;
   try { await fs.mkdir(dir); } catch (mkdirError) { if (mkdirError.code !== 'EEXIST') throw mkdirError; }
   const info = await fs.lstat(dir); if (info.isSymbolicLink() || !info.isDirectory()) throw new Error('Invalid version history directory');
  }
  return dir;
 }
 async checkedFile(file: string): Promise<void> {
  const info = await fs.lstat(file);
  if (info.isSymbolicLink() || !info.isFile() || info.nlink > 1) throw new Error('Invalid version history file');
 }
 async list(id) {
  let dir: string; try { dir = await this.checkedDirectory(id); } catch (e) { if (e.code === 'ENOENT') return []; throw e; }
  const files = await fs.readdir(dir);
  const versions = [];
  for (const file of files.filter(f => /^[0-9a-f-]{36}\.json$/.test(f))) { const name = path.join(dir,file); await this.checkedFile(name); versions.push(JSON.parse(await fs.readFile(name, 'utf8'))); }
  return versions.sort((a,b) => (b.sequence || 0) - (a.sequence || 0) || b.created.localeCompare(a.created));
 }
 async snapshot(id) {
  const names = (await this.store.files(id)).sort(), hash = crypto.createHash('sha256');
  const directories: string[] = [];
  const visit = nodes => { for (const node of nodes) if (node.children) { directories.push(node.path); visit(node.children); } };
  visit(await this.store.tree(id)); directories.sort();
  const metadata = await this.store.metadata(id), rootFile = metadata.rootFile || 'main.tex';
  hash.update('refract-snapshot-v2:'+JSON.stringify({directories, rootFile}));
  const entries = await mapLimited<string,{data:Buffer;sha256:string}>(names,async name=>this.sourceCache.read(await this.store.existingFile(id,name)));
  for(let i=0;i<names.length;i++)hash.update(JSON.stringify([names[i],entries[i].data.length,entries[i].sha256]));
  const digest = hash.digest('hex'), latest = (await this.list(id))[0];
  if (latest?.digest === digest) return latest;
  // Reuse only compressed bytes that this instance created and verified from
  // project content, never hashes supplied by an editable history manifest.
  const previous=this.archives.get(id),zip=previous ? new Zip(previous.bytes) : new Zip();
  const currentNames=new Set(names);
  if(previous)for(const name of previous.hashes.keys())if(!currentNames.has(name))zip.deleteFile(name);
  for(let i=0;i<names.length;i++)if(previous?.hashes.get(names[i])!==entries[i].sha256)zip.addFile(names[i],entries[i].data);
  const version = {id:crypto.randomUUID(),created:new Date().toISOString(),sequence:(latest?.sequence || 0) + 1,digest,files:names,directories,rootFile};
  const dir = await this.checkedDirectory(id,true);
  const archive = path.join(dir, version.id+'.zip'), metadataFile = path.join(dir, version.id+'.json');
  const temp = path.join(dir, version.id+'.json.tmp');
  let archiveCreated = false;
  const archiveData:Buffer=zip.toBuffer();
  try {
   await fs.writeFile(archive, archiveData, {flag:'wx'}); archiveCreated = true;
   await fs.writeFile(temp, JSON.stringify(version), {flag:'wx'});
   await fs.rename(temp, metadataFile);
  } catch (error) {
   await fs.rm(temp, {force:true}).catch(() => {});
   if (archiveCreated) await fs.rm(archive, {force:true}).catch(() => {});
   throw error;
  }
  if(previous){this.archives.delete(id);this.archiveBytes-=previous.bytes.length;}
  if(archiveData.length<=32*1024*1024) {
   while(this.archiveBytes+archiveData.length>32*1024*1024 && this.archives.size){const oldest=this.archives.keys().next().value!;this.archiveBytes-=this.archives.get(oldest)!.bytes.length;this.archives.delete(oldest);}
   this.archives.set(id,{bytes:archiveData,hashes:new Map(names.map((name,index)=>[name,entries[index].sha256]))});this.archiveBytes+=archiveData.length;
  }
  return version;
 }
 async load(id, versionId) {
  if (!/^[0-9a-f-]{36}$/.test(versionId)) throw new Error('Invalid version');
  const version = (await this.list(id)).find(v => v.id === versionId);
  if (!version || !Array.isArray(version.files)) throw new Error('Version not found');
  const dir = await this.checkedDirectory(id), archive = path.join(dir, versionId+'.zip');
  await this.checkedFile(archive);
  const zip = new Zip(archive);
  const files = new Map<string, Buffer>(), seen = new Set<string>();
  for (const name of version.files) {
   if (typeof name !== 'string' || name.split(/[\\/]/).some(part => part.startsWith('.'))) throw new Error('Invalid path in version');
   this.store.filePath(id,name);
   if (seen.has(name.toLowerCase())) throw new Error('Duplicate file in version');
   seen.add(name.toLowerCase());
   const entry = zip.getEntry(name); if (!entry || entry.isDirectory) throw new Error(`File missing from version: ${name}`);
   files.set(name,entry.getData());
  }
  const directories = Array.isArray(version.directories) ? version.directories : null;
  if (directories) for (const name of directories) {
   if (typeof name !== 'string' || name.split(/[\\/]/).some(part => part.startsWith('.'))) throw new Error('Invalid path in version');
   this.store.filePath(id,name);
  }
  if (version.rootFile) {
   if (typeof version.rootFile !== 'string' || version.rootFile.split(/[\\/]/).some(part => part.startsWith('.'))) throw new Error('Invalid root file in version');
   this.store.filePath(id,version.rootFile);
  }
  return {version, files, directories};
 }
 async readFile(id, versionId, name) {
  const {files} = await this.load(id,versionId);
  const data = files.get(name); if (!data) throw new Error('File not found in version');
  return data;
 }
 async compare(id, versionId) {
  const {version, files, directories} = await this.load(id,versionId), current = new Map<string, Buffer>();
  for (const name of await this.store.files(id)) current.set(name,await fs.readFile(await this.store.existingFile(id,name)));
  const result = [];
  for (const name of [...new Set([...files.keys(),...current.keys()])].sort()) {
   const previous = files.get(name), now = current.get(name);
   if (previous && now && previous.equals(now)) continue;
   const text = data => {
    if (!data || data.length > 100000 || data.includes(0)) return null;
    try { return new TextDecoder('utf-8',{fatal:true}).decode(data); } catch { return null; }
   };
   result.push({name, status: !previous ? 'added' : !now ? 'deleted' : 'changed', previous: text(previous), current: text(now)});
  }
  if (directories) {
   const currentDirs: string[] = [];
   const visit = nodes => { for (const node of nodes) if (node.children) { currentDirs.push(node.path); visit(node.children); } };
   visit(await this.store.tree(id));
   for (const name of [...new Set([...directories,...currentDirs])].sort()) {
    const before = directories.includes(name), now = currentDirs.includes(name);
    if (before !== now) result.push({name: name + '/',status: now ? 'folder added' : 'folder deleted',previous:before ? 'Folder exists' : null,current:now ? 'Folder exists' : null});
   }
  }
  const meta = await this.store.metadata(id), rootFile = meta.rootFile || 'main.tex';
  if (version.rootFile && version.rootFile !== rootFile) result.push({name:'Root document',status:'changed',previous:version.rootFile,current:rootFile});
  return result;
 }
 async recoverInterrupted(id: string): Promise<boolean> {
  const journal = path.join(this.store.projectDir(id), '.refract-restore-journal.json');
  let record: RestoreRecord;
  try { record = JSON.parse(await fs.readFile(journal, 'utf8')); }
  catch (error) { if (error.code === 'ENOENT') return false; throw error; }
  if (!record || typeof record.recoveryId !== 'string' || !/^[0-9a-f-]{36}$/.test(record.recoveryId)) throw new Error('Invalid restore recovery record');
  await this.restore(id, record.recoveryId, true);
  return true;
 }
 async retainedStages(id: string): Promise<number> {
  await this.retryRetainedStages(id);
  const project=await this.store.destinationDir(id);
  return (await fs.readdir(project)).filter((name:string)=>/^\.refract-restore-[0-9a-f-]{36}$/.test(name)).length;
 }
 async retryRetainedStages(id: string): Promise<void> {
  const project = await this.store.destinationDir(id);
  for (const name of await fs.readdir(project)) {
   if (!/^\.refract-cleanup-[0-9a-f-]{36}\.json$/.test(name)) continue;
   const file = path.join(project,name);
   try {
    await this.checkedFile(file);
    const record: RestoreRecord = JSON.parse(await fs.readFile(file,'utf8'));
    if (!Array.isArray(record.stages)) continue;
    let pending = false;
    for (const stage of record.stages) {
     try { await this.cleanStage(id,stage); } catch { pending = true; continue; }
     if (!stage || typeof stage.id !== 'string' || !/^[0-9a-f-]{36}$/.test(stage.id)) { pending = true; continue; }
     try { await fs.lstat(path.join(project,`.refract-restore-${stage.id}`)); pending = true; }
     catch (error) { if (error.code !== 'ENOENT') pending = true; }
    }
    if (!pending) await fs.unlink(file);
   } catch { /* Preserve records that cannot be safely read or cleaned. */ }
  }
 }
 // A stage may contain project files displaced by a restore. Remove it only
 // when every remaining entry is accounted for by an existing snapshot.
 async cleanStage(id: string, record: RestoreStage): Promise<void> {
  const uuid = /^[0-9a-f-]{36}$/;
  if (!record || !uuid.test(record.id) || !uuid.test(record.owner) || !uuid.test(record.targetId) || !uuid.test(record.beforeId) || !Array.isArray(record.files) || !Array.isArray(record.blockers)) return;
  const project = await this.store.destinationDir(id), stage = path.join(project, `.refract-restore-${record.id}`);
  let stat; try { stat = await fs.lstat(stage); } catch (error) { if (error.code === 'ENOENT') return; throw error; }
  if (stat.isSymbolicLink() || !stat.isDirectory()) return;
  const marker = path.join(stage,'.refract-stage-owner');
  let markerInfo; try { markerInfo = await fs.lstat(marker); } catch (error) { if (error.code === 'ENOENT') return; throw error; }
  if (markerInfo.isSymbolicLink() || !markerInfo.isFile() || markerInfo.nlink > 1 || await fs.readFile(marker,'utf8') !== record.owner) return;
  const [target,before] = await Promise.all([this.load(id,record.targetId),this.load(id,record.beforeId)]);
  if (!before.directories || record.files.some(name => typeof name !== 'string' || !target.files.has(name)) || record.blockers.some(name => typeof name !== 'string')) return;
  const removals: string[] = [], directories: string[] = [];
  const observed = new Map<string, import('node:fs').Stats>([[stage,stat],[marker,markerInfo]]);
  const check = async (entry: string, original: string, data: SnapshotData): Promise<boolean> => {
   const info = await fs.lstat(entry);
   if (info.isSymbolicLink() || info.nlink > 1 && !info.isDirectory()) return false;
   observed.set(entry,info);
   if (info.isFile()) {
    const expected = data.files.get(original); if (!expected || !expected.equals(await fs.readFile(entry))) return false;
    removals.push(entry); return true;
   }
   if (!info.isDirectory() || !data.directories?.includes(original)) return false;
   for (const name of await fs.readdir(entry)) {
    if (name.startsWith('.') || !await check(path.join(entry,name),`${original}/${name}`,data)) return false;
   }
   directories.push(entry); return true;
  };
  for (const name of await fs.readdir(stage)) {
   if (name === '.refract-stage-owner') continue;
   if (/^(0|[1-9][0-9]*)$/.test(name)) {
    const index = Number(name), original = record.files[index];
    if (!Number.isSafeInteger(index) || original === undefined || !await check(path.join(stage,name),original,target)) return;
   } else if (/^blocked-(0|[1-9][0-9]*)$/.test(name)) {
    const index = Number(name.slice(8)) - record.files.length, original = record.blockers[index];
    if (!Number.isSafeInteger(index) || original === undefined || !await check(path.join(stage,name),original,before)) return;
   } else return;
  }
  // Do not remove anything if an entry changed while the stage was checked.
  for (const [entry,previous] of observed) {
   const current = await fs.lstat(entry);
   if (current.isSymbolicLink() || current.dev !== previous.dev || current.ino !== previous.ino || current.size !== previous.size || current.mtimeMs !== previous.mtimeMs || current.nlink !== previous.nlink) return;
  }
  for (const file of removals) await fs.unlink(file);
  for (const dir of directories) await fs.rmdir(dir);
  await fs.unlink(marker);
  await fs.rmdir(stage);
 }
 async restore(id, versionId, recovering = false) {
  const target = await this.load(id,versionId);
  const recovery = await this.snapshot(id);
  const backup = await this.load(id,recovery.id);
  const project = this.store.projectDir(id);
  const journal = path.join(project, '.refract-restore-journal.json');
  let record: RestoreRecord;
  if (!recovering) {
   record = {recoveryId: recovery.id, stages: []};
   const temp = path.join(project, `.refract-restore-journal-${crypto.randomUUID()}.tmp`);
   await fs.writeFile(temp, JSON.stringify(record), {flag:'wx'});
   try { await fs.link(temp, journal); } finally { await fs.unlink(temp).catch(() => {}); }
  } else { record = JSON.parse(await fs.readFile(journal,'utf8')); if (!Array.isArray(record.stages)) record.stages = []; }
  const saveRecord = async () => {
   const temp = path.join(project, `.refract-restore-journal-${crypto.randomUUID()}.tmp`);
   try {
    await fs.writeFile(temp, JSON.stringify(record), {flag:'wx'});
    const delays = [80, 160, 320, 640];
    for (let attempt = 0; ; attempt++) {
     try { await fs.rename(temp, journal); break; }
     catch (error) {
      if (process.platform !== 'win32' || !['EPERM', 'EBUSY', 'EACCES'].includes(error.code) || attempt >= delays.length) throw error;
      await new Promise(resolve => setTimeout(resolve, delays[attempt]));
     }
    }
   }
   finally { await fs.unlink(temp).catch(error=>{if(error.code!=='ENOENT')throw error;}); }
  };
  const cleanStages = async () => {
   let pending=0;
   for (const stage of record.stages || []) {
    let failed=false;
    for (let attempt=0;attempt<3;attempt++) {
     try { await this.cleanStage(id,stage); break; }
     catch (error) {
      if (attempt===2 || !['EBUSY','EPERM','EACCES','EMFILE'].includes(error.code)) {failed=true;break;}
      // Recheck ownership and contents before each retry; a lock can outlive
      // the first cleanup attempt and the stage may have changed meanwhile.
      await new Promise(resolve=>setTimeout(resolve,50*(attempt+1)));
     }
    }
    if (failed || !stage || typeof stage.id!=='string' || !/^[0-9a-f-]{36}$/.test(stage.id)) {pending++;continue;}
    try {await fs.lstat(path.join(project,`.refract-restore-${stage.id}`));pending++;}
    catch(error) {if(error.code!=='ENOENT')pending++;}
   }
   return pending;
  };
  const apply = async (data: SnapshotData) => {
   const before = await this.snapshot(id);
    // Move blocking visible files or directories aside; hidden data and links are never removed.
    const blockers = new Set<string>();
    const ensureVisible = async dir => {
     for (const entry of await fs.readdir(dir,{withFileTypes:true})) {
      if (entry.name.startsWith('.') || entry.isSymbolicLink()) throw new Error(`Hidden data or a link prevents replacing folder: ${path.relative(project,dir)}`);
      if (entry.isDirectory()) await ensureVisible(path.join(dir,entry.name));
      else if (!entry.isFile()) throw new Error(`Unsupported entry prevents replacing folder: ${path.relative(project,dir)}`);
     }
    };
    for (const name of [...data.files.keys(), ...(data.directories || [])]) {
     const parts = name.split(/[\\/]/), dirEntry = data.directories?.includes(name);
     let cursor = project;
     for (let i = 0; i < parts.length; i++) {
      cursor = path.join(cursor,parts[i]);
      let stat; try { stat = await fs.lstat(cursor); } catch (e) { if (e.code === 'ENOENT') break; throw e; }
      if (stat.isSymbolicLink()) throw new Error(`Cannot restore through a link: ${name}`);
      if (i < parts.length - 1 && !stat.isDirectory()) { if (!stat.isFile()) throw new Error(`Unsupported path in project: ${name}`); blockers.add(cursor); break; }
      if (i === parts.length - 1 && (dirEntry ? !stat.isDirectory() : !stat.isFile())) {
       if (stat.isDirectory()) await ensureVisible(cursor);
       else if (!stat.isFile()) throw new Error(`Unsupported path in project: ${name}`);
       blockers.add(cursor);
      }
     }
    }
    const orderedBlockers = [...blockers].sort((a,b) => a.length-b.length).filter(blocker => ![...blockers].some(parent => parent !== blocker && blocker.startsWith(parent + path.sep)));
    const stageRecord: RestoreStage = {id:crypto.randomUUID(),owner:crypto.randomUUID(),targetId:data.version.id,beforeId:before.id,files:[...data.files.keys()],blockers:orderedBlockers.map(blocker=>path.relative(project,blocker).split(path.sep).join('/'))};
    record.stages!.push(stageRecord); await saveRecord();
    const stage = path.join(project, `.refract-restore-${stageRecord.id}`); await fs.mkdir(stage);
    await fs.writeFile(path.join(stage,'.refract-stage-owner'),stageRecord.owner,{flag:'wx'});
    let index = 0;
    const staged = new Map<string,string>();
    for (const [name,bytes] of data.files) { const file = path.join(stage,String(index++)); await fs.writeFile(file,bytes,{flag:'wx'}); staged.set(name,file); }
    for (const blocker of orderedBlockers) {
     await fs.rename(blocker,path.join(stage,`blocked-${index++}`));
    }
    for (const name of data.directories || []) await fs.mkdir(this.store.filePath(id,name),{recursive:true});
    for (const [name,file] of staged) {
     const destination = this.store.filePath(id,name);
     await fs.mkdir(path.dirname(destination),{recursive:true});
     await fs.rename(file,destination);
    }
    const targetNames = new Set([...data.files.keys()].map(name => name.toLowerCase()));
    for (const name of await this.store.files(id)) if (!targetNames.has(name.toLowerCase())) await fs.unlink(this.store.filePath(id,name));
    if (data.directories) {
     const existing: string[] = [];
     const visit = nodes => { for (const node of nodes) if (node.children) { existing.push(node.path); visit(node.children); } };
     visit(await this.store.tree(id));
     const targetDirs = new Set(data.directories.map(name => name.toLowerCase()));
     for (const name of existing.sort((a,b) => b.length-a.length)) if (!targetDirs.has(name.toLowerCase())) {
      try { await fs.rmdir(this.store.filePath(id,name)); } catch (e) { if (!['ENOTEMPTY','EEXIST'].includes(e.code)) throw e; }
     }
    }
    const meta = await this.store.metadata(id);
    if (data.version.rootFile) meta.rootFile = data.version.rootFile;
    else if (!data.files.has(meta.rootFile || 'main.tex')) meta.rootFile = data.files.has('main.tex') ? 'main.tex' : [...data.files.keys()].find(name => /\.tex$/i.test(name)) || meta.rootFile;
    const file = path.join(project,'.refract.json'), temp = path.join(stage,'metadata');
    await fs.writeFile(temp,JSON.stringify(meta,null,2)); await fs.rename(temp,file);
  };
  try { await apply(target); } catch (error) {
   try { await apply(backup); } catch (rollback) { throw new Error(`Restore failed: ${error.message}. Recovery also failed: ${rollback.message}. Recovery version: ${recovery.id}`); }
   const pending = !recovering ? await cleanStages() : 0;
   if (!recovering) { if (pending) await fs.rename(journal,path.join(project,`.refract-cleanup-${crypto.randomUUID()}.json`)); else await fs.unlink(journal); }
   throw new Error(`Restore failed and the previous project was recovered: ${error.message}${pending ? '. Temporary restore data remains in the project folder.' : ''}`);
  }
  await this.snapshot(id);
  const cleanupPending=await cleanStages();
  if (cleanupPending) await fs.rename(journal,path.join(project,`.refract-cleanup-${crypto.randomUUID()}.json`));
  else await fs.unlink(journal);
  return {recoveryId:recovery.id,cleanupPending};
 }
}
module.exports = {Versions};
