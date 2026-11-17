import test = require('node:test');
import assert = require('node:assert/strict');
import fs = require('node:fs/promises');
import os = require('node:os');
import path = require('node:path');
import http = require('node:http');
import crypto = require('node:crypto');
import fixture = require('./electron-fixture.cjs');
const {launchTestDesktop} = fixture;

test('real NSIS updater rejects corrupt downloads, reuses verified cache and requests silent relaunch', {timeout:60000}, async t => {
 const dir = await fs.mkdtemp(path.join(os.tmpdir(), 'refract-updater-integration-'));
 const payload = Buffer.from('Inert installer fixture; never executed.');
 const hash = crypto.createHash('sha512').update(payload).digest('base64');
 let downloads = 0;
 const server = http.createServer((request, response) => {
  if (request.url?.split('?')[0].endsWith('.yml')) {
   response.end(`version: 99.0.0\nfiles:\n  - url: Refract-Setup-99.0.0.exe\n    sha512: ${hash}\n    size: ${payload.length}\npath: Refract-Setup-99.0.0.exe\nsha512: ${hash}\nreleaseDate: '2026-11-17T00:00:00.000Z'\n`);
  } else {
   downloads++;
   response.end(request.url?.startsWith('/bad/') ? Buffer.from('Corrupt installer') : payload);
  }
 });
 await new Promise<void>(resolve => server.listen(0, '127.0.0.1', resolve));
 t.after(async () => {
  await new Promise<void>((resolve, reject) => server.close(error => error ? reject(error) : resolve()));
  await fs.rm(dir, {recursive:true,force:true,maxRetries:5,retryDelay:200});
 });
 const address = server.address() as import('node:net').AddressInfo;
 const env:NodeJS.ProcessEnv = {...process.env, REFRACT_DATA_DIR:dir, REFRACT_TEST_HIDDEN:'1'};
 delete env.ELECTRON_RUN_AS_NODE;
 const app = await launchTestDesktop({args:[path.resolve(__dirname, '../..')], env});
 try {
  const result = await app.evaluate(async ({app}, {dir, base}) => {
   const require = (process.getBuiltinModule('module') as typeof import('node:module')).createRequire(app.getAppPath() + '/package.json');
   const {NsisUpdater}:typeof import('electron-updater') = require('electron-updater');
   const {ElectronHttpExecutor}:typeof import('electron-updater/out/electronHttpExecutor') = require('electron-updater/out/electronHttpExecutor');
   const fs:typeof import('node:fs/promises') = require('node:fs/promises');
   const {createUpdateClient} = require('./build/src/updates.cjs');
   const arch = Object.getOwnPropertyDescriptor(process, 'arch')!;
   try {
    Object.defineProperty(process, 'arch', {value:'arm64'});
    createUpdateClient({tag_name:'v99.0.0', draft:false, prerelease:false, assets:[{name:'latest.yml'}]});
   } finally {Object.defineProperty(process, 'arch', arch);}
   const config = dir + '/app-update.yml';
   await fs.writeFile(config, 'updaterCacheDirName: updater-test\n');
   const launches:{file:string;args:string[]}[] = [];
   let quitHandlers = 0;
   class TestUpdater extends NsisUpdater {
    protected override async spawnLog(file:string, args:string[] = []):Promise<boolean> {
     launches.push({file,args}); return true;
    }
   }
   const make = (feed:string) => {
    const updater = new TestUpdater({provider:'generic',url:base+feed+'/',channel:'latest'}, {
     version:'1.0.0', name:'RefractTest', isPackaged:true, appUpdateConfigPath:config,
     userDataPath:dir, baseCachePath:dir, whenReady:async()=>{}, relaunch:()=>{}, quit:()=>{},
     onQuit:()=>{quitHandlers++;}
    });
    Object.assign(updater, {httpExecutor:new ElectronHttpExecutor()});
    updater.setFeedURL({provider:'generic', url:base+feed+'/', channel:'latest'});
    updater.autoDownload = false; updater.autoInstallOnAppQuit = false;
    updater.disableDifferentialDownload = true; updater.logger = null;
    return updater;
   };
   const bad = make('bad'); await bad.checkForUpdates();
   let rejected = false;
   try {await bad.downloadUpdate();} catch (error) {rejected = /checksum/i.test(String(error));}
   const good = make('good'); await good.checkForUpdates();
   const files = await good.downloadUpdate();
   const cached = make('good'); await cached.checkForUpdates(); await cached.downloadUpdate();
   const beforeInstall = launches.length;
   const installed = cached.install(true, true);
   return {rejected, files:files.length, beforeInstall, installed, launches, quitHandlers};
  }, {dir, base:`http://127.0.0.1:${address.port}/`});
  assert.equal(result.rejected, true);
  assert.equal(result.files, 1);
  assert.equal(downloads, 2, 'verified cache must avoid a third download');
  assert.equal(result.beforeInstall, 0);
  assert.equal(result.quitHandlers, 0, 'ordinary quits must never install');
  assert.equal(result.installed, true);
  assert.equal(result.launches.length, 1);
  assert.deepEqual(result.launches[0].args, ['--updated','/S','--force-run']);
 } finally {
  await app.close();
 }
});
