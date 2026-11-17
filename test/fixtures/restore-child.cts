const fs = require('node:fs/promises');
const path = require('node:path');
const {ProjectStore} = require('../../src/project-store.cjs');
const {Versions} = require('../../src/versions.cjs');
const [root, id, version, point = 'file'] = process.argv.slice(2);
const store = new ProjectStore(root), versions = new Versions(store), destination = path.join(store.projectDir(id),'a.txt');
const rename = fs.rename, unlink=fs.unlink, mkdir=fs.mkdir, writeFile=fs.writeFile, project=store.projectDir(id);
fs.rename = async (from: string, to: string) => {
 await rename(from,to);
 if ((point==='file' && to===destination) || (point==='blocker' && from===path.join(project,'item')) || (point==='metadata' && to===path.join(project,'.refract.json'))) process.exit(73);
 if (point==='journal' && to===path.join(project,'.refract-restore-journal.json')) process.exit(73);
};
fs.unlink=async (file:string)=>{await unlink(file);if(point==='delete' && file===path.join(project,'added.txt')) process.exit(73);const parts=path.relative(project,file).split(path.sep);if(point==='cleanup' && parts.length>1 && /^\.refract-restore-[0-9a-f-]{36}$/.test(parts[0]))process.exit(73);};
fs.mkdir=async (directory:string,...args:any[])=>{const result=await mkdir(directory,...args);if(point==='mkdir' && /^\.refract-restore-[0-9a-f-]{36}$/.test(path.basename(directory)))process.exit(73);return result;};
fs.writeFile=async (file:string,...args:any[])=>{const result=await writeFile(file,...args);const parts=path.relative(project,file).split(path.sep);if(point==='stage-write' && parts.length===2 && /^\.refract-restore-[0-9a-f-]{36}$/.test(parts[0]) && /^\d+$/.test(parts[1]))process.exit(73);return result;};
versions.restore(id,version).then(() => process.exit(0), (error: Error) => { console.error(error); process.exit(1); });
