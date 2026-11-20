import test = require('node:test');
import assert = require('node:assert/strict');
import fixture = require('./fixtures/headless-ui.cjs');
import type { Page } from 'playwright';

async function openProject(page:Page):Promise<void> {
 await page.getByRole('button',{name:'Open Spectral methods for fluid dynamics',exact:true}).click();
 await page.locator('.pdf-pages canvas').waitFor({state:'attached'});
}
const names=['agent','editor','preview'] as const;
async function verify(page:Page,visible:boolean[]):Promise<void> {
 await page.waitForFunction(expected=>['#chat','#source','.pdf-pane'].every((selector,index)=>{
  const element=document.querySelector<HTMLElement>(selector)!;
  return Boolean(element.getClientRects().length)===expected[index];
 }),visible,{polling:50});
 for(const [index,selector] of ['#chat','#source','.pdf-pane'].entries()) {
  assert.equal(await page.locator(selector).isVisible(),visible[index],selector);
  assert.equal(await page.locator(`#pane-${names[index]}`).getAttribute('aria-pressed'),String(visible[index]));
 }
 assert.equal(await page.getByRole('group',{name:'Visible panes',exact:true}).isVisible(),true);
}

test('all seven pane combinations remain recoverable and preserve editor/chat drafts', {timeout:60000},async()=>{
 const {browser,page,errors}=await fixture.openHeadlessUi();
 try {
  await openProject(page);
  await page.setViewportSize({width:1000,height:740});await verify(page,[true,true,false]);
  await page.setViewportSize({width:1440,height:900});await verify(page,[true,true,true]);
  const tools=page.getByRole('button',{name:'Tools',exact:true});
  const paneGroup=page.getByRole('group',{name:'Visible panes',exact:true});
  const titlebar = page.locator('.app-titlebar');
  const projectTitle = titlebar.getByRole('button',{name:'Spectral methods for fluid dynamics',exact:true});
  const sidebar = page.locator('#project-navigation');
  const titleAligned = () => page.waitForFunction(() => {
   const title = document.querySelector('#project-name')!.getBoundingClientRect();
   const sidebar = document.querySelector('#project-navigation')!.getBoundingClientRect();
   return Math.abs(title.left - sidebar.right - 12) < 1;
  });
  await titleAligned();
  const sidebarWidth = (await sidebar.boundingBox())!.width;
  await page.locator('#collapse').click();
  assert.ok((await sidebar.boundingBox())!.width < sidebarWidth);
  await titleAligned();
  await page.locator('#collapse').click();
  await page.locator('#sidebar-resize').press('ArrowRight');
  assert.ok((await sidebar.boundingBox())!.width > sidebarWidth);
  await titleAligned();
  await projectTitle.click();
  await page.locator('#rename-dialog').waitFor();
  await page.keyboard.press('Escape');
  for (const width of [1440, 720]) {
   await page.setViewportSize({width,height:900});
   const bounds = await titlebar.boundingBox();
   assert.ok(bounds);
   for (const control of [projectTitle, ...await paneGroup.getByRole('button').all(), tools]) {
    const box = await control.boundingBox();
    assert.ok(box && box.y >= bounds.y && box.y + box.height <= bounds.y + bounds.height);
   }
  }
  await page.setViewportSize({width:1440,height:900});
  assert.equal(await paneGroup.getByRole('button').count(),3);
  assert.equal(await paneGroup.getByRole('button',{name:'Tools',exact:true}).count(),0);
  assert.equal(await tools.locator('svg').count(),1);
  assert.equal(await tools.getAttribute('aria-expanded'),'false');
  await tools.click();
  assert.equal(await tools.getAttribute('aria-expanded'),'true');
  await page.locator('#tools-panel').waitFor();
  await tools.click();
  assert.equal(await tools.getAttribute('aria-expanded'),'false');
  await page.locator('#pdf-content').waitFor();
  await tools.click();
  await page.getByRole('button',{name:'Expand tools panel',exact:true}).click();
  for(const name of names) assert.equal(await page.locator(`#pane-${name}`).getAttribute('aria-pressed'),'false');
  assert.equal(await page.locator('#source').isVisible(),false);
  await page.locator('#pane-editor').click();await verify(page,[true,true,true]);
  assert.equal(await tools.getAttribute('aria-expanded'),'false');
  await page.locator('#source').fill('% unsaved selection and draft');
  await page.locator('#source').evaluate(el=>(el as HTMLTextAreaElement).setSelectionRange(3,9));
  await page.locator('#chat-input').fill('Keep this question');
  for(const mask of [7,6,5,4,3,2,1]) {
   const target=names.map((_,index)=>Boolean(mask & (1<<index)));
   // Enable panes first to avoid triggering the last-pane fallback mid-transition.
   for(const on of [true,false]) for(const [index,name] of names.entries()) {
    const button=page.locator(`#pane-${name}`);
    if(target[index]===on && (await button.getAttribute('aria-pressed')==='true')!==on) await button.click();
   }
   await verify(page,target);
  }
  // Agent is the last pane. Keyboard activation restores the editor.
  await page.locator('#pane-agent').focus();await page.keyboard.press('Space');await verify(page,[false,true,false]);
  await page.locator('#pane-editor').click();await verify(page,[false,true,false]);
  assert.equal(await page.locator('#source').inputValue(),'% unsaved selection and draft');
  assert.deepEqual(await page.locator('#source').evaluate(el=>[(el as HTMLTextAreaElement).selectionStart,(el as HTMLTextAreaElement).selectionEnd]),[3,9]);
  await page.locator('#pane-agent').click();assert.equal(await page.locator('#chat-input').inputValue(),'Keep this question');
  assert.deepEqual(errors,[]);
 } finally {await browser.close();}
});

test('pane preferences survive reload and narrow preview switching; malformed preferences recover', {timeout:60000},async()=>{
 const {browser,page,errors}=await fixture.openHeadlessUi('dark');
 try {
  await openProject(page);await page.locator('#pane-agent').click();await page.locator('#pane-editor').click();await verify(page,[false,false,true]);
  await page.reload();await page.waitForFunction(()=>!document.body.inert);await openProject(page);await verify(page,[false,false,true]);
  await page.setViewportSize({width:1000,height:740});await verify(page,[false,false,true]);
  await page.locator('#pane-agent').click();await verify(page,[true,false,false]);
  await page.locator('#pane-editor').click();await verify(page,[true,true,false]);
  await page.locator('#pane-preview').click();await verify(page,[false,false,true]);
  await page.setViewportSize({width:1440,height:900});await verify(page,[true,true,true]);
  await page.setViewportSize({width:1000,height:740});await verify(page,[false,false,true]);
  await page.locator('#pane-preview').click();await verify(page,[true,true,false]);
  await page.setViewportSize({width:1440,height:900});await verify(page,[true,true,true]);
  for(const saved of ['broken','{"agent":false,"editor":false,"preview":false}','{"agent":"false","editor":false,"preview":false}']) {
   await page.evaluate(value=>localStorage.setItem('refract.workspace-panes.v1',value),saved);
   await page.reload();await page.waitForFunction(()=>!document.body.inert);await openProject(page);
   assert.equal(await page.locator('#source').isVisible(),true);
  }
  assert.deepEqual(errors,[]);
 } finally {await browser.close();}
});
