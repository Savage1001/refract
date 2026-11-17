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

test('outline renders inline math and retains section navigation', {timeout:30000}, async () => {
 const source = [
  '\\section{Introduction}',
  '\\subsection*{$J_{2}$ Truth, \\(\\Delta t=1800\\) s}',
  '\\section{Cost \\$5 <example>}'
 ].join('\n');
 const {browser,page,errors} = await fixture.openHeadlessUi('light', {source, pdf:''});
 try {
  await page.getByRole('button',{name:'Open Spectral methods for fluid dynamics',exact:true}).click();
  const rows = page.locator('#outline-list button');
  const mathRow = rows.nth(1);
  await mathRow.locator('math').nth(1).waitFor();
  assert.equal(await mathRow.locator('msub').textContent(),'J2');
  assert.equal(await mathRow.locator('math').nth(1).textContent(),'Δt=1800');
  assert.match(await mathRow.textContent() ?? '', / Truth, .* s$/);
  assert.equal(await rows.nth(0).textContent(),'Introduction');
  assert.equal(await rows.nth(2).textContent(),'Cost \\$5 <example>');
  await mathRow.click();
  assert.equal(await page.locator('#source').evaluate((editor:HTMLTextAreaElement) => editor.selectionStart),source.indexOf('\\subsection'));
  assert.deepEqual(errors,[]);
 } finally { await browser.close(); }
});
