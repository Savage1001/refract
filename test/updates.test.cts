import test = require('node:test');
import assert = require('node:assert/strict');
import eventsModule = require('node:events');
import fs = require('node:fs/promises');
import os = require('node:os');
import path = require('node:path');
const { UpdateService, selectRelease, CHECK_INTERVAL_MS } = require('../src/updates.cjs');
import type { UpdateClient } from '../src/updates.cjs';
const release = (tag_name: string, prerelease = false, draft = false) => ({tag_name, prerelease, draft, assets:[{name:prerelease ? 'beta.yml' : 'latest.yml'}]});
test('tracks select newest eligible semver, exclude drafts, alpha and downgrades', () => {
 const releases = [release('v1.1.0'), release('v1.2.0-beta.2',true), release('v1.2.0-beta.10',true), release('v2.0.0-alpha.1',true), release('v3.0.0',false,true), release('bad')];
 assert.equal(selectRelease(releases,'stable','1.0.0')?.tag_name,'v1.1.0');
 assert.equal(selectRelease(releases,'beta','1.0.0')?.tag_name,'v1.2.0-beta.10');
 assert.equal(selectRelease([...releases,release('v1.2.0')],'beta','1.2.0-beta.10')?.tag_name,'v1.2.0');
 assert.equal(selectRelease(releases,'stable','1.2.0-beta.10'),undefined);
 assert.equal(CHECK_INTERVAL_MS,600000);
});
test('startup download, persisted track, disabled development and retry after failure', async () => {
 const dir = await fs.mkdtemp(path.join(os.tmpdir(),'refract-update-'));
 const file = path.join(dir,'settings.json');
 const clients: UpdateClient[] = []; let fail = true;
 const service = new UpdateService(file,'1.0.0',true,async()=>{if(fail)throw new Error('offline');return [release('v1.1.0'),release('v1.2.0-beta.1',true)];}, r => {
  const client: UpdateClient = {autoDownload:true,autoInstallOnAppQuit:false,allowPrerelease:false,allowDowngrade:true,on:()=>{},checkForUpdates:async()=>({updateInfo:{version:r.tag_name.slice(1)}}),downloadUpdate:async()=>{},quitAndInstall:()=>{}};
  clients.push(client);return client;
 });
 try {
  await service.start(); await service.check(); assert.equal(service.status().phase,'error');
  fail=false; await service.check();assert.equal(service.status().phase,'ready');assert.equal(clients[0].autoInstallOnAppQuit,false);
  await service.setTrack('beta');assert.equal(clients[0].autoInstallOnAppQuit,false);assert.equal(clients[1].allowPrerelease,true);assert.equal(clients[1].allowDowngrade,false);assert.equal(clients[1].autoInstallOnAppQuit,false);
  await assert.rejects(service.setTrack('nightly'));
  const restarted=new UpdateService(file,'1.0.0',false);await restarted.start();assert.equal(restarted.status().track,'beta');assert.equal(restarted.status().phase,'disabled');restarted.stop();
 } finally {service.stop();await fs.rm(dir,{recursive:true,force:true});}
});
test('switching track during a download prevents arming the previous track', async () => {
 const dir=await fs.mkdtemp(path.join(os.tmpdir(),'refract-update-race-'));
 let finish!:()=>void;let started!:()=>void;
 const downloading=new Promise<void>(r=>started=r);const download=new Promise<void>(r=>finish=r);
 let armed=false;
 const progress = new eventsModule.EventEmitter<{'download-progress': [{percent: number; total: number}]; error: [Error]}>();
 await fs.writeFile(path.join(dir,'settings.json'),JSON.stringify({track:'beta'}));
 const service=new UpdateService(path.join(dir,'settings.json'),'1.0.0',true,async()=>[release('v1.1.0-beta.1',true)],()=>({autoDownload:false,autoInstallOnAppQuit:false,allowPrerelease:true,allowDowngrade:false,on:progress.on.bind(progress),checkForUpdates:async()=>({updateInfo:{version:'1.1.0-beta.1'}}),downloadUpdate:async()=>{started();await download;},quitAndInstall:()=>{armed=true;}}));
 try {
  await service.start();await downloading;
  assert.equal(service.status().downloadPercent,undefined);
  progress.emit('download-progress',{percent:64.8,total:615 * 1024});
  assert.equal(service.status().downloadPercent,64);
  assert.match(service.status().message,/615\.0 KiB/);
  progress.emit('download-progress',{percent:25,total:131 * 1024 ** 2});
  assert.equal(service.status().downloadPercent,25);
  assert.match(service.status().message,/131\.0 MiB/);
  progress.emit('download-progress',{percent:NaN,total:0});
  assert.equal(service.status().downloadPercent,25);
  const stable=service.setTrack('stable');
  // Wait until the new preference has been published while the old download is in flight.
  while(service.status().track!=='stable') await new Promise<void>(r=>setImmediate(r));
  progress.emit('download-progress',{percent:90,total:131 * 1024 ** 2});
  assert.equal(service.status().downloadPercent,undefined);
  finish();await stable;
  assert.equal(armed,false);assert.equal(service.status().phase,'idle');assert.throws(()=>service.prepareRestart());
 } finally {finish();service.stop();await fs.rm(dir,{recursive:true,force:true});}
});
test('failed download and mismatched metadata never arm installation', async () => {
 const dir=await fs.mkdtemp(path.join(os.tmpdir(),'refract-update-failure-'));
 let mismatch=true,downloads=0,armed=false;
 const service=new UpdateService(path.join(dir,'settings.json'),'1.0.0',true,async()=>[release('v1.1.0')],()=>({autoDownload:false,autoInstallOnAppQuit:false,allowPrerelease:false,allowDowngrade:false,on:()=>{},checkForUpdates:async()=>({updateInfo:{version:mismatch?'2.0.0':'1.1.0'}}),downloadUpdate:async()=>{downloads++;throw new Error('checksum mismatch');},quitAndInstall:()=>{armed=true;}}));
 try {
  await service.start();await service.check();assert.equal(downloads,0);assert.equal(service.status().phase,'error');
  mismatch=false;await service.check();assert.equal(downloads,1);assert.equal(armed,false);assert.match(service.status().message,/checksum mismatch/);
 } finally {service.stop();await fs.rm(dir,{recursive:true,force:true});}
});

test('downloaded updates never install on ordinary quit and require explicit restart', async () => {
 const dir=await fs.mkdtemp(path.join(os.tmpdir(),'refract-update-explicit-'));
 let installs=0;
 const clients:UpdateClient[]=[];
 const makeService=()=>new UpdateService(path.join(dir,'settings.json'),'1.0.0',true,async()=>[release('v1.1.0')],()=>{
  const client:UpdateClient={autoDownload:true,autoInstallOnAppQuit:true,allowPrerelease:false,allowDowngrade:false,on:()=>{},checkForUpdates:async()=>({updateInfo:{version:'1.1.0'}}),downloadUpdate:async()=>{},quitAndInstall:(silent,relaunch)=>{assert.equal(silent,false);assert.equal(relaunch,true);installs++;}};
  clients.push(client);return client;
 });
 const closing=makeService();const explicit=makeService();
 try {
  assert.throws(()=>closing.prepareRestart());
  await closing.start();await closing.check();
  assert.equal(closing.status().phase,'ready');assert.equal(clients[0].autoInstallOnAppQuit,false);
  closing.stop();assert.equal(installs,0);
  await explicit.start();await explicit.check();
  const afterShutdown=explicit.prepareRestart();
  assert.equal(installs,0,'requesting restart must wait for the shutdown barrier');
  assert.equal(clients[1].autoInstallOnAppQuit,false);
  assert.equal(explicit.status().phase,'restarting');
  assert.throws(()=>explicit.prepareRestart());
  await assert.rejects(explicit.setTrack('beta'));
  await explicit.check();assert.equal(clients.length,2);
  afterShutdown();afterShutdown();assert.equal(installs,1);
 } finally {closing.stop();explicit.stop();await fs.rm(dir,{recursive:true,force:true});}
});

test('pending track changes block restart before preference I/O completes', async () => {
 const dir = await fs.mkdtemp(path.join(os.tmpdir(), 'refract-update-pending-'));
 const service = new UpdateService(path.join(dir, 'settings.json'), '1.0.0', true,
  async () => [release('v1.1.0')], () => ({autoDownload:false, autoInstallOnAppQuit:false,
   allowPrerelease:false, allowDowngrade:false, on:()=>{},
   checkForUpdates:async()=>({updateInfo:{version:'1.1.0'}}), downloadUpdate:async()=>{}, quitAndInstall:()=>{}}));
 try {
  await service.start(); await service.check();
  const pending = service.setTrack('beta');
  assert.throws(() => service.prepareRestart());
  await pending;
  assert.equal(service.status().track, 'beta');
  assert.doesNotThrow(() => service.prepareRestart());
 } finally {service.stop(); await fs.rm(dir, {recursive:true, force:true});}
});

test('beta track accepts numbered beta and pre-release versions', () => {
 assert.equal(selectRelease([release('v2.0.0-pre-release-1', true), release('v2.0.0-pre-release-2', true)], 'beta', '1.0.0')?.tag_name, 'v2.0.0-pre-release-2');
 assert.equal(selectRelease([release('v2.0.0-pre-release-1', true)], 'stable', '1.0.0'), undefined);
 for (const tag of ['v2.0.0-pre-release-0', 'v2.0.0-pre-release-foo', 'v2.0.0-pre-release-1.extra']) {
  assert.equal(selectRelease([release(tag, true)], 'beta', '1.0.0'), undefined);
 }
 for (const tag of ['v2.0.0-beta', 'v2.0.0-beta.foo', 'v2.0.0-beta.1.extra']) {
  assert.equal(selectRelease([release(tag, true)], 'beta', '1.0.0'), undefined);
 }
});

test('installer handoff errors are persisted and allow downloading a retry', async () => {
 const dir=await fs.mkdtemp(path.join(os.tmpdir(),'refract-update-handoff-'));
 const events = new eventsModule.EventEmitter<{'download-progress': [{percent: number; total: number}]; error: [Error]}>();
 const service=new UpdateService(path.join(dir,'settings.json'),'1.0.0',true,async()=>[release('v1.1.0')],()=>({
  autoDownload:false,autoInstallOnAppQuit:false,allowPrerelease:false,allowDowngrade:false,
  on:events.on.bind(events), checkForUpdates:async()=>({updateInfo:{version:'1.1.0'}}),
  downloadUpdate:async()=>{},quitAndInstall:()=>{events.emit('error',new Error('installer denied'));}
 } satisfies UpdateClient));
 let recovered=false;service.onRestartFailed=()=>{recovered=true;};
 try {
  await service.start();await service.check();service.prepareRestart()();
  assert.equal(recovered,true);assert.equal(service.status().phase,'error');
  assert.match(service.status().message,/installer denied/);
  assert.match(await fs.readFile(path.join(dir,'logs','updates.jsonl'),'utf8'),/restart-failed/);
  await service.check();assert.equal(service.status().phase,'ready');
  assert.match(service.status().message,/Installed: 1.0.0/);
 } finally {service.stop();await fs.rm(dir,{recursive:true,force:true});}
});
