import fs = require('node:fs/promises');
import path = require('node:path');
import assert = require('node:assert/strict');
import fixture = require('../test/fixtures/headless-ui.cjs');
import type { Page } from 'playwright';

const output = path.resolve(process.env.REFRACT_THEME_ARTIFACTS || 'docs/ui');
async function settledPdf(page:Page):Promise<void> {
 await page.getByText('Compiled',{exact:true}).waitFor();
 await page.waitForFunction(()=>Number(document.querySelector<HTMLCanvasElement>('.pdf-pages canvas')?.dataset.renderScale)>0);
 await page.evaluate(()=>document.fonts.ready);
}

async function main():Promise<void> {
 await fs.mkdir(output,{recursive:true});
 const captures:Record<string,{width:number;height:number;sidebar:number;chat:number;selection:number[];comments:string[]}> = {};
 for (const theme of ['light','dark'] as const) {
  // Fresh application state per theme: no carried pane sizes, comments or selection.
  const {browser,page,errors}=await fixture.openHeadlessUi(theme);
  const capture = async (name:string):Promise<void> => {
   await page.evaluate(()=>document.fonts.ready);
   const state = await page.evaluate(()=>{
    const editor=document.querySelector<HTMLTextAreaElement>('#source')!;
    return {width:innerWidth,height:innerHeight,sidebar:document.querySelector('.sidebar')!.clientWidth,chat:document.getElementById('chat')!.clientHeight,selection:[editor.selectionStart,editor.selectionEnd],comments:Array.from(document.querySelectorAll('.comment-card>p')).map(el=>el.textContent ?? '')};
   });
   if(theme==='dark')assert.deepEqual(state,captures[`${name}-light`],`Equivalent state required for ${name}`);
   captures[`${name}-${theme}`]=state;
   await page.screenshot({path:path.join(output,`${name}-${theme}.png`)});
  };
  try {
   await capture('populated');await page.locator('.sidebar').evaluate(el=>(el as HTMLElement).style.setProperty('--sidebar-width','150px'));await page.screenshot({path:path.join(output,`branding-narrow-${theme}.png`)});await page.locator('.sidebar').evaluate(el=>(el as HTMLElement).style.removeProperty('--sidebar-width'));await page.locator('#grid-view').click();await capture('grid');await page.locator('#list-view').click();
   await page.getByRole('button',{name:'Open Spectral methods for fluid dynamics',exact:true}).click();await settledPdf(page);
   await page.locator('#file-list button[title="sections/abstract.tex"]').click();await page.getByRole('button',{name:'main.tex',exact:true}).click();
   await capture('workspace');
   await page.locator('.layout-menu>summary').click();await page.screenshot({path:path.join(output,`layout-${theme}.png`)});await page.keyboard.press('Escape');
   await page.locator('#project-name').click();await capture('dialog');await page.locator('#rename-dialog button[value=cancel]').click();
   await page.locator('#tools').click();await page.locator('#tool-info').click();await page.locator('.project-facts').waitFor();await capture('tools');
   await page.locator('#tool-comments').click();await page.waitForFunction(()=>document.querySelectorAll('.comment-card').length===2);await capture('comments');
   await page.locator('#tool-versions').click();await page.locator('.version-row').waitFor();
   await page.locator('.version-comparison>summary').click();await page.getByText('changed: main.tex',{exact:true}).click();await page.locator('.comparison-wrap').waitFor();await capture('versions');
   await page.locator('#tools').click();await page.locator('.model-trigger').click();
   await page.locator('[data-focus-key="model:model-a"]').click();await page.locator('.model-trigger').click();await capture('models');await page.keyboard.press('Escape');
   await page.setViewportSize({width:1000,height:740});await capture('compact');
   await page.locator('.layout-menu>summary').click();await page.getByLabel('Workspace view',{exact:true}).selectOption('pdf');await page.locator('.layout-menu>summary').click();await settledPdf(page);await capture('compact-preview');
   await page.getByRole('button',{name:'Return to editor',exact:true}).click();await page.setViewportSize({width:900,height:650});await page.locator('#source').focus();await capture('short');
   await page.setViewportSize({width:1440,height:900});await page.locator('.layout-menu>summary').click();await page.getByLabel('Workspace view',{exact:true}).selectOption('split');await page.keyboard.press('Escape');await page.locator('#home').click();
   await page.getByRole('button',{name:'Open Conference paper - autumn notes',exact:true}).click();await page.locator('.message.assistant').waitFor();await settledPdf(page);
   await capture('conversation');await page.getByRole('button',{name:'Expand chat',exact:true}).click();await capture('expanded-chat');
   assert.deepEqual(errors,[]);
  } finally {await browser.close();}
 }
 assert.equal(Object.keys(captures).length,26);
 await fs.writeFile(path.join(output,'capture-manifest.json'),JSON.stringify({renderer:'Headless Chromium with isolated sample IPC data',pairedStateVerified:true,captures},null,2)+'\n');
 console.log('Captured 26 screenshots; all 13 theme pairs have identical dimensions, pane sizes, selection and comments. No native windows were created.');
}
void main().catch(error=>{console.error(error);process.exitCode=1;});
