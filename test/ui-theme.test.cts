const {launchTestDesktop,useFakeCodex}=require('./electron-fixture.cjs');
const { test } = require('node:test') as typeof import('node:test');
const assert: typeof import('node:assert/strict') = require('node:assert/strict');
const fs = require('node:fs/promises') as typeof import('node:fs/promises');
const os = require('node:os') as typeof import('node:os');
const path = require('node:path') as typeof import('node:path');
export {};

async function resizeDesktop(app: import('playwright').ElectronApplication, page: import('playwright').Page, width: number, height: number): Promise<void> {
 // Resize the native window so Electron updates its window-controls overlay geometry.
 await app.evaluate(({BrowserWindow}, size) => BrowserWindow.getAllWindows()[0].setContentSize(size.width, size.height), {width, height});
 await page.waitForFunction(size => Math.abs(window.innerWidth - size.width) <= 2 && Math.abs(window.innerHeight - size.height) <= 2, {width, height});
}

async function chooseView(page: import('playwright').Page, view:'split'|'editor'|'pdf'):Promise<void> {
 // Hidden Electron windows do not reliably deliver animation frames after viewport emulation.
 await page.evaluate(()=>window.dispatchEvent(new Event('resize')));
 if(await page.locator('#tools-panel').isVisible()) await page.locator('#tools').click();
 const target={agent:view!=='pdf',editor:view!=='pdf',preview:view!=='editor'};
 for(const enabled of [true,false]) for(const name of ['agent','editor','preview'] as const) {
  const button=page.locator(`#pane-${name}`);
  if(target[name]===enabled && (await button.getAttribute('aria-pressed')==='true')!==enabled) await button.click();
 }
 await page.locator('#pane-editor').focus();
}

test('standard palette stays coherent across light and dark app surfaces', { timeout: 180000 }, async () => {
 const data = await fs.mkdtemp(path.join(os.tmpdir(), 'refract-theme-'));
 const { ProjectStore }: { ProjectStore: new (directory: string) => { create(name: string): Promise<{ id: string }>; write(id: string, file: string, text: string): Promise<void> } } = require('../src/project-store.cjs');
 const { ChatService }: { ChatService: new (store: object) => { save(id: string, data: { sessionId: null; messages: { role: string; text: string }[] }): Promise<void> } } = require('../src/chat.cjs');
 const projectStore = new ProjectStore(path.join(data, 'projects'));
 for (const name of ['Spectral methods for fluid dynamics', 'Conference paper - autumn notes', 'A short introduction to graph theory']) {
  const project = await projectStore.create(name);
  if (name.startsWith('Conference')) await new ChatService(projectStore).save(project.id, {sessionId:null, messages:[{role:'user', text:'How should I structure the introduction?'}, {role:'assistant', text:'Start with the **research question**, then connect the existing work to your contribution.\n\n1. Set the context.\n2. Identify the gap.\n3. State your contribution.\n\nUse `\\section{Introduction}` to begin the section.'}]});
  await projectStore.write(project.id, 'sections/abstract.tex', '% Research notes\nWe investigate a stable spectral approximation.\n');
 }
 const env: NodeJS.ProcessEnv = { ...process.env, REFRACT_DATA_DIR: data, REFRACT_TEST_HIDDEN: '1' };
 delete env.ELECTRON_RUN_AS_NODE;
 const app = await launchTestDesktop({ args: [path.resolve(__dirname, '../..')], env });
 try {
  const page = await app.firstWindow();
  assert.equal(await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows().some(window => window.isVisible())), false);
  await useFakeCodex(app);
  await page.reload();
  // Hidden windows must still process frames during layout and focus checks.
  await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0].webContents.setBackgroundThrottling(false));
  const errors: string[] = []; page.on('pageerror', error => errors.push(error.message));
  await page.waitForFunction(() => !document.body.inert, undefined, {timeout:10000});
  const artifacts = process.env.REFRACT_THEME_ARTIFACTS;
  if (artifacts) await fs.mkdir(artifacts, { recursive: true });
  const capture = async (name: string): Promise<void> => {
   if (artifacts) await page.screenshot({ path: path.join(artifacts, `${name}.png`) });
  };
  for (const mode of ['light', 'dark'] as const) {
   await app.evaluate(({ nativeTheme }, theme) => { nativeTheme.themeSource = theme; }, mode);
   await page.emulateMedia({ colorScheme: mode });
   if (await page.locator('#workspace').isVisible()) await page.locator('#home').click();
   assert.equal(await page.locator('#project-navigation').isHidden(), true);
   const background = (selector: string) => page.locator(selector).evaluate(el => getComputedStyle(el).backgroundColor);
   assert.equal(await background('body'), mode === 'dark' ? 'rgb(10, 10, 10)' : 'rgb(252, 252, 252)');
   assert.equal(await background('.sidebar'), mode === 'dark' ? 'rgb(0, 0, 0)' : 'rgb(250, 250, 250)');
   assert.equal(await background('.app-titlebar'), await background('.sidebar'));
   assert.equal(await background('#new'), mode === 'dark' ? 'rgb(52, 107, 241)' : 'rgb(27, 78, 216)');
   await capture(`${mode}-projects`);
   await page.getByRole('button', { name: 'Open Spectral methods for fluid dynamics', exact: true }).click();
   await page.locator('#source').waitFor({ state: 'visible' });
   await page.getByText('Compiled', { exact: true }).waitFor({ timeout: 45000 });
   await page.locator('#pdf-area canvas').first().waitFor();
   await page.locator('#file-list button[title="sections/abstract.tex"]').click();
   await page.getByRole('button', { name: 'main.tex', exact: true }).click();
   await capture(`${mode}-workspace`);
   assert.equal(await page.locator('.editor-actions').evaluate(el => el.scrollWidth <= el.clientWidth), true);
   assert.equal(await page.locator('body').evaluate(el => el.scrollWidth <= el.clientWidth), true);
   await page.locator('#chat-input').focus();
   assert.match(await page.locator('.composer').evaluate(el => getComputedStyle(el).boxShadow), mode === 'dark' ? /52, 107, 241/ : /27, 78, 216/);
   await page.locator('#project-name').click();
   await capture(`${mode}-dialog`);
   assert.equal(await background('#rename-dialog'), mode === 'dark' ? 'rgb(17, 17, 17)' : 'rgb(255, 255, 255)');
   await page.locator('#rename-dialog button[value=cancel]').click();
   await page.locator('#tools').click();
   await page.locator('#tool-info').click();
   await capture(`${mode}-tools`);
   await page.locator('#tool-comments').click();
   await page.getByRole('textbox', {name:'Add a comment'}).fill('Clarify the assumptions before stating the main result.');
   await page.getByRole('button', {name:'Comment', exact:true}).click();
   await capture(`${mode}-comments`);
   await page.locator('#tool-versions').click();
   await capture(`${mode}-versions`);
   await page.locator('#tools').click();
   await page.locator('.model-trigger').click();
   await page.getByRole('button',{name:'Fixture model',exact:true}).click();
   await capture(`${mode}-models`);
   const bounds = await page.locator('.model-panel').boundingBox();
   assert.ok(bounds && bounds.x >= 0 && bounds.y >= 0 && bounds.x + bounds.width <= 1440);
   await page.keyboard.press('Escape');
   await resizeDesktop(app, page, 1000, 740);
   await capture(`${mode}-compact`);
   await chooseView(page,'pdf');
   await capture(`${mode}-compact-preview`);
   await page.locator('#pane-editor').click();
   await resizeDesktop(app, page, 900, 650);
   await capture(`${mode}-short`);
   await resizeDesktop(app, page, 1440, 900);
   await chooseView(page,'split');
   await page.locator('#home').click();
   assert.equal(await page.locator('#project-navigation').isHidden(), true);
   await capture(`${mode}-populated`);
   await page.locator('#grid-view').click();
   await capture(`${mode}-grid`);
   await page.locator('#list-view').click();
   await page.getByRole('button', {name:'Open Conference paper - autumn notes', exact:true}).click();
   await page.locator('.message.assistant').waitFor();
   await capture(`${mode}-conversation`);
   assert.ok(await page.locator('#chat-messages').evaluate(el => el.clientHeight) >= 180);
   for (const id of ['sidebar-resize','workspace-resize','chat-resize']) {
    const handle = page.locator(`#${id}`);
    const before = Number(await handle.getAttribute('aria-valuenow'));
    await handle.focus();
    await page.keyboard.press(id === 'chat-resize' ? 'ArrowUp' : 'ArrowRight');
    const after = Number(await handle.getAttribute('aria-valuenow'));
    assert.ok(Number.isFinite(after) && after !== before);
    assert.ok(after >= Number(await handle.getAttribute('aria-valuemin')));
    assert.ok(after <= Number(await handle.getAttribute('aria-valuemax')));
   }
   await page.getByRole('button', {name:'Expand chat', exact:true}).click();
   await capture(`${mode}-expanded-chat`);
   await page.getByRole('button', {name:'Show editor', exact:true}).click();
  }
  assert.deepEqual(errors, []);
 } catch (error) {
  console.error(error);
  const page = await app.firstWindow();
  await page.screenshot({path:path.join(os.tmpdir(), 'refract-theme-failure.png')});
  console.error(await page.locator('body').innerText());
  throw error;
 } finally {
  await app.close();
  await fs.rm(data, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 });
 }
});

test('workspace layout transitions always reveal and focus the requested editor', {timeout:180000}, async () => {
 const data = await fs.mkdtemp(path.join(os.tmpdir(), 'refract-view-transitions-'));
 const env: NodeJS.ProcessEnv = {...process.env, REFRACT_DATA_DIR:data, REFRACT_TEST_HIDDEN:'1'};
 delete env.ELECTRON_RUN_AS_NODE;
 const app = await launchTestDesktop({args:[path.resolve(__dirname, '../..')], env});
 try {
  await useFakeCodex(app);
  const page = await app.firstWindow();
  page.setDefaultTimeout(10000);
  // Hidden windows otherwise throttle animation frames during rapid layout changes.
  await app.evaluate(({BrowserWindow}) => BrowserWindow.getAllWindows()[0].webContents.setBackgroundThrottling(false));
  const errors: string[] = []; page.on('pageerror', error => errors.push(error.message));
  await page.locator('#new').click();
  await page.locator('#source').waitFor({state:'visible'});
  const focusEditor = () => chooseView(page,'editor');
  const verifyEditor = async (): Promise<void> => {
   assert.equal(await page.locator('#source').isVisible(), true);
   assert.equal(await page.locator('#pane-editor').evaluate(el => el === document.activeElement), true);
   assert.equal(await page.locator('#workspace').evaluate(el => el.classList.contains('show-compact-preview') || el.classList.contains('tools-expanded')), false);
  };
  for (const colorScheme of ['light','dark'] as const) {
   await page.emulateMedia({colorScheme});
   for (const width of [1440,1000]) {
    await resizeDesktop(app, page, width, 900);
    const split = () => chooseView(page, width <= 1200 ? 'editor' : 'split');
    await chooseView(page,'pdf');
    assert.equal(await page.locator('#source').isVisible(), false);
    await focusEditor(); await verifyEditor();
    assert.equal(await page.locator('.pdf-pane').isVisible(), false);
    await split();
    await page.locator('#tools').click();
    await page.getByRole('button', {name:'Expand tools panel', exact:true}).click();
    assert.equal(await page.locator('#source').isVisible(), false);
    await focusEditor(); await verifyEditor();
    assert.equal(await page.locator('.expand-tools').getAttribute('aria-label'), 'Expand tools panel');
    await split();
    await page.getByRole('button', {name:'Expand chat', exact:true}).click();
    await focusEditor(); await verifyEditor();
    assert.equal(await page.getByRole('button', {name:'Expand chat', exact:true}).getAttribute('aria-pressed'), 'false');
    await split();
    await chooseView(page,'pdf');
    await page.locator('#pane-editor').click();
    await verifyEditor();
   }
  }
  assert.deepEqual(errors, []);
 } finally {
  await app.close();
  await fs.rm(data, {recursive:true,force:true,maxRetries:5,retryDelay:200});
 }
});
