import test = require('node:test');
import assert = require('node:assert/strict');
import fixture = require('./fixtures/headless-ui.cjs');
import type { Page } from 'playwright';

type FixtureWindow = Window & {
  store: Record<string, (...args: unknown[]) => Promise<unknown>>;
  fixture: { delay: number; counters: Record<string, number> };
};

async function openSampleProject(page: Page): Promise<void> {
  await page.getByRole('button', { name: 'Open Spectral methods for fluid dynamics', exact: true }).press('Enter');
  await page.getByText('Compiled', { exact: true }).waitFor();
}

test('project creation is single-flight and the library restores navigation focus and view', { timeout: 30_000 }, async () => {
  const { browser, page, errors } = await fixture.openHeadlessUi();
  try {
    await page.locator('#grid-view').click();
    await page.reload();
    assert.equal(await page.locator('#grid-view').getAttribute('aria-pressed'), 'true');
    await openSampleProject(page);
    assert.equal(await page.locator('#project-name').evaluate(el => el === document.activeElement), true);
    await page.locator('#home').press('Enter');
    const previous = page.getByRole('button', { name: 'Open Spectral methods for fluid dynamics', exact: true });
    await page.waitForFunction(() => document.activeElement?.getAttribute('aria-label') === 'Open Spectral methods for fluid dynamics');
    assert.equal(await previous.isVisible(), true);
    await page.evaluate(() => { (window as unknown as FixtureWindow).fixture.delay = 200; });
    await page.locator('#new').evaluate((button: HTMLButtonElement) => { button.click(); button.click(); });
    await page.getByRole('button', { name: 'New Project', exact: true }).waitFor();
    assert.equal(await page.evaluate(() => (window as unknown as FixtureWindow).fixture.counters.create), 1);
    await page.locator('#home').click();
    await page.locator('#new').click();
    await previous.click();
    await page.waitForFunction(() => !(document.querySelector('#new') as HTMLButtonElement).disabled);
    assert.equal(await page.locator('#project-name').textContent(), 'Spectral methods for fluid dynamics');
    await page.locator('#home').click();
    await page.locator('#new').click();
    await page.getByRole('button', {name:'Settings', exact:true}).click();
    await page.waitForFunction(() => (window as unknown as FixtureWindow).fixture.counters.create === 3);
    assert.equal(await page.locator('#workspace').isHidden(), true);
    await page.getByRole('button', {name:'Close', exact:true}).click();
    await page.getByRole('button', {name:'New Project', exact:true}).waitFor();
    assert.equal(await page.locator('#project-name').evaluate(el => el === document.activeElement), true);
    assert.deepEqual(errors, []);
  } finally { await browser.close(); }
});

test('compact file, search, outline, and comment destinations reveal source and preserve writing panes', { timeout: 30_000 }, async () => {
  const { browser, page, errors } = await fixture.openHeadlessUi();
  try {
    await openSampleProject(page);
    await page.setViewportSize({ width: 900, height: 650 });
    const editor = page.locator('#source');
    const preview = page.locator('#pane-preview');
    await preview.click();
    await page.getByRole('button', { name: 'Open sections/abstract.tex', exact: true }).press('Enter');
    await page.waitForFunction(() => document.activeElement?.id === 'source');
    assert.match(await editor.inputValue(), /stable spectral/);
    assert.equal(await page.locator('#chat').isVisible(), true);

    await page.evaluate(() => {
      (window as unknown as FixtureWindow).store.search = async () => [
        { file: 'main.tex', line: 8, column: 9, text: '\\section{Introduction}' }
      ];
    });
    await preview.click();
    await page.getByLabel('Search project', { exact: true }).fill('Introduction');
    await page.getByRole('button', { name: /main.tex:8/ }).click();
    assert.equal(await editor.evaluate((el: HTMLTextAreaElement) => el.value.slice(el.selectionStart, el.selectionEnd)), 'Introduction');
    await page.locator('#outline-list button').first().click();
    assert.equal(await editor.isVisible(), true);
    await page.locator('#tools').click();
    await page.locator('#tool-comments').click();
    await page.locator('.comment-reference').first().click();
    await page.waitForFunction(() => document.activeElement?.id === 'source');
    assert.equal(await page.locator('#tools-panel').isHidden(), true);
    await page.evaluate(() => {
      const {store} = window as unknown as FixtureWindow;
      store.commentLocate = async () => null;
      store.versionList = async () => [{ id:'version-1', created:'2026-01-01', files:['main.tex'] }];
    });
    await page.locator('#tools').click();
    await page.locator('.comment-reference').first().click();
    await page.getByText('The comment’s original selection is unavailable in this file.').waitFor();
    assert.equal(await editor.isVisible(), true);
    await page.setViewportSize({ width:1440, height:900 });
    await page.locator('#tools').click();
    await page.locator('#tool-versions').click();
    const restore = page.getByRole('button', { name:'Restore file', exact:true });
    await restore.waitFor();
    assert.equal(await restore.isEnabled(), true);
    await page.getByRole('button', { name:'Open sections/abstract.tex', exact:true }).click();
    await page.waitForFunction(() => (document.querySelector('[data-restore-version-file]') as HTMLButtonElement).disabled);
    assert.deepEqual(errors, []);
  } finally { await browser.close(); }
});

test('file actions keep invalid renames open and restore focus on cancel', { timeout: 30_000 }, async () => {
  const { browser, page, errors } = await fixture.openHeadlessUi('dark');
  try {
    page.setDefaultTimeout(7000);
    await openSampleProject(page);
    const actions = page.getByRole('button', { name: 'Actions for main.tex', exact: true });
    await actions.press('Enter');
    await page.getByRole('button', { name: 'Rename file', exact: true }).press('Enter');
    const input = page.getByRole('dialog', { name: 'Rename file' }).getByLabel('File name', { exact: true });
    await input.fill('sections/main.tex');
    await input.press('Enter');
    assert.equal(await page.getByRole('dialog', { name: 'Rename file' }).isVisible(), true);
    assert.equal(await input.evaluate((el: HTMLInputElement) => el.validationMessage), 'Enter a file name without a folder path.');
    await input.press('Escape');
    await page.waitForFunction(() => document.activeElement?.getAttribute('aria-label') === 'Actions for main.tex', undefined, {timeout:7000});
    await actions.press('Enter');
    await page.getByRole('button', { name: 'Rename file', exact: true }).press('Enter');
    await input.fill('.hidden.tex');
    await input.press('Enter');
    assert.match(await page.locator('#rename-file-error').textContent(), /other than a dot/);
    await input.fill('paper.tex');
    await input.press('Enter');
    await page.waitForFunction(() => document.activeElement?.getAttribute('aria-label') === 'Actions for paper.tex', undefined, {timeout:7000});
    assert.deepEqual(errors, []);
  } finally { await browser.close(); }
});

test('compact build diagnostics open the reported include rather than the current file', { timeout: 30_000 }, async () => {
  const { browser, page, errors } = await fixture.openHeadlessUi();
  try {
    await openSampleProject(page);
    await page.setViewportSize({ width: 1000, height: 740 });
    await page.evaluate(() => {
      (window as unknown as FixtureWindow).store.compile = async () => ({
        ok: false,
        log: './sections/abstract.tex:2: Undefined control sequence.\nl.2'
      });
    });
    await page.locator('#pane-preview').click();
    await page.getByRole('button', { name: 'Compiled', exact: true }).click();
    await page.getByText('Showing the last successful PDF.', { exact: false }).waitFor();
    assert.equal(await page.locator('#compile').evaluate(el => el === document.activeElement), true);
    assert.equal(await page.locator('.pdf-pages canvas').isVisible(), true);
    assert.equal(await page.locator('.pdf-pages canvas').getAttribute('title'), '');
    await page.getByRole('button', { name: 'Go to line 2 in sections/abstract.tex' }).click();
    const editor = page.locator('#source');
    await editor.waitFor();
    assert.equal(await editor.evaluate((el: HTMLTextAreaElement) => el.value.slice(el.selectionStart, el.selectionEnd)), 'A stable spectral approximation.');
    assert.deepEqual(errors, []);
  } finally { await browser.close(); }
});

test('an update check failure survives status polls and clears on retry', { timeout: 30_000 }, async () => {
  const { browser, page, errors } = await fixture.openHeadlessUi();
  try {
    await page.evaluate(() => {
      const {store} = window as unknown as FixtureWindow;
      let attempts = 0;
      store.updateCheck = async () => {
        if (++attempts === 1) throw new Error('offline');
        return store.updateStatus();
      };
    });
    await page.getByRole('button', { name: 'Settings', exact: true }).click();
    const check = page.getByRole('button', { name: 'Check for updates' });
    await check.click();
    const error = page.getByRole('alert').filter({ hasText: 'Could not check for updates' });
    await error.waitFor();
    await page.getByText('You are up to date. Checking every 10 minutes.').waitFor();
    assert.ok(await error.isVisible());
    await check.click();
    await error.waitFor({ state: 'hidden' });
    await page.evaluate(() => {
      const {store} = window as unknown as FixtureWindow;
      const rejected = async () => {
        await new Promise(resolve => setTimeout(resolve, 200));
        throw new Error('offline');
      };
      store.updateTrack = rejected;
      store.updateRestart = rejected;
      store.updateStatus = async () => ({ track:'stable', phase:'ready', message:'An update is ready.' });
    });
    const track = page.getByLabel('Update track', { exact:true });
    await track.press('End');
    await page.getByText('Could not save the update track. Please try again.').waitFor();
    await page.waitForFunction(() => document.activeElement?.id === 'update-track');
    const restart = page.getByRole('button', { name:'Restart to update' });
    await restart.press('Enter');
    await page.getByText('Could not restart to update. Please try again.').waitFor();
    assert.equal(await restart.evaluate(el => el === document.activeElement), true);
    assert.deepEqual(errors, []);
  } finally { await browser.close(); }
});

test('build refresh preserves output filters and presents the previous PDF while pending', { timeout: 30_000 }, async () => {
  const { browser, page, errors } = await fixture.openHeadlessUi();
  try {
    await openSampleProject(page);
    const pdfText = page.locator('.pdf-accessible-text');
    await pdfText.waitFor({ state: 'attached' });
    assert.match(await pdfText.textContent(), /Spectral methods/);
    await page.evaluate(() => {
      const { store } = window as unknown as FixtureWindow;
      const compile = store.compile;
      store.compile = async (...args) => {
        await new Promise(resolve => setTimeout(resolve, 450));
        return compile(...args);
      };
    });
    await page.locator('#source').press('End');
    await page.locator('#source').press('Space');
    await page.getByText('Showing the last successful build while changes compile.').waitFor();
    assert.equal(await page.locator('.pdf-pages canvas').isVisible(), true);
    assert.equal(await page.locator('#download').isDisabled(), true);
    await page.getByText('Compiled', { exact: true }).waitFor();
    await page.locator('#tools').click();
    await page.locator('#tool-info').click();
    await page.locator('#draft-mode').check();
    await page.locator('#tool-logs').click();
    await page.getByText('Full build output', { exact: true }).click();
    await page.getByRole('button', { name: 'Disable line wrap' }).click();
    const filter = page.getByRole('searchbox', { name: 'Filter build output' });
    await filter.fill('Output written');
    await page.getByText('1 matching line', { exact: true }).waitFor();
    assert.equal(await filter.evaluate(el => el === document.activeElement), true);
    assert.equal(await page.getByRole('button', { name: 'Enable line wrap' }).isVisible(), true);
    await page.locator('#tool-info').click();
    await page.locator('#tool-logs').click();
    assert.equal(await filter.inputValue(), 'Output written');
    assert.equal(await filter.isVisible(), true);
    assert.deepEqual(errors, []);
  } finally { await browser.close(); }
});

test('a keyboard stop returns to the composer and a reply is announced', { timeout: 30_000 }, async () => {
  const { browser, page, errors } = await fixture.openHeadlessUi();
  try {
    await page.evaluate(() => {
      const {store, fixture:state} = window as unknown as FixtureWindow;
      const originalSend = store.chatSend;
      let complete: () => void;
      store.chatSend = async () => {
        state.counters.chatSendStarted = 1;
        await new Promise<void>(resolve => { complete = resolve; });
        return originalSend();
      };
      store.chatCancel = async () => { setTimeout(complete, 100); };
    });
    await openSampleProject(page);
    const composer = page.locator('#chat-input');
    assert.equal(await page.locator('#send').isDisabled(), true);
    await composer.fill('My question');
    await page.locator('#send').click();
    await page.waitForFunction(() => (window as unknown as FixtureWindow).fixture.counters.chatSendStarted === 1);
    const stop = page.getByRole('button', { name:'Stop', exact:true });
    await stop.press('Enter');
    assert.equal(await page.getByRole('button', { name:'Stopping…' }).evaluate(el => el === document.activeElement), true);
    await page.waitForFunction(() => document.activeElement?.id === 'chat-input');
    await page.getByText('Assistant response ready.', {exact:true}).waitFor({state:'attached'});
    assert.deepEqual(errors, []);
  } finally { await browser.close(); }
});

test('stopping during preparation preserves the unsent prompt and the new draft', { timeout: 30_000 }, async () => {
  const { browser, page, errors } = await fixture.openHeadlessUi();
  try {
    await page.evaluate(() => {
      const w = window as unknown as FixtureWindow & {codexPickerReady():Promise<void>; finishPreparation():void};
      w.codexPickerReady = () => new Promise<void>(resolve => { w.finishPreparation = resolve; });
    });
    await openSampleProject(page);
    const composer = page.locator('#chat-input');
    await composer.fill('First prompt');
    await page.locator('#send').click();
    await composer.fill('Second draft');
    await page.getByRole('button', {name:'Stop', exact:true}).press('Enter');
    await page.evaluate(() => (window as unknown as {finishPreparation():void}).finishPreparation());
    await page.getByText('Message stopped before sending.', {exact:false}).waitFor({state:'attached'});
    assert.equal(await composer.inputValue(), 'First prompt\n\nSecond draft');
    assert.equal(await page.evaluate(() => (window as unknown as FixtureWindow).fixture.counters.chatSend ?? 0), 0);
    assert.deepEqual(errors, []);
  } finally { await browser.close(); }
});

test('settings shows download progress while a manual check is pending', { timeout: 30_000 }, async () => {
  const { browser, page, errors } = await fixture.openHeadlessUi();
  try {
    await page.evaluate(() => {
      const {store} = window as unknown as FixtureWindow;
      let downloading = false;
      store.updateStatus = async () => downloading
        ? { track:'stable', phase:'downloading', message:'Downloading update', downloadPercent:36 }
        : { track:'stable', phase:'idle', message:'Up to date' };
      store.updateCheck = async () => {
        downloading = true;
        await new Promise(resolve => setTimeout(resolve, 3500));
        return store.updateStatus();
      };
    });
    await page.getByRole('button', {name:'Settings', exact:true}).click();
    await page.getByText('Up to date', {exact:true}).waitFor();
    const check = page.getByRole('button', {name:'Check for updates'});
    await check.click();
    await page.getByText('36%', {exact:true}).waitFor();
    assert.equal(await check.isDisabled(), true);
    assert.deepEqual(errors, []);
  } finally { await browser.close(); }
});

test('track changes show download progress without replacing the selected track', { timeout: 30_000 }, async () => {
  const { browser, page, errors } = await fixture.openHeadlessUi();
  try {
    await page.evaluate(() => {
      const w = window as unknown as FixtureWindow & {finishTrack():void};
      let checking = false;
      w.store.updateStatus = async () => checking
        ? { track:'stable', phase:'downloading', message:'Downloading update', downloadPercent:36 }
        : { track:'stable', phase:'idle', message:'Up to date' };
      w.store.updateTrack = async () => {
        checking = true;
        await new Promise<void>(resolve => { w.finishTrack = resolve; });
        return {track:'beta', phase:'idle', message:'Up to date'};
      };
    });
    await page.getByRole('button', {name:'Settings', exact:true}).click();
    await page.getByText('Up to date', {exact:true}).waitFor();
    const track = page.getByLabel('Update track', {exact:true});
    await track.selectOption('beta');
    await page.getByText('36%', {exact:true}).waitFor();
    assert.equal(await track.isDisabled(), true);
    assert.equal(await track.inputValue(), 'beta');
    await page.evaluate(() => (window as unknown as {finishTrack():void}).finishTrack());
    assert.deepEqual(errors, []);
  } finally { await browser.close(); }
});

test('slow tab closing and version restoration respect newer navigation and focus', { timeout: 30_000 }, async () => {
  const { browser, page, errors } = await fixture.openHeadlessUi();
  try {
    await openSampleProject(page);
    const otherFile = page.getByRole('button', {name:'Open sections/abstract.tex', exact:true});
    await otherFile.click();
    await page.evaluate(() => {
      const w = window as unknown as FixtureWindow & {finishFile():void; finishOtherFile():void; finishRestore():void};
      const open = w.store.openFile;
      w.store.openFile = async (...args) => {
        if (args[1] === 'main.tex') await new Promise<void>(resolve => { w.finishFile = resolve; });
        if (args[1] === 'sections/abstract.tex') await new Promise<void>(resolve => { w.finishOtherFile = resolve; });
        return open(...args);
      };
      w.store.versionRestoreFile = () => new Promise<void>(resolve => { w.finishRestore = resolve; });
      const list = w.store.versionList;
      w.store.versionList = async (...args) => {
        w.fixture.counters.versionList = (w.fixture.counters.versionList ?? 0) + 1;
        return list(...args);
      };
    });
    await page.getByRole('button', {name:'Close sections/abstract.tex', exact:true}).press('Enter');
    const composer = page.locator('#chat-input');
    await composer.focus();
    await page.evaluate(() => (window as unknown as {finishFile():void}).finishFile());
    await page.waitForFunction(() => document.querySelector('#file-tabs [title="main.tex"]')?.getAttribute('aria-current') === 'page');
    assert.equal(await composer.evaluate(el => el === document.activeElement), true);

    await page.locator('#tools').click();
    await page.locator('#tool-versions').click();
    await page.getByRole('button', {name:'Restore file', exact:true}).press('Enter');
    await page.locator('#app-dialog-action').click();
    await otherFile.click();
    await page.evaluate(() => (window as unknown as {finishRestore():void}).finishRestore());
    await page.evaluate(() => (window as unknown as {finishOtherFile():void}).finishOtherFile());
    await page.waitForFunction(() => document.querySelector('#file-tabs [title="sections/abstract.tex"]')?.getAttribute('aria-current') === 'page');
    await page.waitForFunction(() => (window as unknown as FixtureWindow).fixture.counters.versionList === 2);
    assert.match(await page.locator('#source').inputValue(), /A stable spectral approximation/);
    assert.equal(await otherFile.evaluate(el => el === document.activeElement), true);
    assert.deepEqual(errors, []);
  } finally { await browser.close(); }
});

test('moving a folder reveals a collapsed destination and focuses the moved folder', { timeout: 30_000 }, async () => {
  const { browser, page, errors } = await fixture.openHeadlessUi();
  try {
    await page.evaluate(() => {
      const {store, fixture:state} = window as unknown as FixtureWindow & {fixture:{files:Record<string,string>}};
      state.files['archive/existing.tex'] = 'Archive notes';
      const move = store.moveFolder;
      const tree = store.tree;
      let moveApplied = false;
      let responseReleased = false;
      store.moveFolder = async (...args) => {
        const project = await move(...args);
        moveApplied = true;
        await new Promise(resolve => setTimeout(resolve, 3200));
        responseReleased = true;
        return project;
      };
      store.tree = async (...args) => {
        const snapshot = await tree(...args);
        if (moveApplied && responseReleased) {
          responseReleased = false;
          await new Promise(resolve => setTimeout(resolve, 3200));
        }
        return snapshot;
      };
    });
    await openSampleProject(page);
    await page.locator('summary[title="archive"]').click();
    await page.getByRole('button', {name:'Actions for sections', exact:true}).press('Enter');
    await page.getByRole('button', {name:'Move or rename folder', exact:true}).press('Enter');
    const input = page.getByLabel('New folder path');
    await input.fill('archive/sections');
    await input.press('Enter');
    await page.waitForFunction(() => document.activeElement?.getAttribute('aria-label') === 'Actions for archive/sections');
    assert.equal(await page.locator('details[data-path="archive"]').getAttribute('open'), '');
    assert.deepEqual(errors, []);
  } finally { await browser.close(); }
});

test('moving a folder respects a file opened during its tree refresh', { timeout: 30_000 }, async () => {
  const { browser, page, errors } = await fixture.openHeadlessUi();
  try {
    await page.evaluate(() => {
      const w = window as unknown as FixtureWindow & {fixture:{files:Record<string,string>}; finishTree():void};
      w.fixture.files['archive/existing.tex'] = 'Archive notes';
      const move = w.store.moveFolder;
      const tree = w.store.tree;
      let moved = false;
      w.store.moveFolder = async (...args) => {
        const result = await move(...args);
        moved = true;
        return result;
      };
      w.store.tree = async (...args) => {
        const result = await tree(...args);
        if (moved) {
          moved = false;
          await new Promise<void>(resolve => { w.finishTree = resolve; });
        }
        return result;
      };
    });
    await openSampleProject(page);
    await page.getByRole('button', {name:'Actions for sections', exact:true}).press('Enter');
    await page.getByRole('button', {name:'Move or rename folder', exact:true}).press('Enter');
    const input = page.getByLabel('New folder path');
    await input.fill('archive/sections');
    await input.press('Enter');
    await input.waitFor({state:'hidden'});
    const otherFile = page.getByRole('button', {name:'Open archive/existing.tex', exact:true});
    await otherFile.click();
    const editor = page.locator('#source');
    await page.waitForFunction(() => (document.querySelector('#source') as HTMLTextAreaElement).value === 'Archive notes');
    await page.evaluate(() => (window as unknown as {finishTree():void}).finishTree());
    await page.locator('details[data-path="archive/sections"]').waitFor();
    assert.equal(await editor.inputValue(), 'Archive notes');
    assert.equal(await otherFile.evaluate(el => el === document.activeElement), true);
    assert.deepEqual(errors, []);
  } finally { await browser.close(); }
});
