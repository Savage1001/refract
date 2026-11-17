const fs = require('node:fs/promises');
const path = require('node:path');
const {execFile} = require('node:child_process');
const {promisify} = require('node:util');
const os = require('node:os');
const zlib = require('node:zlib');
const execute = promisify(execFile);
let ansiDecoder:Promise<TextDecoder>|null=null;

type SyncPosition = {page:number; x:number; y:number};
type SourcePosition = {file:string; line:number};

async function syncOutput(project:string):Promise<string> {
 const root=await fs.lstat(project);if(root.isSymbolicLink() || !root.isDirectory())throw new Error('Invalid project directory');
 const dir=path.join(project,'.refract-build'),info=await fs.lstat(dir);
 if(info.isSymbolicLink() || !info.isDirectory())throw new Error('Invalid compilation directory');
 for(const name of ['main.pdf','main.synctex.gz']) {const stat=await fs.lstat(path.join(dir,name));if(stat.isSymbolicLink() || !stat.isFile())throw new Error('Invalid synchronization data');}
 return path.join(dir,'main.pdf');
}
function fields(output:string):Map<string,string> {
 const section=output.match(/SyncTeX result begin\r?\n([\s\S]*?)SyncTeX result end/);
 if(!section)throw new Error('No matching source or PDF position');
 const values=new Map<string,string>();for(const line of section[1].split(/\r?\n/)){const split=line.indexOf(':');if(split>0 && !values.has(line.slice(0,split)))values.set(line.slice(0,split),line.slice(split+1));}return values;
}
async function decode(bytes:Buffer):Promise<string> {
 let text:string;
 try {text=new TextDecoder('utf-8',{fatal:true}).decode(bytes);}
 catch(error) {
  if(process.platform!=='win32')throw error;
  ansiDecoder ||= (async()=>{
   const result=await execute('reg.exe',['query','HKLM\\SYSTEM\\CurrentControlSet\\Control\\Nls\\CodePage','/v','ACP'],{windowsHide:true,timeout:5000});
   const code=String(result.stdout).match(/\bACP\s+REG_\w+\s+(\d+)/i)?.[1];if(!code)throw new Error('Could not determine the Windows text encoding');
   const label=({ '65001':'utf-8','932':'shift_jis','936':'gbk','949':'euc-kr','950':'big5' } as Record<string,string>)[code] || `windows-${code}`;
   return new TextDecoder(label,{fatal:true});
  })();
  try {text=(await ansiDecoder).decode(bytes);} catch(error) {ansiDecoder=null;throw error;}
 }
 return text;
}
// SyncTeX can emit Windows-encoded output paths and UTF-8 source paths in the
// same response. Decode each line independently rather than guessing once.
async function decodeLines(bytes:Buffer):Promise<string> {
 const lines:string[]=[];let start=0;
 for(let i=0;i<bytes.length;i++)if(bytes[i]===10){lines.push(await decode(bytes.subarray(start,i)));start=i+1;}
 lines.push(await decode(bytes.subarray(start)));return lines.join('\n');
}
async function query(executable:string,project:string,args:string[]):Promise<Map<string,string>> {
 const {stdout}=await execute(executable,args,{cwd:project,windowsHide:true,timeout:15000,maxBuffer:1024*1024,encoding:'buffer'});return fields(await decodeLines(stdout));
}
async function forwardUnicode(executable:string,project:string,file:string,line:number,pdf:string):Promise<Map<string,string>> {
 const syncPath=path.join(path.dirname(pdf),'main.synctex.gz'),info=await fs.stat(syncPath);
 if(info.size>64*1024*1024)throw new Error('SyncTeX data is too large');
 const compressed=await fs.readFile(syncPath);
 const source:Buffer=zlib.gunzipSync(compressed,{maxOutputLength:64*1024*1024});
 const wanted=path.resolve(project,file).toLowerCase(),chunks:Buffer[]=[];let found=false,start=0,last=0;
 while(start<source.length) {
  const newline=source.indexOf(10,start),end=newline<0?source.length:newline;
  if(source.subarray(start,start+6).toString('ascii')==='Input:') {
   const second=source.indexOf(58,start+6);
   if(second>=0 && second<end && /^\d+$/.test(source.subarray(start+6,second).toString('ascii'))) {
    const contentEnd=end>second+1 && source[end-1]===13?end-1:end,input=await decode(source.subarray(second+1,contentEnd));
    if(path.resolve(project,input).toLowerCase()===wanted) {chunks.push(source.subarray(last,second+1),Buffer.from('refract-sync-source.tex'));last=contentEnd;found=true;}
   }
  }
  start=newline<0?source.length:newline+1;
 }
 const rewritten=Buffer.concat([...chunks,source.subarray(last)]);
 if(!found)throw new Error('No matching source or PDF position');
 const parent=await fs.realpath(os.tmpdir()),dir=await fs.mkdtemp(path.join(parent,'refract-synctex-query-'));
 const syncFile=path.join(dir,'main.synctex.gz'),pdfFile=path.join(dir,'main.pdf');
 try {
  await fs.writeFile(syncFile,zlib.gzipSync(rewritten),{flag:'wx'});await fs.writeFile(pdfFile,Buffer.alloc(0),{flag:'wx'});
  return await query(executable,project,['view','-i',`${line}:0:refract-sync-source.tex`,'-o',pdfFile]);
 } finally {
  const resolved=await fs.realpath(dir);if(path.dirname(resolved).toLowerCase()!==parent.toLowerCase() || !path.basename(resolved).startsWith('refract-synctex-query-'))throw new Error('Unsafe SyncTeX cleanup path');
  for(const name of ['main.synctex.gz','main.pdf'])await fs.unlink(path.join(resolved,name)).catch(error=>{if(error.code!=='ENOENT')throw error;});await fs.rmdir(resolved);
 }
}
async function forward(executable:string,project:string,file:string,line:number):Promise<SyncPosition> {
 if(typeof file!=='string' || !file || /[\r\n:]/.test(file) || path.isAbsolute(file) || file.split(/[\\/]/).some(part=>!part || part==='.' || part==='..' || part.startsWith('.')) || !Number.isSafeInteger(line) || line<1 || line>10000000)throw new Error('Invalid source position');
 const pdf=await syncOutput(project);let result:Map<string,string>;
 try {result=await query(executable,project,['view','-i',`${line}:0:${file}`,'-o',pdf]);}
 catch(error) {if(!/[^\x00-\x7f]/.test(file) || error.message!=='No matching source or PDF position')throw error;result=await forwardUnicode(executable,project,file,line,pdf);}
 const page=Number(result.get('Page')),x=Number(result.get('x')),y=Number(result.get('y'));
 if(!Number.isSafeInteger(page) || page<1 || !Number.isFinite(x) || !Number.isFinite(y))throw new Error('Invalid SyncTeX result');
 return {page,x,y};
}
async function reverse(executable:string,project:string,page:number,x:number,y:number):Promise<SourcePosition> {
 if(!Number.isSafeInteger(page) || page<1 || page>1000000 || !Number.isFinite(x) || !Number.isFinite(y) || x<0 || y<0 || x>1000000 || y>1000000)throw new Error('Invalid PDF position');
 const pdf=await syncOutput(project),result=await query(executable,project,['edit','-o',`${page}:${x}:${y}:${pdf}`]);
 const input=result.get('Input'),line=Number(result.get('Line'));if(!input || !Number.isSafeInteger(line) || line<1)throw new Error('Invalid SyncTeX result');
 const relative=path.relative(project,path.resolve(project,input));
 if(!relative || path.isAbsolute(relative) || relative.split(/[\\/]/).some(part=>!part || part==='..' || part.startsWith('.')))throw new Error('SyncTeX source is outside the project');
 return {file:relative.split(path.sep).join('/'),line};
}
module.exports={forward,reverse};
