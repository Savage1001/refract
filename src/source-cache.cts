import fs = require('node:fs/promises');
import crypto = require('node:crypto');
type Data = {data:Buffer;sha256:string};
type Entry = Data & {signature:string};
function signature(info:import('node:fs').BigIntStats): string {
  return [info.dev,info.ino,info.size,info.mtimeNs,info.ctimeNs,info.birthtimeNs,info.mode,info.nlink].join(':');
}
class SourceCache {
  private readonly entries=new Map<string,Entry>();
  private bytes=0;
  private reads=0;private bytesRead=0;private hits=0;
  constructor(private readonly limit=64*1024*1024) {}
  async read(file:string):Promise<Data> {
    const info=await fs.lstat(file,{bigint:true});
    if(!info.isFile() || info.isSymbolicLink())throw new Error('Expected a regular project file');
    const key=signature(info), cached=this.entries.get(file);
    if(cached?.signature===key && info.nlink===1n){this.hits++;this.entries.delete(file);this.entries.set(file,cached);return cached;}
    if(cached){this.bytes-=cached.data.length;this.entries.delete(file);}
    const data=await fs.readFile(file), after=await fs.lstat(file,{bigint:true});
    this.reads++;this.bytesRead+=data.length;
    if(signature(after)!==key)throw new Error('Project file changed while capturing its state; retry');
    const entry={data,sha256:crypto.createHash('sha256').update(data).digest('hex'),signature:key};
    if(data.length<=this.limit && info.nlink===1n) {
      while(this.bytes+data.length>this.limit && this.entries.size){const oldest=this.entries.keys().next().value!;this.bytes-=this.entries.get(oldest)!.data.length;this.entries.delete(oldest);}
      this.entries.set(file,entry);this.bytes+=data.length;
    }
    return entry;
  }
  statistics(){return {filesRead:this.reads,bytesRead:this.bytesRead,cacheHits:this.hits};}
}
async function mapLimited<T,R>(items:T[],work:(item:T)=>Promise<R>,concurrency=8):Promise<R[]> {
  const results:R[]=new Array(items.length);let next=0;
  await Promise.all(Array.from({length:Math.min(concurrency,items.length)},async()=>{for(;;){const index=next++;if(index>=items.length)return;results[index]=await work(items[index]);}}));
  return results;
}
export = {SourceCache,mapLimited,signature};
