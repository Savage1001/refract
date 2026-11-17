const {test}=require('node:test'), assert=require('node:assert/strict'), fs=require('node:fs/promises'), path=require('node:path'), os=require('node:os');
const {ProjectStore}=require('../src/project-store.cjs'),{inspectFile,MAX_PREVIEW_BYTES}=require('../src/file-preview.cjs');
test('project files are classified without decoding binary as text',async t=>{
 const root=await fs.mkdtemp(path.join(os.tmpdir(),'refract-preview-'));t.after(()=>fs.rm(root,{recursive:true,force:true}));
 const store=new ProjectStore(root),p=await store.create('Paper');
 const put=async(name,bytes)=>fs.writeFile(store.filePath(p.id,name),bytes);
 await put('note.txt',Buffer.from('Plain UTF-8 ✓')); assert.deepEqual(await inspectFile(store,p.id,'note.txt'),{kind:'text',text:'Plain UTF-8 ✓'});
 await put('bom.tex',Buffer.from([0xef,0xbb,0xbf,0x41])); assert.equal((await inspectFile(store,p.id,'bom.tex')).text,'\ufeffA');
 await put('bad.tex',Buffer.from([0x80,0xff,0x41])); assert.equal((await inspectFile(store,p.id,'bad.tex')).kind,'unsupported');
 await put('binary.dat',Buffer.from([0x41,0,0x42])); assert.equal((await inspectFile(store,p.id,'binary.dat')).kind,'unsupported');
 await put('program.exe',Buffer.from('printable bytes')); assert.equal((await inspectFile(store,p.id,'program.exe')).kind,'unsupported');
 await put('raw.bin',Buffer.from('printable bytes')); assert.equal((await inspectFile(store,p.id,'raw.bin')).kind,'unsupported');
 await put('paper.dat',Buffer.from('%PDF-1.4\n')); assert.equal((await inspectFile(store,p.id,'paper.dat')).kind,'pdf');
 await put('diagram.svg',Buffer.from('<svg xmlns="http://www.w3.org/2000/svg"/>')); assert.match((await inspectFile(store,p.id,'diagram.svg')).data,/^data:image\/svg\+xml;base64,/);
 const large=await fs.open(store.filePath(p.id,'large.txt'),'w'); await large.truncate(MAX_PREVIEW_BYTES+1); await large.close();
 assert.match((await inspectFile(store,p.id,'large.txt')).reason,/too large/i);
 await assert.rejects(inspectFile(store,p.id,'../escape'));
});

test('file previews reject linked paths',async t=>{
 const root=await fs.mkdtemp(path.join(os.tmpdir(),'refract-preview-link-'));t.after(()=>fs.rm(root,{recursive:true,force:true}));
 const store=new ProjectStore(path.join(root,'projects')),p=await store.create('Paper'), outside=path.join(root,'outside');
 await fs.mkdir(outside);await fs.writeFile(path.join(outside,'secret.txt'),'secret');
 try {await fs.symlink(outside,store.filePath(p.id,'link'),process.platform==='win32'?'junction':'dir');}
 catch(error){if(['EPERM','EACCES'].includes(error.code)) return t.skip('Creating directory links is not permitted');throw error;}
 await assert.rejects(inspectFile(store,p.id,'link/secret.txt'),/linked folders/);
});

test('preview bounds reads when a file grows after its size check',async t=>{
 const root=await fs.mkdtemp(path.join(os.tmpdir(),'refract-preview-growth-'));t.after(()=>fs.rm(root,{recursive:true,force:true}));
 const store=new ProjectStore(root),p=await store.create('Paper'), target=store.filePath(p.id,'growing.txt');
 await fs.writeFile(target,'small');
 const originalOpen=fs.open;
 t.mock.method(fs,'open',async (...args)=>{
  if (args[0]===target && args[1]==='r') {
   const writer=await originalOpen(target,'r+');
   try {await writer.truncate(MAX_PREVIEW_BYTES+1);} finally {await writer.close();}
  }
  return originalOpen(...args);
 });
 assert.match((await inspectFile(store,p.id,'growing.txt')).reason,/too large/i);
});
