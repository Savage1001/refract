import fs = require('node:fs/promises');
import path = require('node:path');
import crypto = require('node:crypto');

// Local application state, never project metadata or archive contents.
const warning = 'Trust allows automatic compilation and assistant execution of this project and its instructions. TeX (including LuaLaTeX), bibliography tools, and assistants run under your Windows account without a filesystem sandbox. They may read files outside this project, write files, and use network access where supported. -no-shell-escape does not confine file access. Trust only content and authors you know. Future edits, restored versions, and uploaded files in this project share this trust.';
function recordPath(project:string):string {
  const resolved=path.resolve(project);
  return path.join(path.dirname(resolved),'.refract-local-trust',crypto.createHash('sha256').update(resolved).digest('hex')+'.json');
}
async function identity(project:string):Promise<string> {
  const info=await fs.lstat(project,{bigint:true});
  if(!info.isDirectory() || info.isSymbolicLink()) throw new Error('Invalid project directory');
  return [await fs.realpath(project),info.dev,info.ino,info.birthtimeNs].join(':');
}
async function isTrusted(project:string):Promise<boolean> {
  try {
    const file=recordPath(project),dir=await fs.lstat(path.dirname(file)),info=await fs.lstat(file);
    if(!dir.isDirectory() || dir.isSymbolicLink() || !info.isFile() || info.isSymbolicLink() || info.nlink!==1)return false;
    const record:unknown=JSON.parse(await fs.readFile(file,'utf8'));
    return typeof record==='object' && record!==null && 'identity' in record && record.identity===await identity(project);
  } catch {return false;}
}
async function assertTrusted(project:string):Promise<void> {
  if(!await isTrusted(project))throw new Error('Project is untrusted. Review project trust in Refract before compiling or running an assistant.');
}
// Called only for the built-in new-project template or a human consent dialog.
async function grantTrust(project:string):Promise<void> {
  const stamp=await identity(project),file=recordPath(project),folder=path.dirname(file);
  await fs.mkdir(folder,{recursive:true});
  const dir=await fs.lstat(folder);if(!dir.isDirectory() || dir.isSymbolicLink())throw new Error('Invalid local trust directory');
  const temp=file+'.'+crypto.randomUUID()+'.tmp';
  try {await fs.writeFile(temp,JSON.stringify({identity:stamp}),{flag:'wx'});await fs.rename(temp,file);}
  finally {await fs.rm(temp,{force:true});}
}
async function revokeTrust(project:string):Promise<void> {await fs.rm(recordPath(project),{force:true});}
async function requestTrust(project:string, confirm:(message:string)=>Promise<boolean>):Promise<boolean> {
  const before=await identity(project);
  if(!await confirm(warning))return false;
  if(before!==await identity(project))throw new Error('Project changed during trust review');
  await grantTrust(project);return true;
}
export = {warning,isTrusted,assertTrusted,grantTrust,revokeTrust,requestTrust};
