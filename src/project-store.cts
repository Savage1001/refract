import trust = require('./project-trust.cjs');
﻿const fs = require('node:fs/promises');
const path = require('node:path');
const crypto = require('node:crypto');

async function renameProjectDirectory(source, destination) {
  const delays=[80,160,320,640];
  for(let attempt=0;;attempt++) {
    try {await fs.rename(source,destination);return;}
    catch(error) {
      if(process.platform!=='win32' || !['EPERM','EBUSY','EACCES'].includes(error.code) || attempt>=delays.length) throw error;
      try {const info=await fs.lstat(source);if(!info.isDirectory() || info.isSymbolicLink()) throw error;}
      catch {throw error;}
      try {await fs.lstat(destination);throw error;} catch(check) {if(check.code!=='ENOENT') throw error;}
      await new Promise(resolve=>setTimeout(resolve,delays[attempt]));
    }
  }
}

class ProjectStore {
  root: string;
  constructor(root) { this.root = path.resolve(root); }
  async init() { await fs.mkdir(this.root, { recursive: true }); }
  projectDir(id) {
    if (!/^[0-9a-f-]{36}$/.test(id)) throw new Error('Invalid project ID');
    return path.join(this.root, id);
  }
  filePath(id, name) {
    if (!name || path.isAbsolute(name) || name.includes(':') || name.split(/[\\/]/).some(part => !part || part === '.' || part === '..')) throw new Error('Invalid file path');
    const dir = this.projectDir(id), target = path.resolve(dir, name);
    if (!target.startsWith(dir + path.sep)) throw new Error('Invalid file path');
    return target;
  }
  async create(name) {
    const id = crypto.randomUUID(), dir = this.projectDir(id);
    await fs.mkdir(dir, { recursive: true });
    const meta = { id, name: String(name).trim() || 'New Project', created: new Date().toISOString() };
    await fs.writeFile(path.join(dir, '.refract.json'), JSON.stringify(meta, null, 2));
    await fs.writeFile(path.join(dir, 'main.tex'), '\\documentclass[11pt]{article}\n\\usepackage[margin=1in]{geometry}\n\\usepackage{amsmath,amssymb}\n\n\\title{Untitled}\n\\author{Author Name}\n\\date{\\today}\n\n\\begin{document}\n\\maketitle\n\n\\section{Introduction}\n\n\\end{document}\n');
    await trust.grantTrust(dir);
    return meta;
  }
  async list() {
    await this.init();
    const entries = await fs.readdir(this.root, { withFileTypes: true });
    const projects = await Promise.all(entries.filter(e => e.isDirectory() && /^[0-9a-f-]{36}$/.test(e.name)).map(async e => {
      try {
        const dir=path.join(this.root,e.name), meta=JSON.parse(await fs.readFile(path.join(dir,'.refract.json'),'utf8'));
        if(meta.id!==e.name) return null;
        const latestEdit=async (folder:string):Promise<number>=>{
          const entries=await fs.readdir(folder,{withFileTypes:true}).catch(()=>[]);
          let latest=0;
          for(const entry of entries) {
            if(entry.name.startsWith('.')) continue;
            const file=path.join(folder,entry.name);
            if(entry.isDirectory()) latest=Math.max(latest,await latestEdit(file));
            else if(entry.isFile()) {
              const info=await fs.lstat(file).catch(()=>null);
              if(info?.isFile()) latest=Math.max(latest,info.mtimeMs);
            }
          }
          return latest;
        };
        const edited=await latestEdit(dir);
        return {...meta,edited:edited ? new Date(edited).toISOString() : meta.created};
      } catch { return null; }
    }));
    return projects.filter(Boolean);
  }
  async projectTrashDir(create=false) {
    await this.init(); const rootInfo=await fs.lstat(this.root);
    if(rootInfo.isSymbolicLink() || !rootInfo.isDirectory()) throw new Error('Invalid project storage directory');
    const trash=path.join(this.root,'.refract-project-trash');
    if(create) await fs.mkdir(trash,{recursive:true});
    try {const info=await fs.lstat(trash);if(info.isSymbolicLink() || !info.isDirectory()) throw new Error('Invalid deleted projects directory');}
    catch(error) {if(error.code==='ENOENT' && !create) return null;throw error;}
    return trash;
  }
  async trashProject(id) {
    const source=this.projectDir(id), info=await fs.lstat(source);
    if(info.isSymbolicLink() || !info.isDirectory()) throw new Error('Invalid project directory');
    const meta=await this.metadata(id);if(meta.id!==id || typeof meta.name!=='string') throw new Error('Invalid project metadata');
    const trash=await this.projectTrashDir(true), token=crypto.randomUUID(), target=path.join(trash,token), record=path.join(trash,token+'.json');
    await fs.writeFile(record,JSON.stringify({id:token,projectId:id,name:meta.name,deleted:new Date().toISOString()}),{flag:'wx'});
    try {try {await fs.lstat(target);throw new Error('Deleted project identifier already exists');} catch(error) {if(error.code!=='ENOENT') throw error;} await renameProjectDirectory(source,target);}
    catch(error) {await fs.rm(record,{force:true});throw error;}
  }
  async listDeletedProjects() {
    const trash=await this.projectTrashDir();if(!trash) return [];
    const entries=await fs.readdir(trash), result=[];
    for(const file of entries.filter(name=>/^[0-9a-f-]{36}\.json$/.test(name))) {
      const token=file.slice(0,-5);
      try {
        const record=JSON.parse(await fs.readFile(path.join(trash,file),'utf8'));
        if(!record || typeof record!=='object' || record.id!==token || typeof record.projectId!=='string' || !/^[0-9a-f-]{36}$/.test(record.projectId) || typeof record.name!=='string') continue;
        const dir=path.join(trash,token), info=await fs.lstat(dir);if(!info.isDirectory() || info.isSymbolicLink()) continue;
        const meta=JSON.parse(await fs.readFile(path.join(dir,'.refract.json'),'utf8'));if(meta.id!==record.projectId) continue;
        const deleted=typeof record.deleted==='string' ? Date.parse(record.deleted) : NaN;
        if(Number.isFinite(deleted) && Date.now()-deleted>=30*24*60*60*1000) {
          await fs.rm(dir,{recursive:true,force:true});
          await fs.rm(path.join(trash,file),{force:true});
          continue;
        }
        result.push(record);
      } catch(error) {if(error.code!=='ENOENT' && !(error instanceof SyntaxError)) throw error;}
    }
    return result.sort((a,b)=>String(b.deleted).localeCompare(String(a.deleted)));
  }
  async restoreProject(token) {
    if(typeof token!=='string' || !/^[0-9a-f-]{36}$/.test(token)) throw new Error('Invalid deleted project');
    const record=(await this.listDeletedProjects()).find(item=>item.id===token);if(!record) throw new Error('Deleted project not found');
    const trash=await this.projectTrashDir(), source=path.join(trash,token), destination=this.projectDir(record.projectId);
    try {await fs.lstat(destination);throw Object.assign(new Error('A project with this ID already exists'),{code:'EEXIST'});} catch(error) {if(error.code!=='ENOENT') throw error;}
    await renameProjectDirectory(source,destination);
    await fs.rm(path.join(trash,token+'.json'),{force:true});
    return this.metadata(record.projectId);
  }
  async duplicate(id) {
    await this.destinationDir(id);
    const original = await this.metadata(id), copyId = crypto.randomUUID();
    const source = this.projectDir(id), target = this.projectDir(copyId);
    await fs.mkdir(target);
    const copyTree = async (from, to) => {
      for (const entry of await fs.readdir(from, {withFileTypes:true})) {
        if (entry.name.startsWith('.')) continue;
        const src = path.join(from, entry.name), dst = path.join(to, entry.name);
        if (entry.isDirectory()) { await fs.mkdir(dst); await copyTree(src, dst); }
        else if (entry.isFile()) await fs.copyFile(src, dst, fs.constants.COPYFILE_EXCL);
      }
    };
    try {
      await copyTree(source, target);
      // Copy project discussion and snapshots, but not Codex sessions, attachments or trash.
      const copyPrivateFile = async (from, to) => {
        const info=await fs.lstat(from);
        if (!info.isFile() || info.isSymbolicLink() || info.nlink !== 1) throw new Error('Project metadata contains an unsupported link or file');
        await fs.copyFile(from,to,fs.constants.COPYFILE_EXCL);
      };
      const commentFile=path.join(source,'.refract-comments.json');
      try { await copyPrivateFile(commentFile,path.join(target,'.refract-comments.json')); }
      catch(error) { if(error.code!=='ENOENT') throw error; }
      const history=path.join(source,'.refract-versions'), historyCopy=path.join(target,'.refract-versions');
      let entries;
      try { const info=await fs.lstat(history); if(!info.isDirectory() || info.isSymbolicLink()) throw new Error('Invalid version history directory'); entries=await fs.readdir(history); }
      catch(error) { if(error.code!=='ENOENT') throw error; entries=[]; }
      const versionFiles=entries.filter(name=>/^[0-9a-f-]{36}\.(json|zip)$/.test(name));
      if(versionFiles.length) {
        await fs.mkdir(historyCopy);
        for(const name of versionFiles) await copyPrivateFile(path.join(history,name),path.join(historyCopy,name));
      }
      const meta: {id: string; name: string; created: string; rootFile?: string; engine?: string; draftMode?: boolean} = {id:copyId, name:`${original.name} (copy)`, created:new Date().toISOString()};
      if (original.rootFile) meta.rootFile = original.rootFile;
      if (original.engine) meta.engine = original.engine;
      if (original.draftMode === true) meta.draftMode = true;
      await fs.writeFile(path.join(target,'.refract.json'),JSON.stringify(meta,null,2),{flag:'wx'});
      return meta;
    } catch (error) {
      if (target.startsWith(this.root + path.sep)) await fs.rm(target,{recursive:true,force:true});
      throw error;
    }
  }
  async existingFile(id, name) {
    const target=this.filePath(id,name);
    await this.destinationDir(id,name.split(/[\\/]/).slice(0,-1).join('/'));
    const info=await fs.lstat(target);
    if(!info.isFile() || info.isSymbolicLink()) throw new Error('Only regular project files can be used');
    return target;
  }
  async read(id, name) { return fs.readFile(await this.existingFile(id, name), 'utf8'); }
  async metadata(id) { return JSON.parse(await fs.readFile(path.join(this.projectDir(id), '.refract.json'), 'utf8')); }
  async renameProject(id, name) {
    name = String(name).trim();
    if (!name) throw new Error('Project name is required');
    const file = path.join(this.projectDir(id), '.refract.json');
    const meta = JSON.parse(await fs.readFile(file, 'utf8'));
    meta.name = name;
    const temp = `${file}.${crypto.randomUUID()}.tmp`;
    try { await fs.writeFile(temp, JSON.stringify(meta, null, 2), 'utf8'); await fs.rename(temp, file); }
    catch (error) { await fs.rm(temp, {force:true}).catch(() => {}); throw error; }
    return meta;
  }
  async setDraftMode(id: string, draftMode: boolean) {
    if (typeof draftMode !== 'boolean') throw new Error('Invalid draft mode');
    const file=path.join(this.projectDir(id),'.refract.json'), meta=await this.metadata(id);
    meta.draftMode=draftMode;
    const temp=`${file}.${crypto.randomUUID()}.tmp`;
    try { await fs.writeFile(temp,JSON.stringify(meta,null,2),'utf8'); await fs.rename(temp,file); }
    catch(error) { await fs.rm(temp,{force:true}).catch(()=>{}); throw error; }
    return meta;
  }
  async setEngine(id, engine) {
    if (!['pdflatex','xelatex','lualatex'].includes(engine)) throw new Error('Unsupported LaTeX engine');
    const file=path.join(this.projectDir(id),'.refract.json'), meta=await this.metadata(id);
    meta.engine=engine;
    const temp=`${file}.${crypto.randomUUID()}.tmp`;
    try { await fs.writeFile(temp,JSON.stringify(meta,null,2),'utf8'); await fs.rename(temp,file); }
    catch(error) { await fs.rm(temp,{force:true}).catch(()=>{}); throw error; }
    return meta;
  }
  async createFile(id, name) {
    const file = this.filePath(id, name);
    if(name.split(/[\\/]/).some(part=>part.startsWith('.'))) throw new Error('Invalid file name');
    const parent=name.split(/[\\/]/).slice(0,-1).join('/'); await this.destinationDir(id,parent,true);
    await fs.writeFile(file, '', { encoding: 'utf8', flag: 'wx' });
  }
  async createFolder(id, name) {
    const dir = this.filePath(id, name);
    if(name.split(/[\\/]/).some(part=>part.startsWith('.'))) throw new Error('Invalid folder name');
    const parent=name.split(/[\\/]/).slice(0,-1).join('/'); await this.destinationDir(id,parent,true);
    await fs.mkdir(dir);
  }
  async destinationDir(id, relative='', create=false) {
    const root=this.projectDir(id); let current=root;
    const rootInfo=await fs.lstat(root); if(rootInfo.isSymbolicLink() || !rootInfo.isDirectory()) throw new Error('Invalid project directory');
    if(relative) {
      this.filePath(id,relative);
      for(const part of relative.split(/[\\/]/)) {
        if(part.startsWith('.')) throw new Error('Invalid destination folder');
        current=path.join(current,part);
        try {const stat=await fs.lstat(current);if(stat.isSymbolicLink() || !stat.isDirectory()) throw new Error('Invalid destination folder');}
        catch(error) {if(error.code==='ENOENT' && create) await fs.mkdir(current);else throw error;}
      }
    }
    return current;
  }
  async renameFile(id, oldName, newName) {
    const oldPath = this.filePath(id, oldName), newPath = this.filePath(id, newName);
    if (oldName.split(/[\\/]/).some(p => p.startsWith('.')) || newName.split(/[\\/]/).some(p => p.startsWith('.'))) throw new Error('Invalid file name');
    await this.destinationDir(id,oldName.split(/[\\/]/).slice(0,-1).join('/'));
    await this.destinationDir(id,newName.split(/[\\/]/).slice(0,-1).join('/'));
    const info = await fs.lstat(oldPath);
    if (!info.isFile()) throw new Error('Only files can be renamed');
    const meta = await this.metadata(id), originalMeta = {...meta}, rootFile = meta.rootFile || 'main.tex';
    const metadataFile = path.join(this.projectDir(id), '.refract.json');
    const writeMetadata = async value => {
      const temp = `${metadataFile}.${crypto.randomUUID()}.tmp`;
      try { await fs.writeFile(temp, JSON.stringify(value, null, 2)); await fs.rename(temp, metadataFile); }
      catch (error) { await fs.rm(temp,{force:true}).catch(() => {}); throw error; }
    };
    await fs.link(oldPath, newPath);
    let metadataChanged = false;
    try {
      if (rootFile === oldName) {
        meta.rootFile = newName;
        await writeMetadata(meta); metadataChanged = true;
      }
      await fs.unlink(oldPath);
    } catch (error) {
      const failures = [];
      if (metadataChanged) try { await writeMetadata(originalMeta); } catch (rollback) { failures.push(`metadata: ${rollback.message}`); }
      try { await fs.unlink(newPath); } catch (rollback) { failures.push(`new file ${newName}: ${rollback.message}`); }
      if (failures.length) throw new Error(`Rename failed: ${error.message}. Could not fully reverse it (${failures.join('; ')}).`);
      throw error;
    }
    return meta;
  }
  async moveFolder(id, oldName, newName) {
    const parts = name => { if(typeof name!=='string' || name.split(/[\\/]/).some(p=>p.startsWith('.'))) throw new Error('Invalid folder path'); this.filePath(id,name); return name.replaceAll('\\','/'); };
    oldName=parts(oldName); newName=parts(newName);
    if(newName.toLowerCase()===oldName.toLowerCase() || newName.toLowerCase().startsWith(oldName.toLowerCase()+'/')) throw new Error('Choose a destination outside the folder');
    const root=this.projectDir(id), source=await this.destinationDir(id,oldName), target=this.filePath(id,newName);
    await this.destinationDir(id,newName.split('/').slice(0,-1).join('/'));
    const dirs:string[]=[''], files:string[]=[];
    const scan=async relative=>{
      for(const entry of await fs.readdir(path.join(source,relative),{withFileTypes:true})) {
        const name=path.join(relative,entry.name), info=await fs.lstat(path.join(source,name));
        if(info.isSymbolicLink()) throw new Error('Folders containing links cannot be moved');
        if(info.isDirectory()){dirs.push(name);await scan(name);} else if(info.isFile()) files.push(name); else throw new Error('Folder contains an unsupported entry');
      }
    };
    await scan('');
    const meta=await this.metadata(id), original={...meta}, rootFile=String(meta.rootFile || 'main.tex').replaceAll('\\','/');
    const metadataFile=path.join(root,'.refract.json');
    const saveMeta=async value=>{const tmp=`${metadataFile}.${crypto.randomUUID()}.tmp`;try{await fs.writeFile(tmp,JSON.stringify(value,null,2));await fs.rename(tmp,metadataFile);}catch(error){await fs.rm(tmp,{force:true}).catch(()=>{});throw error;}};
    await fs.mkdir(target); const linked:string[]=[], removed:string[]=[], createdDirs:string[]=['']; let metadataChanged=false;
    try {
      for(const dir of dirs.slice(1)) {await fs.mkdir(path.join(target,dir));createdDirs.push(dir);}
      for(const file of files) {await fs.link(path.join(source,file),path.join(target,file));linked.push(file);}
      if(rootFile.toLowerCase().startsWith(oldName.toLowerCase()+'/')) {meta.rootFile=newName+rootFile.slice(oldName.length);await saveMeta(meta);metadataChanged=true;}
      for(const file of files) {await fs.unlink(path.join(source,file));removed.push(file);}
      for(const dir of [...dirs].sort((a,b)=>b.length-a.length)) await fs.rmdir(path.join(source,dir));
      return meta;
    } catch(error) {
      const failures=[];
      try {for(const dir of dirs) await fs.mkdir(path.join(source,dir),{recursive:true});for(const file of removed) await fs.link(path.join(target,file),path.join(source,file));} catch(rollback) {failures.push(`files: ${rollback.message}`);}
      if(metadataChanged) try {await saveMeta(original);} catch(rollback) {failures.push(`metadata: ${rollback.message}`);}
      if(!failures.length) {
        try {for(const file of linked) await fs.unlink(path.join(target,file));for(const dir of [...createdDirs].sort((a,b)=>b.length-a.length)) await fs.rmdir(path.join(target,dir));}
        catch(rollback) {failures.push(`destination cleanup: ${rollback.message}`);}
      }
      if(failures.length) throw new Error(`Folder move failed: ${error.message}. Recovery needs attention (${failures.join('; ')}).`);
      throw error;
    }
  }
  async trashFile(id, name) {
    if (name.split(/[\\/]/).some(p => p.startsWith('.'))) throw new Error('Invalid file name');
    const source=this.filePath(id,name);let parent=await this.destinationDir(id);
    for (const part of name.split(/[\\/]/).slice(0,-1)) { parent=path.join(parent,part); const stat=await fs.lstat(parent); if(stat.isSymbolicLink() || !stat.isDirectory()) throw new Error('Linked folders cannot be used for deleted files'); }
    const info = await fs.lstat(source);
    if (!info.isFile()) throw new Error('Only files can be deleted');
    const trash = path.join(this.projectDir(id), '.refract-trash');
    await fs.mkdir(trash, {recursive:true});
    if ((await fs.lstat(trash)).isSymbolicLink()) throw new Error('Invalid deleted files directory');
    const token=crypto.randomUUID(), target=path.join(trash,token), metadata=path.join(trash,token+'.json');
    await fs.writeFile(metadata,JSON.stringify({id:token,name,deleted:new Date().toISOString()}),{flag:'wx'});
    try { await fs.rename(source,target); } catch(error) { await fs.rm(metadata,{force:true}); throw error; }
  }
  async trashFolder(id, name) {
    if(typeof name!=='string' || name.split(/[\\/]/).some(part=>part.startsWith('.'))) throw new Error('Invalid folder name');
    this.filePath(id,name);
    const root=this.projectDir(id), source=await this.destinationDir(id,name);
    const scan=async dir=>{for(const entry of await fs.readdir(dir,{withFileTypes:true}) ) {const file=path.join(dir,entry.name), stat=await fs.lstat(file);if(stat.isSymbolicLink()) throw new Error('Folders containing links cannot be deleted');if(stat.isDirectory()) await scan(file);else if(!stat.isFile()) throw new Error('Folder contains an unsupported entry');}};
    await scan(source);
    const trash=path.join(root,'.refract-trash');await fs.mkdir(trash,{recursive:true});
    if((await fs.lstat(trash)).isSymbolicLink()) throw new Error('Invalid deleted files directory');
    const token=crypto.randomUUID(), target=path.join(trash,token), metadata=path.join(trash,token+'.json');
    await fs.writeFile(metadata,JSON.stringify({id:token,name:name.replaceAll('\\','/'),kind:'folder',deleted:new Date().toISOString()}),{flag:'wx'});
    try {
      try {await fs.lstat(target);throw new Error('Deleted file identifier already exists');} catch(error) {if(error.code!=='ENOENT') throw error;}
      await fs.rename(source,target);
    } catch(error) {await fs.rm(metadata,{force:true});throw error;}
  }
  async listTrash(id) {
    const trash=path.join(await this.destinationDir(id),'.refract-trash');
    let names; try { if ((await fs.lstat(trash)).isSymbolicLink()) throw new Error('Invalid deleted files directory'); names=await fs.readdir(trash); } catch(error) { if(error.code==='ENOENT') return []; throw error; }
    const items=[];
    for (const file of names.filter(name=>/^[0-9a-f-]{36}\.json$/.test(name))) {
      const token=file.slice(0,-5);
      try {
        const item=JSON.parse(await fs.readFile(path.join(trash,file),'utf8'));
        if(item.id!==token || typeof item.name!=='string' || item.name.split(/[\\/]/).some(part=>part.startsWith('.'))) continue;
        this.filePath(id,item.name);
        const info=await fs.lstat(path.join(trash,token));
        if ((item.kind==='folder' && info.isDirectory() && !info.isSymbolicLink()) || ((!item.kind || item.kind==='file') && info.isFile())) items.push(item);
      } catch(error) { if(error.code!=='ENOENT' && !(error instanceof SyntaxError)) throw error; }
    }
    return items.sort((a,b)=>String(b.deleted).localeCompare(String(a.deleted)));
  }
  async restoreTrash(id, token) {
    if(typeof token!=='string' || !/^[0-9a-f-]{36}$/.test(token)) throw new Error('Invalid deleted file');
    const item=(await this.listTrash(id)).find(entry=>entry.id===token);
    if(!item) throw new Error('Deleted file not found');
    const root=await this.destinationDir(id), target=this.filePath(id,item.name), parts=item.name.split(/[\\/]/); let parent=root;
    for(const part of parts.slice(0,-1)) {
      parent=path.join(parent,part);
      try { const info=await fs.lstat(parent); if(info.isSymbolicLink() || !info.isDirectory()) throw new Error('A folder in the original path is unavailable'); }
      catch(error) { if(error.code==='ENOENT') await fs.mkdir(parent); else throw error; }
    }
    const trash=path.join(root,'.refract-trash'), source=path.join(trash,token);
    if(item.kind==='folder') {
      const dirs:string[]=[''], files:string[]=[];
      const scan=async relative=>{for(const entry of await fs.readdir(path.join(source,relative),{withFileTypes:true})) {const name=path.join(relative,entry.name), info=await fs.lstat(path.join(source,name));if(info.isSymbolicLink()) throw new Error('Folders containing links cannot be restored');if(info.isDirectory()){dirs.push(name);await scan(name);}else if(info.isFile()) files.push(name);else throw new Error('Deleted folder contains an unsupported entry');}};
      await scan(''); await fs.mkdir(target);
      const created:string[]=[''],linked:string[]=[],removed:string[]=[];
      try {
        for(const dir of dirs.slice(1)){await fs.mkdir(path.join(target,dir));created.push(dir);}
        for(const file of files){await fs.link(path.join(source,file),path.join(target,file));linked.push(file);}
        for(const file of files){await fs.unlink(path.join(source,file));removed.push(file);}
        for(const dir of [...dirs].sort((a,b)=>b.length-a.length)) await fs.rmdir(path.join(source,dir));
      } catch(error) {
        const failures=[];
        try {for(const dir of dirs)await fs.mkdir(path.join(source,dir),{recursive:true});for(const file of removed)await fs.link(path.join(target,file),path.join(source,file));} catch(rollback) {failures.push(`files: ${rollback.message}`);}
        if(!failures.length) try {for(const file of linked)await fs.unlink(path.join(target,file));for(const dir of [...created].sort((a,b)=>b.length-a.length))await fs.rmdir(path.join(target,dir));} catch(rollback){failures.push(`destination cleanup: ${rollback.message}`);}
        if(failures.length) throw new Error(`Folder restore failed: ${error.message}. Recovery needs attention (${failures.join('; ')}).`);
        throw error;
      }
      await fs.rm(path.join(trash,token+'.json'),{force:true});return item.name;
    }
    await fs.link(source,target);
    try { await fs.unlink(source); } catch(error) { await fs.unlink(target).catch(()=>{}); throw error; }
    await fs.rm(path.join(trash,token+'.json'),{force:true});
    return item.name;
  }
  async tree(id) {
    const root = this.projectDir(id);
    const walk = async relative => {
      const entries = await fs.readdir(path.join(root, relative), { withFileTypes: true });
      const result = [];
      for (const entry of entries) {
        if (entry.name.startsWith('.')) continue;
        const name = path.join(relative, entry.name);
        if (entry.isDirectory()) result.push({ name: entry.name, path: name.replaceAll('\\', '/'), children: await walk(name) });
        else if (entry.isFile()) result.push({ name: entry.name, path: name.replaceAll('\\', '/') });
      }
      return result.sort((a, b) => Number(Boolean(b.children)) - Number(Boolean(a.children)) || a.name.localeCompare(b.name));
    };
    return walk('');
  }
  async importFiles(id, sources, destination='') {
    await this.destinationDir(id,destination);
    const copied = [];
    for (const source of sources) {
      const name = path.basename(source);
      if (name.startsWith('.')) throw new Error('Hidden files cannot be imported');
      const relative=destination ? `${destination}/${name}` : name;
      const target = this.filePath(id, relative);
      try { await fs.copyFile(source, target, fs.constants.COPYFILE_EXCL); }
      catch(error) {
        if (copied.length) throw new Error(`Upload stopped after copying ${copied.join(', ')}: ${error.message}`);
        throw error;
      }
      copied.push(relative);
    }
    return copied;
  }
  async importFolder(id, source, parent='') {
    const root = this.projectDir(id), sourcePath = path.resolve(source), name = path.basename(sourcePath);
    if (!name || name.startsWith('.')) throw new Error('Invalid folder name');
    await this.destinationDir(id,parent);
    const relative=parent ? `${parent}/${name}` : name;
    const destination = this.filePath(id, relative);
    const normalize = value => value.toLowerCase() + path.sep;
    if (normalize(root).startsWith(normalize(sourcePath)) || normalize(sourcePath).startsWith(normalize(root))) throw new Error('Choose a folder outside this project');
    const stage = this.filePath(id, `.refract-upload-${crypto.randomUUID()}`);
    const copy = async (from, to) => {
      const info = await fs.lstat(from);
      if (info.isSymbolicLink()) throw new Error('Folders containing links cannot be imported');
      if (info.isDirectory()) {
        await fs.mkdir(to);
        for (const name of await fs.readdir(from)) {
          if (name.startsWith('.refract')) throw new Error('Reserved file name in folder');
          await copy(path.join(from, name), path.join(to, name));
        }
      } else if (info.isFile()) await fs.copyFile(from, to, fs.constants.COPYFILE_EXCL);
      else throw new Error('Unsupported file in folder');
    };
    try {
      // Reserve the final directory first: rename cannot safely replace existing data.
      await fs.mkdir(destination);
      try {
        await copy(sourcePath, stage);
        for (const child of await fs.readdir(stage)) await fs.rename(path.join(stage, child), path.join(destination, child));
        await fs.rmdir(stage);
      } catch (error) {
        if (stage.startsWith(root + path.sep)) await fs.rm(stage, { recursive: true, force: true });
        try { await fs.rmdir(destination); }
        catch (cleanup) { if(cleanup.code!=='ENOENT') throw new Error(`Upload stopped and ${relative} may contain partial files: ${error.message}`); }
        throw error;
      }
      return relative;
    } catch (error) { throw error; }
  }
  async files(id) {
    const dir = this.projectDir(id);
    const walk = async (relative = '') => {
      const entries = await fs.readdir(path.join(dir, relative), { withFileTypes: true });
      const result = [];
      for (const entry of entries) {
        if (entry.name.startsWith('.')) continue;
        const name = path.join(relative, entry.name);
        if (entry.isDirectory()) result.push(...await walk(name));
        else if (entry.isFile()) result.push(name.replaceAll('\\', '/'));
      }
      return result;
    };
    return walk();
  }
  async search(id, query) {
    if (typeof query !== 'string' || !query.trim()) return [];
    // Match against the original text so Unicode case folding cannot shift
    // the UTF-16 offsets used by the editor selection.
    const needle = new RegExp(query.replace(/[.*+?^${}()|[\]\\]/g,'\\$&'),'iu'), results = [];
    for (const name of await this.files(id)) {
      if (results.length >= 200) break;
      const file = this.filePath(id, name), stat = await fs.stat(file);
      if (stat.size > 2 * 1024 * 1024) continue;
      const bytes = await fs.readFile(file);
      if (bytes.includes(0)) continue;
      let text: string; try {text = new TextDecoder('utf-8',{fatal:true}).decode(bytes);} catch {continue;}
      const lines = text.split(/\r?\n/);
      for (let i = 0; i < lines.length && results.length < 200; i++) {
        const column = lines[i].search(needle);
        if (column !== -1) results.push({file:name, line:i + 1, column, text:lines[i].slice(0, 240)});
      }
    }
    return results;
  }
  async write(id, name, content) {
    const target = this.filePath(id, name);
    if(name.split(/[\\/]/).some(part=>part.startsWith('.'))) throw new Error('Invalid file name');
    const parent=name.split(/[\\/]/).slice(0,-1).join('/');
    await this.destinationDir(id,parent,true);
    const fileInfo=async () => {try {const info=await fs.lstat(target);if(!info.isFile() || info.isSymbolicLink())throw new Error('Only regular project files can be used');return info;}catch(error){if(error.code==='ENOENT')return null;throw error;}};
    const original=await fileInfo();
    const temp = `${target}.${crypto.randomUUID()}.tmp`;
    await fs.writeFile(temp, content, {encoding:'utf8',flag:'wx'});
    let originalStage: import('node:fs').Stats | null=null;
    try {
      originalStage=await fs.lstat(temp);
      const delays=[80,160,320,640];
      for(let attempt=0;;attempt++) {
        try {await fs.rename(temp,target);break;}
        catch(error) {
          if(process.platform!=='win32' || !['EPERM','EBUSY','EACCES'].includes(error.code) || attempt>=delays.length) throw error;
          await new Promise(resolve=>setTimeout(resolve,delays[attempt]));
          await this.destinationDir(id,parent);
          const staged=await fs.lstat(temp), current=await fileInfo();
          if(!staged.isFile() || staged.isSymbolicLink() || staged.nlink>1 || staged.dev!==originalStage.dev || staged.ino!==originalStage.ino || staged.size!==originalStage.size || staged.mtimeMs!==originalStage.mtimeMs || (original ? !current || current.dev!==original.dev || current.ino!==original.ino || current.size!==original.size || current.mtimeMs!==original.mtimeMs : !!current)) throw error;
        }
      }
    } catch (error) {
      try {await this.destinationDir(id,parent);const current=await fs.lstat(temp);if(originalStage && current.isFile() && !current.isSymbolicLink() && current.nlink===1 && current.dev===originalStage.dev && current.ino===originalStage.ino && current.size===originalStage.size && current.mtimeMs===originalStage.mtimeMs) await fs.unlink(temp);} catch {}
      throw error;
    }
  }
}
module.exports = { ProjectStore };
