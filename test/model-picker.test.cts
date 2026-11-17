const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const path = require('node:path');
const { _electron: electron } = require('playwright');
const root = path.resolve(__dirname, '..', '..');
async function picker(t, mode = 'ok') {
 const env = { ...process.env }; delete env.ELECTRON_RUN_AS_NODE;
 const app = await electron.launch({ args: [path.join(root, 'build/test/fixtures/picker-host.cjs')], env });
 t.after(() => app.close());
 const page = await app.firstWindow(); page.setDefaultTimeout(5000);
 await page.setContent('<div class="chat" style="width:500px;height:160px;max-height:none"><div id="chat-messages"></div><div class="composer"><label class="provider-label" for="chat-provider">Provider</label><select id="chat-provider"><option value="codex">Codex</option><option value="opencode">OpenCode</option></select><div id="model-picker"></div></div></div><button id="outside" style="position:fixed;top:0;right:0;z-index:20">Outside</button>');
 await page.addStyleTag({ content: await fs.readFile(path.join(root, 'build/style.css'), 'utf8') });
 await page.evaluate(mode => {
   const w = window as any; w.saves = []; w.failSave = mode === 'save';
   w.store = {
     discoverCodexModels: async () => { if (mode === 'reject') throw new Error('offline'); if (mode === 'error') return {ok:false,error:'offline'}; return {ok:true,models: (mode === 'versions' || mode === 'first') ? ['GPT-6', 'GPT-6.1', 'GPT-5.10', 'GPT-5.9'].map(name => ({id:name,model:name,displayName:name,description:'',supportedReasoningEfforts: mode === 'first' ? [{effort:'low',description:''},{effort:'medium',description:''},{effort:'high',description:''}] : [],defaultReasoningEffort:null,isDefault:false})) : [{id:'catalog-id',model:'cli-model',displayName:'Alpha',description:'A model',supportedReasoningEfforts:[{effort:'low',description:''},{effort:'high',description:''}],defaultReasoningEffort:'low',isDefault:false},{id:'other-id',model:'other',displayName:'Beta',description:'',supportedReasoningEfforts:[],defaultReasoningEffort:null,isDefault:false}]}; },
     discoverOpenCodeModels: async () => ({ok:true,models:[{id:'opencode/model-2',name:'model-2',variants:[]},{id:'opencode/model-10',name:'model-10',variants:[]}]}),
     openCodePreferencesLoad: async () => ({model:mode === 'first' ? null : 'opencode/model-2',reasoningEffort:null,favorites:[]}),
     openCodePreferencesSave: async value => { w.openCodeSave = value; },
     codexPreferencesLoad: async () => { if(mode === 'load') throw new Error('bad data'); return mode === 'first' ? {model:null,reasoningEffort:null,favorites:[]} : {model:'cli-model',reasoningEffort:'low',favorites:['other']}; },
     codexPreferencesSave: async value => { w.saves.push(value); if (w.failSave) throw new Error('disk'); }
   };
 }, mode);
 await page.addScriptTag({ content: await fs.readFile(path.join(root, 'build/model-picker.js'), 'utf8') });
 return page;
}
test('picker restores, searches, navigates, favorites and saves supported choices', {timeout:30000}, async t => {
 const page = await picker(t); const trigger = page.locator('.model-trigger'); const panel = page.locator('.model-panel');
 await trigger.waitFor({state:'visible'}); await page.waitForFunction(() => !(document.querySelector('.model-trigger') as HTMLButtonElement).disabled);
 assert.equal(await trigger.textContent(), 'Alpha · Low'); assert.equal(await panel.isVisible(), false);
 await trigger.click(); assert.equal(await panel.isVisible(), true); assert.equal(await page.locator('.model-row .provider-logo img').count(), 2); assert.equal(await page.getByLabel('Codex models').locator('img').count(), 1); assert.equal(await page.getByLabel('OpenCode models').locator('img').count(), 1); assert.match(await page.locator('.model-row').first().innerText(), /Alpha/);
 await page.getByLabel('Search models').fill('alpha'); assert.equal(await page.locator('.model-row').count(), 1);
 await page.getByLabel('Add Alpha to favorites').click(); await page.waitForFunction(() => (window as any).saves.length === 1);
 assert.deepEqual(await page.evaluate(() => (window as any).saves[0].favorites), ['other','cli-model']);
 await page.getByLabel('Search models').press('ArrowDown'); assert.match(await page.evaluate(() => document.activeElement.textContent), /Alpha/);
 await page.getByRole('button', {name:'High', exact:true}).click(); await page.waitForFunction(() => (window as any).saves.length === 2);
 assert.match(await trigger.textContent(), /High$/);
 assert.deepEqual(await page.evaluate(() => (window as any).saves[1]), {model:'cli-model',reasoningEffort:'high',favorites:['other','cli-model']});
 await page.getByLabel('Search models').press('Enter'); assert.equal(await panel.isVisible(), false);
 await trigger.click(); await page.keyboard.press('Escape'); assert.equal(await panel.isVisible(), false); assert.equal(await trigger.getAttribute('aria-expanded'),'false');
 await trigger.click(); await page.getByLabel('Search models').fill('beta'); await page.getByLabel('Search models').press('Enter'); await page.waitForFunction(() => (window as any).saves.length === 3); assert.deepEqual(await page.evaluate(() => (window as any).saves[2]), {model:'other',reasoningEffort:null,favorites:['other','cli-model']}); await trigger.click(); await page.locator('#outside').click(); assert.equal(await panel.isVisible(), false);
});
test('discovery errors preserve preferences and preference load failure blocks edits', {timeout:30000}, async t => {
 for (const mode of ['error','reject']) { const page = await picker(t, mode); await page.waitForFunction(() => document.querySelector('[role=status]').textContent.includes('offline')); assert.match(await page.locator('.model-trigger').textContent(), /cli-model \(unavailable\)/); assert.equal(await page.evaluate(() => (window as any).saves.length), 0); await page.locator('.model-trigger').click(); assert.equal(await page.getByText('Models could not be loaded.').isVisible(), true); }
 const page = await picker(t, 'load'); await page.waitForFunction(() => document.querySelector('[role=status]').textContent.includes('Could not load model preferences')); assert.equal(await page.locator('.model-trigger').isDisabled(), false); await page.locator('.model-trigger').click(); assert.equal(await page.locator('.model-pane').isVisible(), false); await page.getByLabel('OpenCode models').click(); assert.equal(await page.locator('.model-trigger').textContent(), 'model-2'); assert.equal(await page.evaluate(() => (window as any).saves.length), 0);
});
test('save failures are visible and retry saves the latest snapshot', {timeout:30000}, async t => {
 const page = await picker(t, 'save'); await page.waitForFunction(() => !(document.querySelector('.model-trigger') as HTMLButtonElement).disabled); await page.locator('.model-trigger').click(); await page.getByRole('button', {name:'High', exact:true}).click(); await page.getByRole('button',{name:'Retry saving'}).waitFor({state:'visible'}); await page.evaluate(() => { (window as any).failSave = false; }); await page.getByRole('button',{name:'Retry saving'}).click(); await page.getByRole('button',{name:'Retry saving'}).waitFor({state:'hidden'}); assert.equal(await page.evaluate(() => (window as any).saves.at(-1).reasoningEffort), 'high');
});

test('sending waits for the latest preference save and rejects failed saves', {timeout:30000}, async t => {
 const page = await picker(t);
 await page.waitForFunction(() => !(document.querySelector('.model-trigger') as HTMLButtonElement).disabled);
 await page.evaluate(() => { const w = window as any; w.store.codexPreferencesSave = async value => { w.saves.push(value); await new Promise<void>(resolve => { w.releaseSave = resolve; }); }; });
 await page.locator('.model-trigger').click(); await page.getByRole('button', {name:'High', exact:true}).click();
 await page.evaluate(() => { const w = window as any; w.ready = false; void w.codexPickerReady().then(() => { w.ready = true; }); });
 assert.equal(await page.evaluate(() => (window as any).ready), false);
 await page.evaluate(() => (window as any).releaseSave()); await page.waitForFunction(() => (window as any).ready);
 const failed = await picker(t, 'save'); await failed.waitForFunction(() => !(document.querySelector('.model-trigger') as HTMLButtonElement).disabled);
 await failed.locator('.model-trigger').click(); await failed.getByRole('button', {name:'High', exact:true}).click();
 await failed.getByRole('button',{name:'Retry saving'}).waitFor({state:'visible'});
 assert.match(await failed.evaluate(async () => { try { await (window as any).codexPickerReady(); return ''; } catch (error) { return (error as Error).message; } }), /Could not save/);
});

test('orders versions numerically and filters favorites', {timeout:30000}, async t => {
 const page = await picker(t, 'versions'); await page.waitForFunction(() => !(document.querySelector('.model-trigger') as HTMLButtonElement).disabled);
 await page.locator('.model-trigger').click();
 assert.deepEqual(await page.locator('.model-row button:first-child').allTextContents(), ['GPT-5.10','GPT-5.9','GPT-6','GPT-6.1','CLI default']);
 await page.getByLabel('Add GPT-5.9 to favorites').click(); await page.getByLabel('Favorite models').click();
 assert.deepEqual(await page.locator('.model-row button:first-child').allTextContents(), ['GPT-5.9']);
 await page.getByLabel('Codex models').click(); assert.equal(await page.locator('.model-row').count(), 5);
});

test('provider can be selected inside the model picker', {timeout:30000}, async t => {
 const page = await picker(t);
 await page.waitForFunction(() => !(document.querySelector('.model-trigger') as HTMLButtonElement).disabled);
 await page.locator('.model-trigger').click();
 await page.getByLabel('OpenCode models').click();
 assert.equal(await page.locator('.model-trigger').textContent(), 'model-2');
 assert.equal(await page.locator('.model-pane').isVisible(), true);
 assert.equal(await page.locator('#chat-provider').inputValue(), 'opencode');
 assert.match(await page.locator('.model-list').innerText(), /model-10/); await page.getByRole('button', {name:'model-10', exact:true}).click(); await page.waitForFunction(() => (window as any).openCodeSave?.model === 'opencode/model-10'); await page.getByLabel('Codex models').click();
 assert.equal(await page.locator('#chat-provider').inputValue(), 'codex');
});

test('provider tabs respect locked chats and unavailable providers', {timeout:30000}, async t => {
 const page = await picker(t);
 await page.waitForFunction(() => !(document.querySelector('.model-trigger') as HTMLButtonElement).disabled);
 await page.evaluate(() => { const provider = document.querySelector('#chat-provider') as HTMLSelectElement; provider.disabled = true; provider.dispatchEvent(new Event('providerstate')); });
 await page.locator('.model-trigger').click();
 assert.equal(await page.getByLabel('OpenCode models').isDisabled(), true);
 await page.getByLabel('Favorite models').click(); await page.getByLabel('Codex models').click();
 assert.equal(await page.getByLabel('Codex models').getAttribute('aria-pressed'), 'true');
});

test('Codex defaults remain unset and can be restored', {timeout:30000}, async t => {
 const page = await picker(t, 'first'); await page.waitForFunction(() => !(document.querySelector('.model-trigger') as HTMLButtonElement).disabled);
 assert.equal(await page.locator('.model-trigger').textContent(), 'CLI default');
 assert.equal(await page.evaluate(() => (window as any).saves.length), 0);
 await page.evaluate(() => (window as any).codexPickerReady());
 await page.locator('.model-trigger').click(); await page.getByRole('button',{name:'GPT-6.1',exact:true}).click();
 await page.waitForFunction(() => (window as any).saves.length === 1);
 assert.deepEqual(await page.evaluate(() => (window as any).saves[0]), {model:'GPT-6.1',reasoningEffort:null,favorites:[]});
 await page.locator('.model-trigger').click(); await page.getByRole('button',{name:'High',exact:true}).click();
 await page.getByRole('button',{name:'Default effort'}).click();
 await page.getByRole('button',{name:'CLI default'}).click();
 await page.waitForFunction(() => (window as any).saves.at(-1)?.model === null);
 assert.deepEqual(await page.evaluate(() => (window as any).saves.at(-1)), {model:null,reasoningEffort:null,favorites:[]});
});

test('OpenCode models show their variants and retain or choose a nearby effort', {timeout:30000}, async t => {
 const page = await picker(t);
 await page.evaluate(() => { const w = window as any; w.store.discoverOpenCodeModels = async () => ({ok:true,models:[]}); });
 // Reload the picker with a controlled catalog and persisted choice.
 await page.setContent('<div class="chat" style="width:500px;height:160px;max-height:none"><div id="chat-messages"></div><div class="composer"><label class="provider-label"></label><select id="chat-provider"><option value="opencode">OpenCode</option></select><div id="model-picker"></div></div></div>');
 await page.addStyleTag({content:await fs.readFile(path.join(root,'build/style.css'),'utf8')});
 await page.evaluate((base: string) => { const element = document.createElement('base'); element.href = base; document.head.append(element); }, require('node:url').pathToFileURL(path.join(root, 'build') + path.sep).href);
 await page.evaluate(() => { const w = window as any; w.store.discoverOpenCodeModels = async () => ({ok:true,models:[{id:'opencode/deepseek-v4.1-flash-free',name:'DeepSeek V4.1 Flash Free',variants:['low','high','max']},{id:'lmstudio/qwen3.5-9b',name:'Qwen 3.5 9B',variants:['high','max']},{id:'opencode/beam-free',name:'Beam Free',variants:['fast','balanced','deep']},{id:'opencode/big-pickle',name:'Big Pickle',variants:[]}]}); w.store.openCodePreferencesLoad = async () => ({model:'opencode/deepseek-v4.1-flash-free',reasoningEffort:'low',favorites:[]}); });
 await page.addScriptTag({content:await fs.readFile(path.join(root,'build/model-picker.js'),'utf8')});
 await page.waitForFunction(() => document.querySelector('.model-trigger')?.textContent?.includes('DeepSeek V4.1 Flash Free'));
 await page.locator('.model-trigger').click();
 await page.getByLabel('Search models').fill('DeepSeek V4.1 Flash');
 assert.equal(await page.locator('.model-row').count(), 1);
 await page.getByLabel('Search models').fill('');
 if (process.env.REFRACT_PICKER_SCREENSHOTS) {
  const dir = process.env.REFRACT_PICKER_SCREENSHOTS;
  await page.emulateMedia({colorScheme:'dark'});
  const logo = 'data:image/svg+xml;base64,' + (await fs.readFile(path.join(root,'build/provider-logos/opencode-dark.svg'))).toString('base64');
  await page.evaluate((data: string) => { document.querySelectorAll('.provider-logo source').forEach(e => e.remove()); document.querySelectorAll<HTMLImageElement>('.provider-logo img').forEach(e => e.src = data); }, logo);
  await fs.mkdir(dir,{recursive:true});
  await page.locator('.model-panel').screenshot({path:path.join(dir,'model-picker.png')});
  for (const name of ['DeepSeek V4.1 Flash Free','Qwen 3.5 9B']) {
   const row = page.locator('.model-row').filter({has:page.getByRole('button',{name})});
   await page.mouse.move(0,0); await row.screenshot({path:path.join(dir,`${name}-default.png`)});
   await row.locator('button').first().hover(); await row.screenshot({path:path.join(dir,`${name}-hover.png`)});
  }
 }
 assert.equal(await page.locator('.effort-options button').filter({hasText:'Low'}).getAttribute('aria-pressed'),'true');
 await page.locator('.model-provider-tooltip').filter({hasText:'Qwen 3.5 9B'}).click();
 assert.equal(await page.locator('.effort-options button').filter({hasText:'High'}).getAttribute('aria-pressed'),'true');
 assert.equal(await page.locator('.model-panel').isVisible(),true);
 await page.getByRole('button',{name:'Max',exact:true}).click();
 await page.locator('.model-provider-tooltip').filter({hasText:'DeepSeek V4.1 Flash Free'}).click();
 assert.equal(await page.locator('.effort-options button').filter({hasText:'Max'}).getAttribute('aria-pressed'),'true');
 await page.locator('.model-provider-tooltip').filter({hasText:'Beam Free'}).click();
 assert.equal(await page.locator('.effort-options button').filter({hasText:'Balanced'}).getAttribute('aria-pressed'),'true');
 await page.locator('.model-provider-tooltip').filter({hasText:'Big Pickle'}).click();
 assert.equal(await page.locator('.effort-options button').count(),0);
 await page.waitForFunction(() => (window as any).openCodeSave?.model === 'opencode/big-pickle');
 assert.equal(await page.evaluate(() => (window as any).openCodeSave.reasoningEffort),null);
 assert.equal(await page.locator('.model-provider-tooltip').filter({hasText:'Qwen 3.5 9B'}).getAttribute('data-provider'),'LM Studio');
 assert.equal(await page.locator('.model-provider-tooltip').filter({hasText:'DeepSeek V4.1 Flash Free'}).getAttribute('data-provider'),'OpenCode');
});

test('OpenCode catalog follows project changes and ignores stale results', {timeout:30000}, async t => {
 const page = await picker(t); await page.waitForFunction(() => !(document.querySelector('.model-trigger') as HTMLButtonElement).disabled);
 await page.evaluate(() => { const w = window as any; w.store.discoverOpenCodeModels = (id: string) => id === 'first' ? new Promise(resolve => { w.finishOld = resolve; }) : Promise.resolve({ok:true,models:[{id:'lmstudio/new-model',name:'new-model',variants:['high']}]}); window.dispatchEvent(new CustomEvent('picker-project',{detail:'first'})); window.dispatchEvent(new CustomEvent('picker-project',{detail:'second'})); });
 await page.evaluate(() => (window as any).finishOld({ok:true,models:[{id:'opencode/old-model',name:'old-model',variants:['low']}]}));
 await page.locator('.model-trigger').click(); await page.getByLabel('OpenCode models').click();
 await page.getByText('new-model',{exact:true}).waitFor(); assert.equal(await page.getByText('old-model',{exact:true}).count(),0);
});

test('project discovery does not overwrite a newer OpenCode selection', {timeout:30000}, async t => {
 const page = await picker(t); await page.waitForFunction(() => !(document.querySelector('.model-trigger') as HTMLButtonElement).disabled);
 await page.locator('.model-trigger').click(); await page.getByLabel('OpenCode models').click();
 await page.evaluate(() => { const w = window as any; w.store.discoverOpenCodeModels = () => new Promise(resolve => { w.finishProject = resolve; }); window.dispatchEvent(new CustomEvent('picker-project',{detail:'new-project'})); });
 await page.locator('.model-row button').first().click();
 const selection = await page.locator('.model-trigger').textContent();
 await page.evaluate(() => (window as any).finishProject({ok:true,models:[{id:'opencode/model-a',name:'Model A',variants:['low']}]}));
 await page.waitForFunction(() => document.querySelector('.model-list')?.textContent?.includes('Model A'));
 assert.equal(await page.locator('.model-trigger').textContent(), selection?.includes('(unavailable)') ? selection : selection?.replace(/(.*?)( \u00b7.*)?$/, '$1 (unavailable)$2'));
});
test('provider tab logos remain within their labeled controls across zoom levels and window sizes', {timeout:30000}, async t => {
 const page = await picker(t);
 await page.waitForFunction(() => !(document.querySelector('.model-trigger') as HTMLButtonElement).disabled);
 await page.locator('.model-trigger').click();
 for (const size of [{width:800,height:600},{width:1280,height:800},{width:1920,height:1080}]) {
  await page.setViewportSize(size);
  for (const zoom of [0.5,0.67,0.8,1,1.1,1.25,1.5,2,3]) {
   await page.evaluate(z => { document.body.style.zoom = String(z); }, zoom);
   for (const name of ['Codex models','OpenCode models']) {
    const tab = page.getByLabel(name); const {button, logo} = await tab.evaluate(element => ({button: element.getBoundingClientRect().toJSON(), logo: element.querySelector('.provider-logo')!.getBoundingClientRect().toJSON()}));
    assert.ok(button && logo);
    assert.ok(logo.x >= button.x - 1 && logo.x + logo.width <= button.x + button.width + 1, `${name} horizontal at ${zoom}`);
    assert.ok(Math.abs((button.y + button.height / 2) - (logo.y + logo.height / 2)) < 1, `${name} vertical at ${zoom}`);
   }
  }
 }
});

 test('Codex speed persists, stays open, and is hidden for OpenCode', {timeout:30000}, async t => {
 const page = await picker(t); await page.waitForFunction(() => !(document.querySelector('.model-trigger') as HTMLButtonElement).disabled);
 await page.locator('.model-trigger').click();
 await page.getByRole('button',{name:'Fast',exact:true}).click();
 assert.equal(await page.getByRole('button',{name:'Fast',exact:true}).getAttribute('aria-pressed'),'true');
 await page.evaluate(() => (window as any).codexPickerReady());
 assert.equal(await page.evaluate(() => (window as any).saves.at(-1).fastMode),true);
 assert.match(await page.locator('.model-trigger').textContent(),/Fast/);
 await page.emulateMedia({colorScheme:'dark'}); await page.setViewportSize({width:1000,height:850});
 await page.locator('.model-trigger').click(); await page.locator('.model-trigger').click();
 if (process.env.REFRACT_PICKER_SCREENSHOT) {
 const logos: Record<string,string> = {};
 for (const name of ['openai-white','openai-black','opencode-dark','opencode-light']) logos[name] = 'data:image/svg+xml;base64,' + (await fs.readFile(path.join(root,'resources/provider-logos',name+'.svg'))).toString('base64');
 await page.evaluate(logos => { document.querySelectorAll<HTMLImageElement>('.provider-logo img').forEach(img => { img.src = logos[img.src.split('/').pop()!.replace('.svg','')]; }); document.querySelectorAll<HTMLSourceElement>('.provider-logo source').forEach(source => { source.srcset = logos[source.srcset.split('/').pop()!.replace('.svg','')]; }); },logos);
 await page.locator('.model-panel').screenshot({path:process.env.REFRACT_PICKER_SCREENSHOT});
 }
 await page.getByLabel('OpenCode models',{exact:true}).click(); assert.equal(await page.getByRole('group',{name:'Codex speed'}).isVisible(),false);
 await page.getByLabel('Codex models',{exact:true}).click(); assert.equal(await page.getByRole('button',{name:'Fast',exact:true}).getAttribute('aria-pressed'),'true');
 await page.getByRole('button',{name:'Standard',exact:true}).click(); await page.evaluate(() => (window as any).codexPickerReady());
 assert.equal(await page.evaluate(() => (window as any).saves.at(-1).fastMode),false);
 });
test('faster replies selects a supported lower effort only after an explicit click',async t=>{
 const page=await picker(t);await page.waitForFunction(()=>!(document.querySelector('.model-trigger') as HTMLButtonElement).disabled);
 await page.locator('.model-trigger').click();await page.getByRole('button',{name:'High',exact:true}).click();
 await page.waitForFunction(()=>(window as any).saves.at(-1)?.reasoningEffort==='high');
 await page.getByRole('button',{name:'Faster replies',exact:true}).click();
 await page.waitForFunction(()=>(window as any).saves.at(-1)?.reasoningEffort==='low');
});

test('keyboard model controls retain focus when favorites and reasoning rerender', async (t: import('node:test').TestContext) => {
 const page: import('playwright').Page = await picker(t);
 await page.waitForFunction(() => !(document.querySelector('.model-trigger') as HTMLButtonElement).disabled);
 await page.locator('.model-trigger').click();
 assert.equal(await page.locator('[data-focus-key="model:cli-model"]').getAttribute('aria-pressed'), 'true');
 await page.getByLabel('Add Alpha to favorites').focus();
 await page.keyboard.press('Enter');
 assert.equal(await page.getByLabel('Remove Alpha from favorites').evaluate(el => el === document.activeElement), true);
 await page.getByRole('button', {name:'High', exact:true}).focus();
 await page.keyboard.press('Enter');
 assert.equal(await page.getByRole('button', {name:'High', exact:true}).evaluate(el => el === document.activeElement), true);
 assert.equal(await page.getByRole('button', {name:'High', exact:true}).getAttribute('aria-pressed'), 'true');
 await page.getByLabel('Favorite models').click();
 await page.getByLabel('Remove Alpha from favorites').focus();
 await page.keyboard.press('Enter');
 assert.equal(await page.getByLabel('Search models').evaluate(el => el === document.activeElement), true);
 for (const width of [1440, 1000, 700]) {
  await page.setViewportSize({width, height:850});
  const bounds = await page.locator('.model-panel').boundingBox();
  assert.ok(bounds && bounds.x >= 0 && bounds.x + bounds.width <= width);
  assert.equal(Math.round(bounds.width), width > 1200 ? 540 : width > 800 ? 520 : 480);
 }
});
