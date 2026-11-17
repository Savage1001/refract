import fs = require('node:fs/promises');
import path = require('node:path');
import assert = require('node:assert/strict');
import fixture = require('../test/fixtures/headless-ui.cjs');

async function main():Promise<void> {
 const output=path.resolve('docs/pane-layout');await fs.mkdir(output,{recursive:true});
 for(const theme of ['light','dark'] as const) {
  const {browser,page,errors}=await fixture.openHeadlessUi(theme);
  try {
   await page.getByRole('button',{name:'Open Spectral methods for fluid dynamics',exact:true}).click();
   await page.getByText('Compiled',{exact:true}).waitFor();await page.locator('.pdf-pages canvas').waitFor();await page.evaluate(()=>document.fonts.ready);
   await page.mouse.move(600,50);await page.screenshot({path:path.join(output,`workspace-${theme}.png`)});
   await page.locator('#pane-agent').click();await page.mouse.move(600,50);await page.screenshot({path:path.join(output,`editor-preview-${theme}.png`)});
   await page.setViewportSize({width:1000,height:740});await page.locator('#pane-preview').click();
   await page.mouse.move(600,50);await page.screenshot({path:path.join(output,`narrow-preview-${theme}.png`)});
   assert.deepEqual(errors,[]);
  } finally {await browser.close();}
 }
}
void main().catch(error=>{console.error(error);process.exitCode=1;});
