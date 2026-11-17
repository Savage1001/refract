import fs = require('node:fs/promises');
import path = require('node:path');
import crypto = require('node:crypto');
import sources = require('./source-cache.cjs');
interface Store {files(id:string):Promise<string[]>;existingFile(id:string,name:string):Promise<string>;destinationDir(id:string):Promise<string>}
interface Options {executable:string;rootFile?:string;draftMode?:boolean;signal?:AbortSignal}
interface Result {ok:boolean;pdf?:string;log:string;exitCode?:number|null;arguments?:string[];cached?:boolean}
type RecordState = {key:string;result:Result;dependencies:Map<string,string>;created:number};
class BuildCache {
  private readonly sources=new sources.SourceCache(32*1024*1024);
  private readonly entries=new Map<string,RecordState>();
  constructor(private readonly store:Store,private readonly compile:(cwd:string,options:Options)=>Promise<Result>) {}
  private async key(id:string,options:Options):Promise<string> {
    await this.store.destinationDir(id);
    const names=(await this.store.files(id)).sort();
    const files=await sources.mapLimited(names,async name=>[name,(await this.sources.read(await this.store.existingFile(id,name))).sha256]);
    const env=Object.entries(process.env).filter(([key])=>/^(PATH|BIBINPUTS|BSTINPUTS|TEX|LUAINPUTS|OSFONTDIR|FONTCONFIG)/i.test(key)).sort(([a],[b])=>a.localeCompare(b));
    const now=new Date();
    return crypto.createHash('sha256').update(JSON.stringify([files,options.executable,options.rootFile,options.draftMode,env,[now.getFullYear(),now.getMonth(),now.getDate(),now.getHours(),now.getMinutes()]])).digest('hex');
  }
  private async dependencies(cwd:string,options:Options):Promise<Map<string,string>> {
    const folder=path.join(cwd,'.refract-build');
    const directory=await fs.lstat(folder);if(!directory.isDirectory()||directory.isSymbolicLink())throw new Error('Invalid compilation directory');
    const recorder=path.join(folder,'main.fls');
    // TeX's recorder does not capture every BibTeX/Biber input or remote data
    // source. Do not reuse bibliography builds until those can be verified too.
    for(const name of ['main.bbl','main.bcf']) {
      try {await fs.lstat(path.join(folder,name));throw new Error('Bibliography builds are not cached');}
      catch(error){if(error.code!=='ENOENT')throw error;}
    }
    const info=await fs.lstat(recorder);if(!info.isFile()||info.isSymbolicLink()||info.nlink>1||info.size>8*1024*1024)throw new Error('Invalid compiler dependency record');
    const content=await fs.readFile(recorder,'utf8');
    const files=new Set<string>([options.executable,recorder,path.join(folder,'main.pdf'),path.join(folder,'main.log'),path.join(folder,'main.synctex.gz')]);
    for(const line of content.split(/\r?\n/))if(line.startsWith('INPUT '))files.add(path.resolve(cwd,line.slice(6)));
    const entries=await sources.mapLimited([...files],async file=>{
      const info=await fs.lstat(file,{bigint:true});
      if(!info.isFile()||info.isSymbolicLink()||info.nlink>1n)throw new Error('Invalid compiler dependency');
      return [file,sources.signature(info)] as const;
    });
    return new Map(entries);
  }
  async run(id:string,cwd:string,options:Options,force=false):Promise<Result> {
    if(options.signal?.aborted)return {ok:false,log:'Compilation cancelled.',exitCode:null};
    const key=await this.key(id,options),cached=this.entries.get(id);
    if(!force && cached?.key===key && Date.now()-cached.created<60000) {
      try {
        const dependencies=await this.dependencies(cwd,options);
        if(dependencies.size===cached.dependencies.size && [...dependencies].every(([file,stamp])=>cached.dependencies.get(file)===stamp)) {
          this.entries.delete(id);this.entries.set(id,cached);return {...cached.result,cached:true};
        }
      } catch { /* Missing/replaced outputs and dependencies require a real build. */ }
    }
    this.entries.delete(id);
    const result=await this.compile(cwd,options);
    if(result.ok && !options.signal?.aborted && (result.pdf?.length || 0)<16*1024*1024) {
      try {
        const dependencies=await this.dependencies(cwd,options);
        if(await this.key(id,options)===key) {
          if(this.entries.size>=3)this.entries.delete(this.entries.keys().next().value!);
          this.entries.set(id,{key,result,dependencies,created:Date.now()});
        }
      } catch { /* A valid build is still returned when dependency recording cannot be cached. */ }
    }
    return {...result,cached:false};
  }
}
export = {BuildCache};
