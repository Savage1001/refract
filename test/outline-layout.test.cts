import test = require('node:test');
import assert = require('node:assert/strict');
import fixture = require('./fixtures/headless-ui.cjs');

for (const theme of ['light', 'dark'] as const) {
 test(`outline retains a bottom inset when collapsed and expanded (${theme})`, {timeout:60000}, async () => {
  const {browser,page,errors} = await fixture.openHeadlessUi(theme);
  try {
   await page.getByRole('button',{name:'Open Spectral methods for fluid dynamics',exact:true}).click();
   await page.locator('#source').waitFor({state:'visible'});
   for (const size of [{width:1440,height:900},{width:900,height:650}]) {
    await page.setViewportSize(size);
    for (const expanded of [false,true]) {
     const toggle = page.locator('#outline-toggle');
     if (await toggle.getAttribute('aria-expanded') !== String(expanded)) await toggle.click();
     const geometry = await page.locator('#outline').evaluate(el => {
      const sidebar = document.querySelector<HTMLElement>('.sidebar')!;
      const bounds = sidebar.getBoundingClientRect();
      const outline = el.getBoundingClientRect();
      const toggle = document.getElementById('outline-toggle')!.getBoundingClientRect();
      return {bottom:window.innerHeight-outline.bottom,left:outline.left-bounds.left,toggleBottom:toggle.bottom,outlineBottom:outline.bottom};
     });
     assert.equal(geometry.bottom,12);
     assert.equal(geometry.bottom,geometry.left);
     assert.ok(geometry.toggleBottom <= geometry.outlineBottom);
     assert.equal(await page.locator('#outline-list').isHidden(),!expanded);
    }
   }
   assert.deepEqual(errors,[]);
  } finally { await browser.close(); }
 });
}
