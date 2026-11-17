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
