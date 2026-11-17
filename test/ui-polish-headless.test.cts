import test = require('node:test');
import assert = require('node:assert/strict');
import fixture = require('./fixtures/headless-ui.cjs');
import type { Page } from 'playwright';

interface StateWindow extends Window { fixture:{counters:Record<string,number>;delay:number} }
async function openProject(page:Page):Promise<void> {
 await page.getByRole('button',{name:'Open Spectral methods for fluid dynamics',exact:true}).click();
 await page.locator('#source').waitFor({state:'visible'});
 await page.getByText('Compiled',{exact:true}).waitFor();
 await page.waitForFunction(()=>Number(document.querySelector<HTMLCanvasElement>('.pdf-pages canvas')?.dataset.renderScale)>0);
}

test('headless editor escape, completion, context menus and dialog primary actions', {timeout:60000}, async()=>{
 const {browser,page,errors}=await fixture.openHeadlessUi();
 try {
  page.setDefaultTimeout(7000);
  await openProject(page);
  const source=page.locator('#source'); await source.fill('abc'); await source.press('End'); await source.press('Tab');
  assert.equal(await source.inputValue(),'abc  '); await source.press('Control+z'); assert.equal(await source.inputValue(),'abc');
  for(const key of ['Tab','Shift+Tab']) {await source.focus();await source.press('Escape');await page.keyboard.press(key);assert.equal(await source.evaluate(el=>el===document.activeElement),false);}
  await source.fill('\\sec');await source.press('Control+Space');await source.press('Tab');assert.match(await source.inputValue(),/\\section/);
  const file=page.locator('#file-list button[title="main.tex"]');await file.focus();await file.press('Shift+F10');
  assert.equal(await page.locator('#file-rename').evaluate(el=>el===document.activeElement),true);
  await page.keyboard.press('Escape');assert.equal(await file.evaluate(el=>el===document.activeElement),true);
  await page.locator('#add-file').click();await page.locator('#file-name').press('Enter');
  assert.equal(await page.locator('#file-name').getAttribute('aria-invalid'),'true');
  await page.locator('#file-name').fill('notes.tex');await page.locator('#file-name').press('Enter');
  await page.locator('#file-list button[title="notes.tex"]').waitFor();
  assert.equal(await page.evaluate(()=>(window as unknown as StateWindow).fixture.counters.createFolder ?? 0),0);
  const added=page.locator('#file-list button[title="notes.tex"]');await added.focus();await added.press('Shift+F10');await page.locator('#file-rename').click();
  await page.locator('#rename-file-name').fill('revised.tex');await page.locator('#rename-file-name').press('Enter');await page.locator('#file-list button[title="revised.tex"]').waitFor();
  const renameCount=await page.evaluate(()=>(window as unknown as StateWindow).fixture.counters.renameFile);
  await page.locator('#file-list button[title="revised.tex"]').focus();await page.keyboard.press('Shift+F10');await page.locator('#file-rename').click();
  await page.locator('#rename-file-name').fill('cancelled-rename.tex');await page.keyboard.press('Escape');await page.locator('#rename-file-dialog').waitFor({state:'hidden'});
  assert.equal(await page.evaluate(()=>(window as unknown as StateWindow).fixture.counters.renameFile),renameCount);assert.equal(await page.locator('#file-list button[title="revised.tex"]').isVisible(),true);
  await page.locator('#project-name').click();await page.locator('#rename-name').fill('Confirmed project');await page.locator('#rename-name').press('Enter');
  await page.waitForFunction(()=>document.getElementById('project-name')?.textContent==='Confirmed project');
  const projectRenameCount=await page.evaluate(()=>(window as unknown as StateWindow).fixture.counters.rename);
  await page.locator('#project-name').click();await page.locator('#rename-name').fill('Cancelled project');await page.keyboard.press('Escape');await page.locator('#rename-dialog').waitFor({state:'hidden'});
  assert.equal(await page.evaluate(()=>(window as unknown as StateWindow).fixture.counters.rename),projectRenameCount);assert.equal(await page.locator('#project-name').textContent(),'Confirmed project');
  const folder=page.locator('#file-list summary[title="sections"]');await folder.focus();await folder.press('Shift+F10');await page.locator('#folder-move').click();
  await page.locator('#folder-move-path').fill('chapters');await page.locator('#folder-move-path').press('Enter');await page.locator('#file-list summary[title="chapters"]').waitFor();
  const before=await page.evaluate(()=>(window as unknown as StateWindow).fixture.counters.createFile);
  for(const cancel of ['button','escape']) {await page.locator('#add-file').click();await page.locator('#file-name').fill('cancelled.tex');if(cancel==='escape')await page.keyboard.press('Escape');else await page.locator('#file-dialog button[value=cancel]').click();}
  assert.equal(await page.evaluate(()=>(window as unknown as StateWindow).fixture.counters.createFile),before);
  assert.deepEqual(errors,[]);
 } catch(error) {console.error(errors);throw error;} finally {await browser.close();}
});

test('headless composition confirmation cannot send or select, and ordinary Enter works', {timeout:60000}, async()=>{
 const {browser,page,errors}=await fixture.openHeadlessUi();
 try {
  await openProject(page); const input=page.locator('#chat-input');await input.fill('Research question');
  await input.dispatchEvent('keydown',{key:'Enter',isComposing:true});await input.dispatchEvent('keydown',{key:'Enter',keyCode:229});
  assert.equal(await page.evaluate(()=>(window as unknown as StateWindow).fixture.counters.chatSend ?? 0),0);
  await page.locator('.model-trigger').click();const search=page.getByLabel('Search models');await search.fill('Research');
  await search.dispatchEvent('keydown',{key:'Enter',isComposing:true});assert.equal(await page.locator('.model-panel').isVisible(),true);
  assert.equal(await page.locator('.model-trigger').textContent(),'CLI default');
  await search.press('Enter');assert.equal(await page.locator('.model-panel').isVisible(),false);
  await input.focus();await input.press('End');await input.press('Shift+Enter');assert.match(await input.inputValue(),/\n$/);
  await input.press('Enter');await page.waitForFunction(()=>(window as unknown as StateWindow).fixture.counters.chatSend===1);
  assert.deepEqual(errors,[]);
 } finally {await browser.close();}
});

test('headless comment actions preserve unrelated drafts and reject duplicate submissions', {timeout:60000}, async()=>{
 const {browser,page,errors}=await fixture.openHeadlessUi();
 try {
  await openProject(page);await page.locator('#tools').click();await page.locator('#tool-comments').click();
  const draft=page.getByRole('textbox',{name:'Add a comment',exact:true});await draft.fill('Unfinished main comment');
  let cards=page.locator('.comment-card');await cards.first().locator('.comment-reply-form summary').click();await cards.first().getByLabel('Write a reply').fill('Unfinished first reply');
  await cards.nth(1).getByRole('button',{name:'Resolve',exact:true}).click();await page.getByRole('button',{name:'Reopen',exact:true}).waitFor();
  await page.getByRole('button',{name:'Reopen',exact:true}).click();await page.waitForFunction(()=>!document.querySelector('.comment-status.resolved'));
  assert.equal(await draft.inputValue(),'Unfinished main comment');assert.equal(await cards.first().getByLabel('Write a reply').inputValue(),'Unfinished first reply');
  await page.getByLabel('Search comments').fill('Clarify');await page.getByLabel('Search comments').fill('');
  assert.equal(await cards.first().getByLabel('Write a reply').inputValue(),'Unfinished first reply');
  await cards.nth(1).locator('.comment-reply-form summary').click();await cards.nth(1).getByLabel('Write a reply').fill('Keep second reply');
  await page.evaluate(()=>{(window as unknown as StateWindow).fixture.delay=150;});
  await cards.first().getByRole('button',{name:'Reply',exact:true}).evaluate((button:HTMLButtonElement)=>{button.click();button.click();});
  await page.waitForFunction(()=>document.querySelector('.comment-replies')?.textContent?.includes('Unfinished first reply'));
  assert.equal(await cards.nth(1).getByLabel('Write a reply').inputValue(),'Keep second reply');assert.equal(await draft.inputValue(),'Unfinished main comment');
  assert.equal(await cards.first().getByLabel('Write a reply').inputValue(),'');
  assert.equal(await page.evaluate(()=>(window as unknown as StateWindow).fixture.counters.commentReply),1);
  await page.getByRole('button',{name:'Comment',exact:true}).evaluate((button:HTMLButtonElement)=>{button.click();button.click();});
  await draft.fill('New text written during submission');
  await page.waitForFunction(()=>document.querySelectorAll('.comment-card').length===3);
  assert.equal(await draft.inputValue(),'New text written during submission');assert.equal(await page.evaluate(()=>(window as unknown as StateWindow).fixture.counters.commentAdd),1);
  await page.getByRole('button',{name:'Comment',exact:true}).click();await page.waitForFunction(()=>document.querySelectorAll('.comment-card').length===4);
  assert.equal(await draft.inputValue(),'');assert.equal(await cards.nth(1).getByLabel('Write a reply').inputValue(),'Keep second reply');
  assert.deepEqual(errors,[]);
 } finally {await browser.close();}
});

test('headless selected states, picker dismissal, and default-choice styling agree', {timeout:60000}, async()=>{
 const {browser,page,errors}=await fixture.openHeadlessUi();
 try {
  assert.equal(await page.locator('#list-view').getAttribute('aria-pressed'),'true');await page.locator('#grid-view').click();
  assert.equal(await page.locator('#grid-view').getAttribute('aria-pressed'),'true');assert.equal(await page.locator('#list-view').getAttribute('aria-pressed'),'false');
  await openProject(page);await page.locator('#chats-tab').click();assert.equal(await page.locator('#chats-tab').getAttribute('aria-pressed'),'true');
  await page.locator('#files-tab').click();assert.equal(await page.locator('#chats-tab').getAttribute('aria-pressed'),'false');
  assert.equal(await page.locator('#file-list button[title="main.tex"]').getAttribute('aria-current'),'page');
  await page.locator('#file-list button[title="sections/abstract.tex"]').click();assert.equal(await page.locator('#file-list button[title="main.tex"]').getAttribute('aria-current'),null);
  assert.equal(await page.locator('#file-list button[title="sections/abstract.tex"]').getAttribute('aria-current'),'page');await page.getByRole('button',{name:'main.tex',exact:true}).click();
  await page.locator('#tools').click();await page.locator('#tool-versions').click();assert.equal(await page.locator('#tool-versions').getAttribute('aria-pressed'),'true');assert.equal(await page.locator('#tool-info').getAttribute('aria-pressed'),'false');await page.locator('#tools').click();
  for(const mode of ['light','dark'] as const) {
   await page.emulateMedia({colorScheme:mode});await page.locator('.model-trigger').click();
   const choice=page.locator('[data-focus-key="model:model-a"]'), fallback=page.locator('[data-focus-key="model:default"]');
   assert.equal(await fallback.evaluate(el=>getComputedStyle(el).fontSize),await choice.evaluate(el=>getComputedStyle(el).fontSize));
   assert.equal(await fallback.evaluate(el=>getComputedStyle(el).padding),await choice.evaluate(el=>getComputedStyle(el).padding));
   await page.locator('[data-focus-key="favorite:model-a"]').click();assert.equal(await page.locator('[data-focus-key="favorite:model-a"]').evaluate(el=>el===document.activeElement),true);
   await choice.click();await page.locator('.model-trigger').click();await page.getByRole('button',{name:'High',exact:true}).click();assert.equal(await page.getByRole('button',{name:'High',exact:true}).evaluate(el=>el===document.activeElement),true);
   await page.getByRole('button',{name:'Fast',exact:true}).focus();await page.keyboard.press('Tab');assert.equal(await page.locator('.model-panel').isVisible(),false);assert.equal(await page.locator('.model-trigger').getAttribute('aria-expanded'),'false');
   await page.locator('.model-trigger').click();await page.keyboard.press('Escape');assert.equal(await page.locator('.model-trigger').evaluate(el=>el===document.activeElement),true);
   await page.locator('.model-trigger').click();await page.locator('#source').click({position:{x:10,y:10}});assert.equal(await page.locator('.model-panel').isVisible(),false);assert.equal(await page.locator('#source').evaluate(el=>el===document.activeElement),true);
   await page.locator('.model-trigger').click();await page.locator('#pane-editor').click();await page.locator('#pane-agent').click();await page.keyboard.press('Escape');assert.equal(await page.locator('.model-trigger').getAttribute('aria-expanded'),'false');await page.locator('#pane-editor').click();await page.locator('#pane-agent').click();
  }
  assert.deepEqual(errors,[]);
 } finally {await browser.close();}
});

test('headless comment keyboard actions restore focus without stealing a new destination', {timeout:60000}, async()=>{
 const {browser,page,errors}=await fixture.openHeadlessUi();
 try {
  await openProject(page);await page.locator('#tools').click();await page.locator('#tool-comments').click();
  const cards=page.locator('.comment-card');
  await cards.first().getByRole('button',{name:'Resolve',exact:true}).focus();await page.keyboard.press('Enter');
  await cards.first().getByRole('button',{name:'Reopen',exact:true}).waitFor();
  assert.equal(await cards.first().getByRole('button',{name:'Reopen',exact:true}).evaluate(el=>el===document.activeElement),true);
  await page.keyboard.press('Enter');await cards.first().getByRole('button',{name:'Resolve',exact:true}).waitFor();
  assert.equal(await cards.first().getByRole('button',{name:'Resolve',exact:true}).evaluate(el=>el===document.activeElement),true);
  const draft=page.getByRole('textbox',{name:'Add a comment',exact:true});await draft.fill('Keyboard comment');
  await page.getByRole('button',{name:'Comment',exact:true}).focus();await page.keyboard.press('Enter');
  await page.waitForFunction(()=>document.querySelectorAll('.comment-card').length===3);
  assert.equal(await draft.evaluate(el=>el===document.activeElement),true);assert.equal(await draft.inputValue(),'');
  await cards.first().locator('.comment-reply-form summary').click();await cards.first().getByLabel('Write a reply').fill('Keyboard reply');
  await cards.first().getByRole('button',{name:'Reply',exact:true}).focus();await page.keyboard.press('Enter');
  await page.waitForFunction(()=>document.querySelector('.comment-replies')?.textContent?.includes('Keyboard reply'));
  assert.equal(await cards.first().getByLabel('Write a reply').evaluate(el=>el===document.activeElement),true);
  await page.evaluate(()=>{(window as unknown as StateWindow).fixture.delay=1000;});
  await cards.nth(1).getByRole('button',{name:'Resolve',exact:true}).focus();await page.keyboard.press('Enter');
  const elsewhere=page.locator('#file-search-input');await elsewhere.focus();
  await cards.nth(1).getByRole('button',{name:'Reopen',exact:true}).waitFor();
  assert.equal(await elsewhere.evaluate(el=>el===document.activeElement),true);
  await cards.nth(1).getByRole('button',{name:'Reopen',exact:true}).focus();await page.keyboard.press('Enter');
  const reference=cards.first().locator('.comment-reference');await reference.focus();
  await cards.nth(1).getByRole('button',{name:'Resolve',exact:true}).waitFor();
  assert.equal(await reference.evaluate(el=>el===document.activeElement),true);
  assert.equal(await page.evaluate(()=>(window as unknown as StateWindow).fixture.counters.commentLocate ?? 0),0);
  assert.deepEqual(errors,[]);
 } finally {await browser.close();}
});

test('headless typography loads bundled Inter and uses the stable T3 code stack', {timeout:60000}, async()=>{
 const {browser,page,errors}=await fixture.openHeadlessUi();
 try {
  await page.evaluate(()=>document.fonts.load('400 14px "Inter Variable"'));
  assert.equal(await page.evaluate(()=>Array.from(document.fonts).some(face=>face.family.includes('Inter Variable') && face.status==='loaded')),true);
  assert.match(await page.locator('body').evaluate(el=>getComputedStyle(el).fontFamily),/Inter Variable/);
  await openProject(page);
  assert.equal(await page.locator('#source').evaluate(el=>getComputedStyle(el).fontFamily),await page.locator('#highlight').evaluate(el=>getComputedStyle(el).fontFamily));
  assert.match(await page.locator('#source').evaluate(el=>getComputedStyle(el).fontFamily),/"SF Mono", SFMono-Regular, Menlo, Consolas/);
  assert.deepEqual(errors,[]);
 } finally {await browser.close();}
});


test('headless sidebar action geometry and explicit workspace layouts', {timeout:60000}, async()=>{
 const {browser,page,errors}=await fixture.openHeadlessUi('dark');
 try {
  assert.equal(await page.locator('#project-navigation').isHidden(),true);
  assert.equal(await page.locator('#projects').evaluate(el => el.clientWidth),1440);
  await openProject(page);
  const branding=await page.locator('.brand').evaluate(el=>{const logo=el.querySelector('img')!.getBoundingClientRect(),toggle=el.querySelector('#collapse')!.getBoundingClientRect();return [logo.left,logo.top,Math.abs(logo.top+logo.height/2-toggle.top-toggle.height/2)];});assert.deepEqual(branding,[18,43,0]);
  assert.equal((await page.locator('.workspace-topbar').boundingBox())?.height,36);
  assert.equal(await page.locator('#project-name').evaluate(el=>getComputedStyle(el).fontSize),'13px');
  const waitForTitlePadding = () => page.waitForFunction(() => {
   const sidebar = document.querySelector('.sidebar')!.getBoundingClientRect();
   const title = document.querySelector('#project-name')!.getBoundingClientRect();
   return Math.abs(title.left - sidebar.right - 12) < 0.5;
  });
  await waitForTitlePadding();
  await page.locator('#collapse').click();
  await page.locator('#home').waitFor({state:'hidden'});
  await waitForTitlePadding();
  await page.locator('#collapse').click();
  await waitForTitlePadding();
  await page.locator('.sidebar').evaluate(el=>(el as HTMLElement).style.setProperty('--sidebar-width','150px'));assert.deepEqual(await page.locator('.brand-home img').evaluate(el=>{const r=el.getBoundingClientRect();return [r.width,r.height];}),[28,28]);await page.locator('.sidebar').evaluate(el=>(el as HTMLElement).style.removeProperty('--sidebar-width'));
  const header=await page.locator('.project-trail').evaluate(el=>{const children=Array.from(el.children).map(child=>child.getBoundingClientRect()).filter(r=>r.height>0);const logo=document.querySelector('.brand-home img')!.getBoundingClientRect();return {centers:children.map(r=>r.top+r.height/2),top:document.querySelector('.workspace-topbar')!.getBoundingClientRect().top,logoCenter:logo.top+logo.height/2};});assert.ok(header.centers.every(y=>Math.abs(y-header.centers[0])<0.5),JSON.stringify(header));assert.equal(header.top,0);assert.ok(header.centers[0]<header.logoCenter);
  assert.equal(await page.locator('#home').evaluate(el=>getComputedStyle(el).getPropertyValue('-webkit-app-region')),'no-drag');
  const folder=page.locator('#file-list summary').first();
  const gaps=await folder.evaluate(el=>{const [arrow,icon,name]=Array.from(el.children).map(child=>child.getBoundingClientRect());return [icon.left-arrow.right,name.left-icon.right,Math.abs((arrow.top+arrow.height/2)-(icon.top+icon.height/2))];});assert.deepEqual(gaps,[8,8,0]);
  await folder.click();assert.equal(await folder.evaluate(el=>(el.parentElement as HTMLDetailsElement).open),false);await folder.press('Enter');assert.equal(await folder.evaluate(el=>(el.parentElement as HTMLDetailsElement).open),true);
  assert.equal(await page.locator('#outline-toggle svg path').getAttribute('d'),await folder.locator('.folder-chevron path').getAttribute('d'));
  assert.equal(await page.locator('#project-trust').count(),0);
  const actions=page.locator('.sidebar-actions>button');assert.equal(await actions.count(),3);
  const boxes=await actions.evaluateAll(nodes=>nodes.map(node=>{const r=node.getBoundingClientRect();return {y:r.y,width:r.width,height:r.height};}));
  assert.ok(boxes.every(box=>box.y===boxes[0].y && box.width===28 && box.height===32));
  assert.equal(await page.getByRole('button',{name:'Deleted files',exact:true}).getAttribute('title'),'Deleted files');
  assert.equal(await page.getByRole('button',{name:'Duplicate project',exact:true}).locator('svg').count(),1);
  await page.locator('#pane-preview').click();assert.equal(await page.locator('.pdf-pane').isVisible(),false);
  await page.locator('#pane-agent').click();assert.equal(await page.locator('#chat').isVisible(),false);
  await page.locator('#pane-editor').click();assert.equal(await page.locator('#source').isVisible(),true);
  await page.locator('#pane-preview').click();await page.locator('#pane-editor').click();
  assert.equal(await page.locator('.editor-pane').isVisible(),false);
  await page.locator('#pane-preview').click();assert.equal(await page.locator('#source').isVisible(),true);
  await page.locator('#tools').click();await page.getByRole('button',{name:'Expand tools panel',exact:true}).click();
  await page.locator('#pane-agent').click();assert.equal(await page.locator('#tools-panel').isVisible(),false);assert.equal(await page.locator('#chat').isVisible(),true);
  await page.setViewportSize({width:900,height:650});await page.locator('#pane-preview').click();
  assert.equal(await page.locator('#source').isVisible(),false);assert.equal(await page.locator('#chat').isVisible(),false);assert.equal(await page.locator('.pdf-pane').isVisible(),true);
  await page.locator('#pane-editor').click();assert.equal(await page.locator('#source').isVisible(),true);assert.equal(await page.locator('.pdf-pane').isVisible(),false);
  assert.deepEqual(errors,[]);
 } finally {await browser.close();}
});

test('headless icon controls share idle and hover colors in both themes', {timeout:60000}, async()=>{
 for (const theme of ['light','dark'] as const) {
  const {browser,page}=await fixture.openHeadlessUi(theme,{pdf:'',source:'% Icon appearance sample'});
  try {
   const colors=await page.locator('html').evaluate(el=>{
    const style=getComputedStyle(el);
    return ['--sidebar-muted','--text','--hover'].map(property=>style.getPropertyValue(property).trim());
   });
   const [idle,hover,box]=colors.map(hex=>`rgb(${hex.slice(1).match(/../g)!.map(pair=>parseInt(pair,16)).join(', ')})`);
   async function check(selector:string, glyph='svg'):Promise<void> {
    const button=page.locator(selector);
    const icon=glyph ? button.locator(glyph) : button;
    await page.mouse.move(0,0);
    assert.equal(await icon.evaluate(el=>getComputedStyle(el).color),idle,selector);
    await button.hover();
    assert.equal(await icon.evaluate(el=>getComputedStyle(el).color),hover,selector);
    assert.equal(await button.evaluate(el=>getComputedStyle(el).backgroundColor),box,selector);
   }
   await check('#settings');
   await check('#deleted-projects');
   await page.getByRole('button',{name:'Open Spectral methods for fluid dynamics',exact:true}).click();
   await page.locator('#source').waitFor({state:'visible'});
   for (const selector of ['#duplicate','#pane-editor','#attach','#send']) await check(selector);
   await page.locator('#chats-tab').click();
   await check('.chat-rename','');
  } finally {await browser.close();}
 }
});

test('headless project search has its own row and retains content search', {timeout:30000}, async()=>{
 for (const theme of ['light','dark'] as const) {
  const {browser,page,errors}=await fixture.openHeadlessUi(theme,{pdf:'',source:'Introduction'});
  try {
   await page.evaluate(()=>{
    const {store}=window as unknown as {store:{search:(id:string,query:string)=>Promise<unknown>}};
    store.search=async(_id,query)=>query==='Introduction' ? [{file:'main.tex',line:1,column:0,text:'Introduction'}] : [];
   });
   await page.getByRole('button',{name:'Open Spectral methods for fluid dynamics',exact:true}).click();
   await page.locator('#source').waitFor({state:'visible'});
   const search=page.getByRole('textbox',{name:'Search project'});
   const toolbar=page.locator('.tabs');
   const tab=await toolbar.locator('#files-tab').boundingBox();
   const field=await search.boundingBox();
   assert.ok(tab && field && field.y>tab.y+tab.height);
   for (const action of await toolbar.locator('.sidebar-actions button').all()) {
    const box=await action.boundingBox();
    assert.ok(box && box.y<tab.y+tab.height && box.y+box.height>tab.y);
   }
   await search.fill('Introduction');
   await page.getByRole('button',{name:'main.tex:1 Introduction'}).click();
   assert.equal(await page.locator('#source').evaluate((editor:HTMLTextAreaElement)=>editor.value.slice(editor.selectionStart,editor.selectionEnd)),'Introduction');
   await page.getByRole('button',{name:'Clear file search'}).click();
   assert.equal(await search.inputValue(),'');
   assert.equal(await page.locator('#file-search-results button').count(),0);
   await search.fill('missing');
   await page.getByText('No matches. Try another search.').waitFor();
   await search.press('Escape');
   assert.equal(await search.inputValue(),'');
   assert.equal(await search.evaluate(el=>el===document.activeElement),true);
   await page.locator('#chats-tab').click();
   assert.equal(await search.isVisible(),false);
   await page.locator('#files-tab').click();
   assert.equal(await search.isVisible(),true);
   await page.locator('.sidebar').evaluate(el=>(el as HTMLElement).style.setProperty('--sidebar-width','150px'));
   const sidebar=await page.locator('.sidebar').boundingBox();
   const narrowField=await search.boundingBox();
   assert.ok(sidebar && narrowField && narrowField.x+narrowField.width<=sidebar.x+sidebar.width);
   assert.deepEqual(errors,[]);
  } finally {await browser.close();}
 }
});
