const {test}=require('node:test'),assert=require('node:assert/strict'),fs=require('node:fs/promises'),path=require('node:path'),os=require('node:os');
const {ProjectStore}=require('../src/project-store.cjs'),{Versions}=require('../src/versions.cjs'),{AssistantCoordinator}=require('../src/assistant-coordinator.cjs');
function queue(){let tail=Promise.resolve();return fn=>{const result=tail.catch(()=>{}).then(fn);tail=result;return result;};}
function deferred(){let resolve;const promise=new Promise(r=>resolve=r);return {promise,resolve};}

test('assistant operations snapshot before and after changes and remain tracked until complete',async t=>{
 const root=await fs.mkdtemp(path.join(os.tmpdir(),'refract-assistant-'));t.after(()=>fs.rm(root,{recursive:true,force:true}));
 const store=new ProjectStore(root),p=await store.create('Paper'),versions=new Versions(store),started=deferred(),release=deferred();
 const coordinator=new AssistantCoordinator(queue(),async id=>store.metadata(id),id=>versions.snapshot(id));
 const run=coordinator.run(p.id,async()=>{started.resolve();await release.promise;await store.write(p.id,'main.tex','Assistant edit');return 'done';});
 assert.equal(coordinator.isBusy(p.id),true);assert.throws(()=>coordinator.run(p.id,async()=>{}),/already running/);
 await started.promise;const before=(await versions.list(p.id))[0];assert.match((await versions.readFile(p.id,before.id,'main.tex')).toString(),/Introduction/);
 let finished=false;const wait=coordinator.waitForAll().then(()=>{finished=true;});await Promise.resolve();assert.equal(finished,false);
 release.resolve();assert.equal(await run,'done');await wait;assert.equal(coordinator.isBusy(p.id),false);
 const history=await versions.list(p.id);assert.equal(history.length,2);assert.equal((await versions.readFile(p.id,history[0].id,'main.tex')).toString(),'Assistant edit');
});

test('assistant failures still snapshot changes and preparation failures do not run',async t=>{
 const root=await fs.mkdtemp(path.join(os.tmpdir(),'refract-assistant-fail-'));t.after(()=>fs.rm(root,{recursive:true,force:true}));
 const store=new ProjectStore(root),p=await store.create('Paper'),versions=new Versions(store);
 const coordinator=new AssistantCoordinator(queue(),async id=>store.metadata(id),id=>versions.snapshot(id));
 await assert.rejects(coordinator.run(p.id,async()=>{await store.write(p.id,'main.tex','Partial edit');throw new Error('runner failed');}),/runner failed/);
 const history=await versions.list(p.id);assert.equal(history.length,2);assert.equal((await versions.readFile(p.id,history[0].id,'main.tex')).toString(),'Partial edit');
 let ran=false;const blocked=new AssistantCoordinator(queue(),async()=>{throw new Error('prepare failed');},async()=>{});
 await assert.rejects(blocked.run(p.id,async()=>{ran=true;}),/prepare failed/);assert.equal(ran,false);assert.equal(blocked.isBusy(p.id),false);
});

test('a failed recovery snapshot is reported and releases the project',async()=>{
 let calls=0;const coordinator=new AssistantCoordinator(queue(),async()=>{},async()=>{if(++calls===2)throw new Error('disk full');});
 await assert.rejects(coordinator.run('p',async()=>42),/operation finished, but its recovery snapshot failed: disk full/);
 assert.equal(coordinator.isBusy('p'),false);
});

test('cancellation reaches work and preserves a snapshot of partial changes',async t=>{
 const root=await fs.mkdtemp(path.join(os.tmpdir(),'refract-cancel-'));t.after(()=>fs.rm(root,{recursive:true,force:true}));
 const store=new ProjectStore(root),p=await store.create('Paper'),versions=new Versions(store),started=deferred();
 const coordinator=new AssistantCoordinator(queue(),async()=>{},id=>versions.snapshot(id));
 const operation=coordinator.run(p.id,async signal=>{
  await store.write(p.id,'main.tex','partial edit');started.resolve();
  await new Promise<void>(resolve=>signal.addEventListener('abort',()=>resolve(),{once:true}));
 });
 await started.promise;assert.equal(coordinator.cancel(p.id),true);await operation;
 assert.equal(coordinator.cancel(p.id),false);
 const history=await versions.list(p.id);assert.equal((await versions.readFile(p.id,history[0].id,'main.tex')).toString(),'partial edit');
});

test('cancellation requested during preparation reaches the work before it starts',async()=>{
 const preparing=deferred(),release=deferred(),snapshots=[];
 const coordinator=new AssistantCoordinator(queue(),async()=>{preparing.resolve();await release.promise;},async()=>{snapshots.push('snapshot');});
 let aborted=false;const operation=coordinator.run('p',async signal=>{aborted=signal.aborted;});
 await preparing.promise;assert.equal(coordinator.cancel('p'),true);release.resolve();await operation;
 assert.equal(aborted,true);assert.equal(snapshots.length,2);
});
