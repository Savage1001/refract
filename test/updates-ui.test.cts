import test = require('node:test');
import assert = require('node:assert/strict');
import fs = require('node:fs/promises');
import type { UpdateStatus } from '../src/updates.cjs';
import fixture = require('./fixtures/headless-ui.cjs');
test('settings gear appears only at home and offers persisted update tracks', {timeout:60000}, async()=>{
 const {browser,page,errors}=await fixture.openHeadlessUi('light',{pdf:'',source:'% Settings UI sample'});
 try {
  await page.getByRole('button',{name:'Settings',exact:true}).click();
  await page.getByText('You are up to date. Checking every 10 minutes.',{exact:true}).waitFor();
  await page.getByLabel('Update track',{exact:true}).selectOption('beta');
  await page.waitForFunction(()=>!(document.querySelector('#update-track') as HTMLSelectElement).disabled);
  await page.getByRole('button',{name:'Close',exact:true}).click();
  await page.getByRole('button',{name:'Settings',exact:true}).click();
  assert.equal(await page.getByLabel('Update track',{exact:true}).inputValue(),'beta');
  await page.evaluate(()=>{(window as unknown as {fixture:{delay:number}}).fixture.delay=300;});
  const check = page.getByRole('button',{name:'Check for updates',exact:true});
  await check.click();
  assert.equal(await check.isDisabled(),true);
  assert.equal(await page.getByLabel('Update track',{exact:true}).isDisabled(),true);
  await page.waitForFunction(()=>!(document.querySelector('#check-updates') as HTMLButtonElement).disabled);
  assert.equal(await page.evaluate(()=>(window as unknown as {fixture:{counters:Record<string,number>}}).fixture.counters.updateCheck),1);
  await page.getByText('You are up to date. Checking every 10 minutes.',{exact:true}).waitFor();
  await page.evaluate(()=>document.fonts.ready);
  await fs.mkdir('docs/updates',{recursive:true});
  await page.screenshot({path:'docs/updates/settings-menu.png'});
  await page.keyboard.press('Escape');
  await page.screenshot({path:'docs/updates/home-settings-icon.png'});
  await page.getByRole('button',{name:'Open Spectral methods for fluid dynamics',exact:true}).click();
  await page.locator('#source').waitFor();
  assert.equal(await page.getByRole('button',{name:'Settings',exact:true}).isVisible(),false);
  assert.deepEqual(errors,[]);
 } finally { await browser.close(); }
});

test('show download progress and expose restart when the update is ready', {timeout:30000}, async()=>{
 const {browser,page,errors}=await fixture.openHeadlessUi('light',{pdf:'',source:'% Update status screenshot sample'});
 try {
  const downloading: UpdateStatus = {
   track:'beta',phase:'downloading',message:'Installed: 0.3.1-pre-release-5. Downloading v0.3.1-pre-release-6...'
  };
  const setStatus = (status: UpdateStatus) => page.evaluate(value=>{
   const w=window as unknown as {store:Record<string,unknown>};
   w.store.updateStatus=async()=>value;
  },status);
  await setStatus(downloading);
  await page.getByRole('button',{name:'Settings',exact:true}).click();
  const progress = page.getByRole('progressbar',{name:'Update download progress'});
  await progress.waitFor();
  assert.equal(await progress.getAttribute('value'),null);
  await setStatus({...downloading,downloadPercent:64});
  await page.getByText('64%',{exact:true}).waitFor();
  assert.equal(await progress.getAttribute('value'),'64');
  await page.evaluate(()=>document.fonts.ready);
  await fs.mkdir('docs/updates',{recursive:true});
  assert.equal(await page.getByRole('button',{name:'Restart to update',exact:true}).isVisible(),false);
  await page.emulateMedia({colorScheme:'dark'});
  await page.getByRole('dialog',{name:'Settings'}).screenshot({path:'docs/updates/update-available.png'});
  await page.emulateMedia({colorScheme:'light'});
  await page.evaluate(()=>{
   const w=window as unknown as {store:Record<string,unknown>};
   w.store.updateStatus=async()=>({track:'stable',phase:'ready',message:'v0.3.1 is ready. Click Restart to update to install it.'});
  });
  await page.getByText('v0.3.1 is ready. Click Restart to update to install it.',{exact:true}).waitFor();
  assert.equal(await progress.isVisible(),false);
  const restart = page.getByRole('button',{name:'Restart to update',exact:true});
  assert.equal(await restart.isEnabled(),true);
  await page.screenshot({path:'docs/updates/update-ready.png'});
  await restart.click();
  assert.equal(await restart.isDisabled(),true);
  await page.waitForFunction(()=>(window as unknown as {fixture:{counters:Record<string,number>}}).fixture.counters.updateRestart===1);
  await page.getByText('Saving your work and restarting to update...',{exact:true}).waitFor();
  assert.equal(await page.getByRole('button',{name:'Check for updates',exact:true}).isVisible(),true);
  assert.deepEqual(errors,[]);
 } finally {await browser.close();}
});
