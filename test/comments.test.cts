const {test}=require('node:test'),assert=require('node:assert/strict'),fs=require('node:fs/promises'),path=require('node:path'),os=require('node:os');
const {ProjectStore}=require('../src/project-store.cjs'),{Comments}=require('../src/comments.cjs');
test('comments persist with selection and resolved state',async t=>{
 const root=await fs.mkdtemp(path.join(os.tmpdir(),'refract-comments-'));t.after(()=>fs.rm(root,{recursive:true,force:true}));
 const store=new ProjectStore(root),p=await store.create('Paper'),comments=new Comments(store);
 const item=await comments.add(p.id,'main.tex','Introduction','Review this'); await comments.resolve(p.id,item.id,true);
 const saved=(await new Comments(store).list(p.id))[0];assert.equal(saved.quote,'Introduction');assert.equal(saved.resolved,true);
 await assert.rejects(comments.add(p.id,'../outside','','bad'));
});

test('comment storage rejects linked project roots and hard-linked data',async t=>{
 const root:string=await fs.mkdtemp(path.join(os.tmpdir(),'refract-comment-links-'));
 t.after(async()=>{const resolved=await fs.realpath(root),parent=await fs.realpath(os.tmpdir());if(path.dirname(resolved).toLowerCase()!==parent.toLowerCase() || !path.basename(resolved).startsWith('refract-comment-links-'))throw new Error('Unsafe cleanup path');await fs.rm(resolved,{recursive:true,force:true});});
 const store=new ProjectStore(path.join(root,'projects')),p=await store.create('Paper'),comments=new Comments(store),file=comments.file(p.id),outside=path.join(root,'outside.json');
 await fs.writeFile(outside,'[{"text":"preserve"}]');await fs.link(outside,file);
 await assert.rejects(comments.list(p.id),/Invalid comments file/);await assert.rejects(comments.save(p.id,[]),/Invalid comments file/);assert.equal(await fs.readFile(outside,'utf8'),'[{"text":"preserve"}]');
 await fs.unlink(file);const outsideDir=path.join(root,'outside-project'),project=store.projectDir(p.id),saved=path.join(root,'saved-project');await fs.mkdir(outsideDir);await fs.writeFile(path.join(outsideDir,'.refract-comments.json'),'outside');await fs.rename(project,saved);
 try {await fs.symlink(outsideDir,project,process.platform==='win32'?'junction':'dir');}catch(error){if(['EPERM','EACCES'].includes(error.code))return t.skip('Creating directory links is not permitted');throw error;}
 try {await assert.rejects(comments.list(p.id),/Invalid project directory/);await assert.rejects(comments.save(p.id,[]),/Invalid project directory/);assert.equal(await fs.readFile(path.join(outsideDir,'.refract-comments.json'),'utf8'),'outside');}finally{await fs.unlink(project);await fs.rename(saved,project);}
});

test('failed comment save retains the previous data',async t=>{
 const root:string=await fs.mkdtemp(path.join(os.tmpdir(),'refract-comment-failure-'));
 t.after(async()=>{const resolved=await fs.realpath(root),parent=await fs.realpath(os.tmpdir());if(path.dirname(resolved).toLowerCase()!==parent.toLowerCase() || !path.basename(resolved).startsWith('refract-comment-failure-'))throw new Error('Unsafe cleanup path');await fs.rm(resolved,{recursive:true,force:true});});
 const store=new ProjectStore(root),p=await store.create('Paper'),comments=new Comments(store);await comments.save(p.id,[{text:'preserved'}]);const rename=fs.rename;
 fs.rename=async(from,to)=>{if(to===comments.file(p.id))throw new Error('simulated save failure');return rename(from,to);};
 try {await assert.rejects(comments.save(p.id,[{text:'new'}]),/simulated save failure/);}finally{fs.rename=rename;}
 assert.deepEqual(await comments.list(p.id),[{text:'preserved'}]);assert.deepEqual((await fs.readdir(store.projectDir(p.id))).filter(name=>name.endsWith('.tmp')),[]);
});

test('anchors track repeated selections across edits, renames and restart',async t=>{
 const root=await fs.mkdtemp(path.join(os.tmpdir(),'refract-comment-anchor-'));t.after(()=>fs.rm(root,{recursive:true,force:true}));
 const store=new ProjectStore(root),p=await store.create('Paper'),comments=new Comments(store);
 let source='😀 first repeat here. Second repeat here. End.'; await store.write(p.id,'main.tex',source);
 const start=source.lastIndexOf('repeat');
 const item=await comments.add(p.id,'main.tex','repeat','Second occurrence',{start,end:start+6});
 assert.deepEqual(await comments.locate(p.id,item.id),{start,end:start+6});
 const edit=async value=>{const updated=await comments.planEdit(p.id,'main.tex',source,value);await store.write(p.id,'main.tex',value);if(updated)await comments.save(p.id,updated);source=value;};
 await edit('Prefix '+source);
 let position=await comments.locate(p.id,item.id); assert.equal(position.start,source.lastIndexOf('repeat'));
 const at=position.start; await edit(source.slice(0,at+3)+'XYZ'+source.slice(at+3));
 position=await comments.locate(p.id,item.id); assert.equal(source.slice(position.start,position.end),'repXYZeat');
 await store.renameFile(p.id,'main.tex','renamed.tex'); await comments.renameFile(p.id,'main.tex','renamed.tex');
 const reopened=new Comments(store), saved=(await reopened.list(p.id))[0];
 assert.equal(saved.file,'renamed.tex'); assert.equal(saved.quote,'repXYZeat');
 assert.deepEqual(await reopened.locate(p.id,item.id),position);
});

test('deleted and ambiguous selections do not jump to another occurrence',async t=>{
 const root=await fs.mkdtemp(path.join(os.tmpdir(),'refract-comment-lost-'));t.after(()=>fs.rm(root,{recursive:true,force:true}));
 const store=new ProjectStore(root),p=await store.create('Paper'),comments=new Comments(store);
 const source='aaa target bbb --- ccc target ddd', at=source.lastIndexOf('target'); await store.write(p.id,'main.tex',source);
 const item=await comments.add(p.id,'main.tex','target','Review',{start:at,end:at+6});
 const changed=source.slice(0,at)+source.slice(at+6), updated=await comments.planEdit(p.id,'main.tex',source,changed);
 await store.write(p.id,'main.tex',changed); await comments.save(p.id,updated);
 assert.equal((await comments.list(p.id))[0].anchor.status,'orphaned'); assert.equal(await comments.locate(p.id,item.id),null);
 const legacy={id:'legacy',file:'main.tex',quote:'target',text:'Legacy',resolved:false};
 await store.write(p.id,'main.tex','target and target'); await comments.save(p.id,[legacy]);
 assert.equal(await comments.locate(p.id,'legacy'),null);
});

test('context can locate an anchor after an external edit and rejects stale selections',async t=>{
 const root=await fs.mkdtemp(path.join(os.tmpdir(),'refract-comment-context-'));t.after(()=>fs.rm(root,{recursive:true,force:true}));
 const store=new ProjectStore(root),p=await store.create('Paper'),comments=new Comments(store);
 const source='first target here, unique context: target followed by details'; await store.write(p.id,'main.tex',source);
 const start=source.lastIndexOf('target'); const item=await comments.add(p.id,'main.tex','target','Review',{start,end:start+6});
 await assert.rejects(comments.add(p.id,'main.tex','target','bad',{start:0,end:6}),/selected text has changed/);
 const changed='external edit before text; '+source; await store.write(p.id,'main.tex',changed);
 const position=await comments.locate(p.id,item.id); assert.equal(position.start,changed.lastIndexOf('target'));
});

test('anchors use textarea offsets for CRLF source files',async t=>{
 const root=await fs.mkdtemp(path.join(os.tmpdir(),'refract-comment-crlf-'));t.after(()=>fs.rm(root,{recursive:true,force:true}));
 const store=new ProjectStore(root),p=await store.create('Paper'),comments=new Comments(store);
 await fs.writeFile(store.filePath(p.id,'main.tex'),'first\r\nsecond target\r\n');
 const normalized='first\nsecond target\n', start=normalized.indexOf('target');
 const item=await comments.add(p.id,'main.tex','target','Review',{start,end:start+6});
 assert.deepEqual(await comments.locate(p.id,item.id),{start,end:start+6});
});

test('comment replies persist in order and support existing comments',async t=>{
 const root=await fs.mkdtemp(path.join(os.tmpdir(),'refract-replies-'));t.after(()=>fs.rm(root,{recursive:true,force:true}));
 const store=new ProjectStore(root),p=await store.create('Paper'),comments=new Comments(store);
 const item=await comments.add(p.id,'main.tex','Introduction','Original');
 await comments.reply(p.id,item.id,' First reply '); await comments.reply(p.id,item.id,'Second reply');
 await assert.rejects(comments.reply(p.id,item.id,'  '),/empty/); await assert.rejects(comments.reply(p.id,'missing','Reply'),/not found/);
 const saved=(await new Comments(store).list(p.id))[0]; assert.deepEqual(saved.replies.map(r=>r.text),['First reply','Second reply']);
 await comments.resolve(p.id,item.id,true); assert.equal((await comments.list(p.id))[0].replies.length,2);
});

test('anchors survive separate edits on both sides without jumping to deleted duplicates', async t => {
 const root: string=await fs.mkdtemp(path.join(os.tmpdir(),'refract-comment-multiedit-'));t.after(()=>fs.rm(root,{recursive:true,force:true}));
 const store=new ProjectStore(root),project=await store.create('Paper'),comments=new Comments(store);
 const source='Opening text. First target is ordinary. Unique lead before target and unique trailing context. Closing text.';
 await store.write(project.id,'main.tex',source);
 const start=source.lastIndexOf('target'),item=await comments.add(project.id,'main.tex','target','Review',{start,end:start+6});
 const changed='New opening. '+source+' New ending.';
 const updated=await comments.planEdit(project.id,'main.tex',source,changed);await store.write(project.id,'main.tex',changed);await comments.save(project.id,updated);
 const position=await comments.locate(project.id,item.id);assert.equal(position.start,changed.lastIndexOf('target'));
 const removed=changed.slice(0,position.start)+changed.slice(position.end);
 const deleted=await comments.planEdit(project.id,'main.tex',changed,removed);await store.write(project.id,'main.tex',removed);await comments.save(project.id,deleted);
 assert.equal(await comments.locate(project.id,item.id),null);
});

test('nested folder moves preserve comment anchors without moving similarly named folders', async t => {
 const root: string=await fs.mkdtemp(path.join(os.tmpdir(),'refract-comment-folder-'));t.after(()=>fs.rm(root,{recursive:true,force:true}));
 const store=new ProjectStore(root),project=await store.create('Paper'),comments=new Comments(store);
 const source='First target here. Another target is reviewed.', start=source.lastIndexOf('target');
 await store.write(project.id,'chapters/deep/main.tex',source);
 await store.write(project.id,'chapters-other/main.tex',source);
 const item=await comments.add(project.id,'chapters/deep/main.tex','target','Review',{start,end:start+6});
 const other=await comments.add(project.id,'chapters-other/main.tex','target','Other',{start,end:start+6});
 await store.moveFolder(project.id,'chapters','drafts');await comments.moveFolder(project.id,'chapters','drafts');
 const reopened=new Comments(store),saved=await reopened.list(project.id);
 assert.equal(saved.find(comment=>comment.id===item.id).file,'drafts/deep/main.tex');
 assert.equal(saved.find(comment=>comment.id===other.id).file,'chapters-other/main.tex');
 assert.deepEqual(await reopened.locate(project.id,item.id),{start,end:start+6});
});


test('file renames update anchors when path casing differs', async t => {
 const root: string=await fs.mkdtemp(path.join(os.tmpdir(),'refract-comment-rename-'));t.after(()=>fs.rm(root,{recursive:true,force:true}));
 const store=new ProjectStore(root),project=await store.create('Paper'),comments=new Comments(store);
 const source='A selected passage.', start=source.indexOf('selected');
 await store.write(project.id,'main.tex',source);
 const item=await comments.add(project.id,'main.tex','selected','Review',{start,end:start+8});
 await store.renameFile(project.id,'main.tex','renamed.tex');
 await comments.renameFile(project.id,'MAIN.TEX','renamed.tex');
 const reopened=new Comments(store);
 assert.equal((await reopened.list(project.id))[0].file,'renamed.tex');
 assert.deepEqual(await reopened.locate(project.id,item.id),{start,end:start+8});
});

test('anchors retain selections across edits inside and around them, and orphan partial overlaps', async t => {
 const root: string=await fs.mkdtemp(path.join(os.tmpdir(),'refract-comment-complex-'));t.after(()=>fs.rm(root,{recursive:true,force:true}));
 const store=new ProjectStore(root),project=await store.create('Paper'),comments=new Comments(store);
 let source='Opening words. Carefully selected phrase here. Closing words.';
 await store.write(project.id,'main.tex',source);
 const start=source.indexOf('selected phrase'),item=await comments.add(project.id,'main.tex','selected phrase','Review',{start,end:start+15});
 const edit=async (next:string)=>{const plan=await comments.planEdit(project.id,'main.tex',source,next);await store.write(project.id,'main.tex',next);if(plan)await comments.save(project.id,plan);source=next;};
 await edit(source.replace('selected phrase','selected revised phrase'));
 await edit('New beginning. '+source+' New ending.');
 const position=await new Comments(store).locate(project.id,item.id);
 assert.equal(source.slice(position.start,position.end),'selected revised phrase');
 await edit(source.slice(0,position.start-2)+source.slice(position.start+5));
 assert.equal(await new Comments(store).locate(project.id,item.id),null);
 assert.equal((await comments.list(project.id))[0].anchor.status,'orphaned');
});

test('missing files and failed comment operations preserve the thread', async t => {
 const root: string=await fs.mkdtemp(path.join(os.tmpdir(),'refract-comment-errors-'));t.after(()=>fs.rm(root,{recursive:true,force:true}));
 const store=new ProjectStore(root),project=await store.create('Paper'),comments=new Comments(store);
 const item=await comments.add(project.id,'main.tex','Introduction','Review');
 await comments.reply(project.id,item.id,'Reply');
 await assert.rejects(comments.resolve(project.id,'missing',true),/not found/);
 await assert.rejects(comments.add(project.id,'main.tex','Introduction',' ',{start:0,end:12}),/empty/);
 await fs.unlink(store.filePath(project.id,'main.tex'));
 assert.equal(await comments.locate(project.id,item.id),null);
 const saved=(await new Comments(store).list(project.id))[0];
 assert.equal(saved.resolved,false);assert.equal(saved.replies[0].text,'Reply');
});
