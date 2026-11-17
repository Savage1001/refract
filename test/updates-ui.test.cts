import test = require('node:test');
import assert = require('node:assert/strict');
import fs = require('node:fs/promises');
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

test('capture available and ready-to-apply update settings', {timeout:30000}, async()=>{
 const {browser,page,errors}=await fixture.openHeadlessUi('light',{pdf:'',source:'% Update status screenshot sample'});
 try {
  await page.evaluate(()=>{
   const w=window as unknown as {store:Record<string,unknown>};
   w.store.updateStatus=async()=>({track:'stable',phase:'downloading',message:'Downloading v0.3.1...'});
  });
  await page.getByRole('button',{name:'Settings',exact:true}).click();
  await page.getByText('Downloading v0.3.1...',{exact:true}).waitFor();
  await page.evaluate(()=>document.fonts.ready);
  await fs.mkdir('docs/updates',{recursive:true});
  await page.screenshot({path:'docs/updates/update-available.png'});
  await page.evaluate(()=>{
   const w=window as unknown as {store:Record<string,unknown>};
   w.store.updateStatus=async()=>({track:'stable',phase:'ready',message:'v0.3.1 is ready. It will install when you close Refract.'});
  });
  await page.getByText('v0.3.1 is ready. It will install when you close Refract.',{exact:true}).waitFor();
  await page.screenshot({path:'docs/updates/update-ready.png'});
  assert.equal(await page.getByRole('button',{name:'Check for updates',exact:true}).isVisible(),true);
  assert.deepEqual(errors,[]);
 } finally {await browser.close();}
});
