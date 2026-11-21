import nodeTest = require('node:test');
import type {TestContext} from 'node:test';
import assert = require('node:assert/strict');
import path = require('node:path');
import playwright = require('playwright');
import type {Page} from 'playwright';
import type {CodexPreferences} from '../src/codex-preferences.cjs';

const {test} = nodeTest;
const {chromium} = playwright;

type ModelSelection = Omit<CodexPreferences, 'favorites'>;
type Mode = 'signed-out' | 'normal' | 'empty' | 'slow' | 'fail-load' | 'fail-discovery' | 'fail-save' | 'no-opencode' | 'provider-check-failed' | 'offline-duplicates' | 'empty-opencode' | 'no-low-default' | 'unknown-default' | 'empty-codex' | 'failed-codex';
interface PickerFixture extends Window {
  codexPickerReady(): Promise<ModelSelection>;
  ready?: Promise<ModelSelection>;
  releasePreferences(): void;
  releaseSave?(): void;
  releaseCatalog?(catalog: unknown): void;
  failLoad: boolean;
  failDiscovery: boolean;
  failSave: boolean;
  holdSave: boolean;
  saves: {provider: string; value: CodexPreferences}[];
  store: {
    discoverOpenCodeModels(project?: string): Promise<unknown>;
    codexPreferencesLoad(mode?: 'cli' | 'account'): Promise<CodexPreferences>;
    codexPreferencesSave(value: CodexPreferences, mode?: 'cli' | 'account'): Promise<void>;
  };
}

const assets = path.resolve(__dirname, '..');
const root = 'https://refract-picker.test';
const markup = `<!doctype html><meta charset="utf-8"><link rel="stylesheet" href="/style.css">
<div class="chat" style="position:fixed;inset:auto 40px 30px 40px;max-height:none">
  <div id="chat-messages"></div>
  <div class="composer"><div><span></span><label class="provider-label" for="chat-provider">Provider</label>
    <select id="chat-provider"><option value="codex">Codex</option><option value="opencode">OpenCode</option></select>
    <div id="model-picker"></div><button id="outside" type="button">Send</button></div>
  </div>
</div><script src="/model-picker.js"></script>`;

async function picker(t: TestContext, mode: Mode = 'normal', selectCodex = true): Promise<Page> {
  const browser = await chromium.launch({channel: 'msedge', headless: true});
  t.after(() => browser.close());
  const page = await browser.newPage({viewport: {width: 1200, height: 800}, colorScheme: mode === 'signed-out' ? 'dark' : 'light'});
  page.setDefaultTimeout(5000);
  await page.route(`${root}/**`, async route => {
    const asset = new URL(route.request().url()).pathname;
    if (asset === '/' && (mode === 'provider-check-failed' || mode === 'offline-duplicates')) {
      const unavailable = markup.replace(/<select id="chat-provider">[^<]*(?:<option[^>]*>[^<]*<\/option>)*<\/select>/,
        '<select id="chat-provider" data-picker-status="error"></select>');
      await route.fulfill({contentType: 'text/html', body: unavailable});
    }
    else if (asset === '/' && mode === 'signed-out') await route.fulfill({
      contentType: 'text/html',
      body: markup.replace('<select id="chat-provider">', '<select id="chat-provider" data-codex-sign-in-required="true">')
        .replace('<option value="codex">', '<option value="codex" disabled>')
        .replace('<option value="opencode">', '<option value="opencode" disabled>')
    });
    else if (asset === '/') await route.fulfill({contentType: 'text/html', body: mode === 'no-opencode' ? markup.replace('<option value="opencode">', '<option value="opencode" disabled>') : markup});
    else await route.fulfill({path: path.join(assets, asset)});
  });
  await page.addInitScript((mode) => {
    const w = window as unknown as PickerFixture;
    w.saves = [];
    w.failLoad = mode === 'fail-load';
    w.failDiscovery = mode === 'fail-discovery';
    w.failSave = mode === 'fail-save';
    const model = (id: string, name: string, efforts: string[]) => ({
      id, model: id, displayName: name, description: 'General research tasks',
      supportedReasoningEfforts: efforts.map(effort => ({effort, description: ''})),
      defaultReasoningEffort: efforts[0] ?? null, isDefault: false
    });
    const initial = () => ({model: null, reasoningEffort: null, favorites: []});
    const save = async (provider: string, value: CodexPreferences) => {
      w.saves.push({provider, value});
      if (w.holdSave) await new Promise<void>(resolve => w.releaseSave = resolve);
      if (w.failSave) throw new Error('Could not write preferences');
    };
    const catalog = [
      {id: 'opencode/gpt-6.2', name: 'GPT-6.2', variants: ['fast', 'balanced', 'deep']},
      {id: 'opencode/effort-model', name: 'Effort model', variants: ['high', 'low']},
      {id: 'lmstudio/model-10', name: 'Model 10', variants: ['high', 'max']},
      {id: 'lmstudio/model-2', name: 'Model 2', variants: []}
    ];
    const codexCatalog = [
      {...model('gpt-6.2', 'GPT-6.2', mode === 'no-low-default' ? ['high', 'medium'] : ['low', 'high']), isDefault: mode !== 'unknown-default'},
      model('research', 'Research', [])
    ];
    if (mode === 'empty' || mode === 'unknown-default') codexCatalog.reverse();
    w.store = Object.assign({
      discoverCodexModels: async () => mode === 'failed-codex' ? {ok: false, error: 'Codex catalog is offline'} :
        {ok: true, models: mode === 'empty-codex' ? [] : codexCatalog},
      discoverOpenCodeModels: async () => {
        if (w.failDiscovery) throw new Error('OpenCode catalog is offline');
        if (mode === 'empty-opencode') return {ok: true, models: []};
        return {ok: true, models: catalog};
      },
      codexPreferencesLoad: async () => {
        if (w.failLoad) throw new Error('Preferences are unavailable');
        if (mode === 'slow') await new Promise<void>(resolve => w.releasePreferences = resolve);
        if (mode === 'unknown-default') return {...initial(), reasoningEffort: 'high'};
        if (['empty', 'no-low-default', 'empty-codex', 'failed-codex'].includes(mode)) return initial();
        return {model: 'gpt-6.2', reasoningEffort: 'low', favorites: ['gpt-6.2']};
      },
      openCodePreferencesLoad: async () => mode === 'empty-opencode' ? initial() : {
        model: 'opencode/gpt-6.2',
        reasoningEffort: 'balanced',
        favorites: mode === 'offline-duplicates'
          ? ['lmstudio/vendor-a/shared', 'lmstudio/vendor-b/shared'] : ['opencode/gpt-6.2', 'lmstudio/legacy']
      },
      codexPreferencesSave: (value: CodexPreferences) => save('codex', value),
      openCodePreferencesSave: (value: CodexPreferences) => save('opencode', value)
    });
  }, mode);
  await page.goto(root);
  if (mode !== 'signed-out') await page.locator('.model-trigger').click();
  if (selectCodex) await page.getByRole('button', {name: 'Codex models'}).click();
  return page;
}

const providerRows = (page: Page, name: string) => page.getByRole('region', {name, exact: true});
const trigger = (page: Page) => page.locator('.model-trigger');
const choice = (page: Page, name: string) => page.getByRole('button', {name, exact: true});
const search = (page: Page) => page.getByRole('searchbox', {name: 'Search models'});
const reasoning = (page: Page, name: string) => page.getByRole('radiogroup', {name: 'Reasoning effort'}).getByRole('radio', {name, exact: true});
const activeProvider = (page: Page) => page.locator('#chat-provider').inputValue();

test('browsing, cross-provider search and Escape preserve the send target', async t => {
  const page = await picker(t);
  assert.equal(await choice(page, 'Codex models').getAttribute('aria-pressed'), 'true');
  await choice(page, 'OpenCode models').click();
  assert.equal(await activeProvider(page), 'codex');
  await page.keyboard.press('Escape');
  assert.equal(await trigger(page).getAttribute('aria-expanded'), 'false');
  assert.ok(await trigger(page).evaluate(el => document.activeElement === el));
  await trigger(page).click();
  assert.equal(await choice(page, 'Favorite models').getAttribute('aria-pressed'), 'true');
  assert.equal(await choice(page, 'All models').count(), 0);
  assert.equal(await trigger(page).locator('.model-chevron').count(), 0);
  assert.match(await choice(page, 'Favorite models').locator('.model-filter-star').evaluate(el => getComputedStyle(el).maskImage), /favorite-star\.svg/);
  await search(page).fill('6.2');
  assert.equal(await page.getByRole('button', {name: 'GPT 6.2', exact: true}).count(), 2);
  await search(page).fill('gpt-6.2');
  assert.equal(await page.getByRole('button', {name: 'GPT 6.2', exact: true}).count(), 2);
  await search(page).press('Enter');
  assert.equal(await activeProvider(page), 'codex');
  assert.equal(await page.locator('button:focus').getAttribute('aria-label'), 'GPT 6.2');
  await search(page).fill('10 studio');
  assert.match(await choice(page, 'Model 10').getAttribute('title') ?? '', /LM Studio/);
  await search(page).press('Enter');
  assert.equal(await activeProvider(page), 'opencode');
  assert.equal(await page.getByRole('radiogroup', {name: 'Model variant'}).getByRole('radio', {name: 'High', exact: true}).getAttribute('aria-checked'), 'true');
  assert.match(await trigger(page).getAttribute('aria-label') ?? '', /OpenCode.*Model 10.*High/);
  assert.equal(await page.locator('.model-panel').isVisible(), true);
  await search(page).fill('effort model');
  await search(page).press('Enter');
  assert.deepEqual(await page.evaluate(() => (window as unknown as PickerFixture).codexPickerReady()),
    {model: 'opencode/effort-model', reasoningEffort: 'low'});
  await page.locator('#outside').click();
  await page.locator('.model-panel').waitFor({state:'hidden'});
  assert.ok(await page.locator('#outside').evaluate(el => document.activeElement === el));
});

test('favorites span providers, distinguish duplicate models, and remove stale entries', async t => {
  const page = await picker(t);
  await choice(page, 'Favorite models').click();
  assert.equal(await page.getByRole('button', {name: 'GPT 6.2', exact: true}).count(), 2);
  const legacy = providerRows(page, 'OpenCode').getByRole('button', {name: 'legacy', exact: true});
  assert.ok(await legacy.isDisabled());
  const legacyStar = choice(page, 'Remove legacy from favorites');
  await legacyStar.focus();
  await page.keyboard.press('Enter');
  const other = providerRows(page, 'OpenCode').getByRole('button', {name: 'Remove GPT 6.2 from favorites'});
  assert.ok(await other.evaluate(el => el === document.activeElement));
  assert.equal(await legacy.count(), 0);
  await providerRows(page, 'OpenCode').getByRole('button', {name: 'Remove GPT 6.2 from favorites'}).click();
  assert.equal(await providerRows(page, 'Codex').getByRole('button', {name: 'GPT 6.2', exact: true}).count(), 1);
  await providerRows(page, 'Codex').getByRole('button', {name: 'Remove GPT 6.2 from favorites'}).click();
  assert.ok(await page.getByText('No favorites yet', {exact: true}).filter({visible: true}).count());
  assert.deepEqual((await page.evaluate(() => (window as unknown as PickerFixture).saves)).map(entry => [entry.provider, entry.value.favorites]), [
    ['opencode', ['opencode/gpt-6.2']], ['opencode', []], ['codex', []]
  ]);
});

test('keyboard navigation keeps model choices and favorite actions distinct', async t => {
  const page = await picker(t);
  await choice(page, 'Codex models').click();
  await choice(page, 'Research').waitFor();
  await search(page).fill('does not exist');
  await search(page).press('Enter');
  assert.match(await trigger(page).getAttribute('aria-label') ?? '', /GPT 6.2/);
  await choice(page, 'Clear search').click();
  await search(page).press('ArrowDown');
  assert.equal(await page.locator('button:focus').getAttribute('aria-label'), 'GPT 6.2');
  await page.keyboard.press('ArrowRight');
  assert.equal(await page.locator('button:focus').getAttribute('aria-label'), 'Remove GPT 6.2 from favorites');
  await page.keyboard.press('Enter');
  assert.equal(await page.locator('button:focus').getAttribute('aria-label'), 'Add GPT 6.2 to favorites');
  await page.keyboard.press('ArrowLeft');
  await page.keyboard.press('End');
  assert.equal(await page.locator('button:focus').getAttribute('aria-label'), 'Research');
  await page.keyboard.press('Escape');
  assert.ok(await trigger(page).evaluate(el => document.activeElement === el));
});

test('Codex defaults to Standard and Low while retaining explicit choices for a selected model', async t => {
  const page = await picker(t, 'empty');
  const ready = () => page.evaluate(() => (window as unknown as PickerFixture).codexPickerReady());
  assert.deepEqual(await ready(), {model: 'gpt-6.2', reasoningEffort: 'low', fastMode: false});
  assert.equal(await choice(page, 'Standard').getAttribute('aria-pressed'), 'true');
  assert.equal(await choice(page, 'Inherit').count(), 0);
  await choice(page, 'GPT 6.2').click();
  assert.equal(await reasoning(page, 'Low').getAttribute('aria-checked'), 'true');
  assert.equal(await reasoning(page, 'Low').getAttribute('aria-description'), 'Default reasoning effort');
  assert.equal(await reasoning(page, 'Use model default').count(), 0);
  const high = reasoning(page, 'High');
  await high.click();
  assert.ok(await high.evaluate(el => document.activeElement === el));
  await choice(page, 'Fast').click();
  await choice(page, 'GPT 6.2').click();
  assert.deepEqual(await ready(), {model: 'gpt-6.2', reasoningEffort: 'high', fastMode: true});
  await choice(page, 'Research').click();
  assert.deepEqual(await ready(), {model: 'research', reasoningEffort: null, fastMode: true});
  await choice(page, 'GPT 6.2').click();
  assert.equal(await reasoning(page, 'Low').getAttribute('aria-checked'), 'true');
  await choice(page, 'Standard').click();
  assert.deepEqual(await ready(), {model: 'gpt-6.2', reasoningEffort: 'low', fastMode: false});
});

test('Codex replaces legacy unset model preferences with a catalog model', async t => {
  await t.test('the default model does not support Low', async t => {
    const page = await picker(t, 'no-low-default');
    const ready = await page.evaluate(() => (window as unknown as PickerFixture).codexPickerReady());
    assert.deepEqual(ready, {model: 'gpt-6.2', reasoningEffort: 'high', fastMode: false});
    assert.equal(await reasoning(page, 'High').getAttribute('aria-checked'), 'true');
    assert.equal(await reasoning(page, 'Low').count(), 0);
  });
  await t.test('the default model is unknown', async t => {
    const page = await picker(t, 'unknown-default');
    const ready = await page.evaluate(() => (window as unknown as PickerFixture).codexPickerReady());
    assert.deepEqual(ready, {model: 'research', reasoningEffort: null, fastMode: false});
    assert.equal(await page.getByRole('radiogroup', {name: 'Reasoning effort'}).count(), 0);
    assert.deepEqual(await page.evaluate(() => (window as unknown as PickerFixture).saves), [
      {provider: 'codex', value: {model: 'research', reasoningEffort: null, favorites: []}}
    ]);
  });
  for (const [mode, error] of [['empty-codex', /choose an available Codex model/], ['failed-codex', /retry loading Codex models/]] as const) {
    await t.test(mode === 'empty-codex' ? 'the catalog is empty' : 'discovery fails', async t => {
      const page = await picker(t, mode);
      await assert.rejects(page.evaluate(() => (window as unknown as PickerFixture).codexPickerReady()), error);
      assert.equal(await providerRows(page, 'Codex').locator('.model-choice').count(), 0);
      assert.deepEqual(await page.evaluate(() => (window as unknown as PickerFixture).saves), []);
    });
  }
  await t.test('the removed entry is absent from search and browsing', async t => {
    const page = await picker(t, 'empty');
    await choice(page, 'GPT 6.2').waitFor();
    assert.equal(await choice(page, 'CLI default').count(), 0);
    await search(page).fill('CLI default');
    await choice(page, 'Clear search').waitFor();
    assert.equal(await choice(page, 'CLI default').count(), 0);
  });
});

test('opening before preferences load still starts in Favorites when the active model is favorited', async t => {
  const page = await picker(t, 'slow', false);
  assert.equal(await choice(page, 'Codex models').getAttribute('aria-pressed'), 'true');
  await page.evaluate(() => (window as unknown as PickerFixture).releasePreferences());
  await page.getByRole('button', {name: 'Favorite models', pressed: true}).waitFor();
  assert.equal(await providerRows(page, 'Codex').getByRole('button', {name: 'GPT 6.2', exact: true}).getAttribute('aria-pressed'), 'true');
});

test('slow Codex preference loading cannot overwrite saved choices', async t => {
  const page = await picker(t, 'slow');
  await choice(page, 'OpenCode models').click();
  await providerRows(page, 'OpenCode').getByRole('button', {name: 'GPT 6.2', exact: true}).waitFor();
  assert.equal(await choice(page, 'Fast').count(), 0);
  assert.deepEqual(await page.evaluate(() => (window as unknown as PickerFixture).saves), []);
  await page.evaluate(() => (window as unknown as PickerFixture).releasePreferences());
  await choice(page, 'Fast').click();
  assert.equal(await choice(page, 'OpenCode models').getAttribute('aria-pressed'), 'true');
  const [saved] = await page.evaluate(() => (window as unknown as PickerFixture).saves);
  assert.deepEqual(saved.value, {model: 'gpt-6.2', reasoningEffort: 'low', favorites: ['gpt-6.2'], fastMode: true});
});

test('failed preference and OpenCode catalog discovery can recover in the picker', async t => {
  const page = await picker(t, 'fail-load');
  await choice(page, 'Retry preferences').waitFor();
  assert.equal(await choice(page, 'Fast').count(), 0);
  await page.evaluate(() => (window as unknown as PickerFixture).failLoad = false);
  await choice(page, 'Retry preferences').click();
  await page.getByLabel('Reasoning effort').waitFor();
  await page.evaluate(() => {
    (window as unknown as PickerFixture).failDiscovery = true;
    window.dispatchEvent(new CustomEvent('picker-project', {detail: 'new'}));
  });
  await choice(page, 'OpenCode models').click();
  await choice(page, 'Retry model discovery').waitFor();
  assert.equal(await choice(page, 'Model 10').count(), 0);
  await page.evaluate(() => (window as unknown as PickerFixture).failDiscovery = false);
  await choice(page, 'Retry model discovery').click();
  await choice(page, 'Model 10').waitFor();
  assert.deepEqual(await page.evaluate(() => (window as unknown as PickerFixture).saves), []);
});

test('scoped OpenCode loading ignores obsolete results and blocks stale selection', async t => {
  const page = await picker(t);
  await choice(page, 'OpenCode models').click();
  await choice(page, 'GPT 6.2').click();
  await page.evaluate(() => {
    const w = window as unknown as PickerFixture;
    w.store.discoverOpenCodeModels = project => project === 'slow'
      ? new Promise(resolve => w.releaseCatalog = resolve)
      : Promise.resolve({ok: true, models: [{id: 'lmstudio/replacement', name: 'Replacement', variants: []}]});
    window.dispatchEvent(new CustomEvent('picker-project', {detail: 'slow'}));
  });
  assert.equal(await choice(page, 'GPT 6.2').count(), 0);
  await page.evaluate(() => {
    window.dispatchEvent(new CustomEvent('picker-project', {detail: 'latest'}));
    (window as unknown as PickerFixture).releaseCatalog({ok: true, models: [{id: 'old/old', name: 'Obsolete', variants: []}]});
  });
  await choice(page, 'Replacement').waitFor();
  assert.match(await page.locator('#chat-provider').getAttribute('data-picker-block') ?? '', /Choose an available OpenCode/);
  assert.equal(await choice(page, 'Obsolete').count(), 0);
  await assert.rejects(page.evaluate(() => (window as unknown as PickerFixture).codexPickerReady()), /choose an available OpenCode model/);
  await choice(page, 'Replacement').click();
  assert.equal(await page.locator('#chat-provider').getAttribute('data-picker-block'), '');
  assert.deepEqual(await page.evaluate(() => (window as unknown as PickerFixture).codexPickerReady()), {model: 'lmstudio/replacement', reasoningEffort: null});
});

test('a send waits for persistence and captures its own immutable selection', async t => {
  const page = await picker(t);
  await page.evaluate(() => (window as unknown as PickerFixture).holdSave = true);
  const effort = reasoning(page, 'High');
  await effort.focus();
  await effort.click();
  await page.evaluate(() => {const w = window as unknown as PickerFixture; w.ready = w.codexPickerReady();});
  await reasoning(page, 'Low').click();
  await page.evaluate(() => {const w = window as unknown as PickerFixture; w.holdSave = false; w.releaseSave?.();});
  assert.deepEqual(await page.evaluate(() => (window as unknown as PickerFixture).ready), {model: 'gpt-6.2', reasoningEffort: 'high', fastMode: false});
});

test('save failures stay actionable and retry writes the latest choices', async t => {
  const page = await picker(t, 'fail-save');
  await reasoning(page, 'High').click();
  await choice(page, 'Retry saving').waitFor();
  await assert.rejects(page.evaluate(() => (window as unknown as PickerFixture).codexPickerReady()), /retry saving/);
  await page.evaluate(() => (window as unknown as PickerFixture).failSave = false);
  await choice(page, 'Retry saving').click();
  assert.deepEqual(await page.evaluate(() => (window as unknown as PickerFixture).codexPickerReady()), {model: 'gpt-6.2', reasoningEffort: 'high', fastMode: false});
});

test('picker fits narrow and short viewports and tracks layout changes', async t => {
  const page = await picker(t);
  const panel = page.locator('.model-panel');
  const bounds = async (width: number, height: number) => {
    const box = await panel.boundingBox();
    assert.ok(box && box.x >= 0 && box.y >= 0 && box.x + box.width <= width && box.y + box.height <= height);
    return box;
  };
  const before = await bounds(1200, 800);
  await page.locator('.chat').evaluate(el => (el as HTMLElement).style.right = '260px');
  await page.waitForFunction(x => document.querySelector('.model-panel')!.getBoundingClientRect().x < x - 100, before.x);
  for (const [width, height] of [[450, 420], [340, 300]]) {
    await page.setViewportSize({width, height});
    await choice(page, 'Show reasoning settings').waitFor();
    await bounds(width, height);
    await choice(page, 'Show reasoning settings').click();
    await page.getByLabel('Reasoning effort').waitFor();
    assert.equal(await search(page).isVisible(), false);
    await choice(page, 'Back to models').click();
    assert.ok(await search(page).isVisible());
  }
  await choice(page, 'Show reasoning settings').focus();
  await page.setViewportSize({width: 800, height: 600});
  await page.waitForFunction(() => document.activeElement?.getAttribute('role') === 'radio');
  await page.emulateMedia({forcedColors: 'active'});
  assert.equal(await choice(page, 'Codex models').evaluate(el => getComputedStyle(el).outlineStyle), 'solid');
  const starColor = await choice(page, 'Favorite models').locator('.model-filter-star').evaluate(el => getComputedStyle(el).backgroundColor);
  assert.notEqual(starColor, await choice(page, 'Favorite models').evaluate(el => getComputedStyle(el).backgroundColor));
});

test('saved favorites remain manageable when a provider is missing at startup', async t => {
  const page = await picker(t, 'no-opencode');
  await choice(page, 'Favorite models').click();
  const openCode = providerRows(page, 'OpenCode');
  assert.ok(await openCode.getByRole('button', {name: 'gpt 6.2', exact: true}).isDisabled());
  await openCode.getByRole('button', {name: 'Remove gpt 6.2 from favorites'}).click();
  await openCode.getByRole('button', {name: 'Remove legacy from favorites'}).click();
  assert.equal(await openCode.count(), 0);
  assert.equal(await activeProvider(page), 'codex');
});

test('a failed favorites-only write does not block an unchanged send', async t => {
  const page = await picker(t, 'fail-save');
  await choice(page, 'Codex models').click();
  await choice(page, 'Add Research to favorites').click();
  await choice(page, 'Retry saving').waitFor();
  assert.notEqual(await page.locator('#chat-provider').getAttribute('data-picker-block'), 'Retry saving model settings');
  assert.deepEqual(await page.evaluate(() => (window as unknown as PickerFixture).codexPickerReady()), {model: 'gpt-6.2', reasoningEffort: 'low', fastMode: false});
});

test('search retains its empty state when another provider fails', async t => {
  const page = await picker(t, 'fail-discovery');
  await search(page).fill('no matching model expected');
  await choice(page, 'Retry model discovery').waitFor();
  assert.ok(await page.getByText('No matching models', {exact: true}).filter({visible: true}).count());
  assert.ok(await choice(page, 'Retry model discovery').isVisible());
  await choice(page, 'Clear search').click();
  assert.equal(await search(page).inputValue(), '');
});

test('the collapsed trigger disambiguates OpenCode models with identical names', async t => {
  const page = await picker(t);
  await choice(page, 'OpenCode models').click();
  await providerRows(page, 'OpenCode').getByRole('button', {name: 'GPT 6.2', exact: true}).waitFor();
  await page.evaluate(() => {
    const w = window as unknown as PickerFixture;
    w.store.discoverOpenCodeModels = async () => ({ok: true, models: [
      {id: 'opencode/gpt-6.2', name: 'GPT-6.2', variants: []},
      {id: 'lmstudio/gpt-6.2', name: 'GPT-6.2', variants: []},
      {id: 'lmstudio/gpt-6.2-copy', name: 'GPT-6.2', variants: []}
    ]});
    window.dispatchEvent(new CustomEvent('picker-project', {detail: 'duplicates'}));
  });
  await choice(page, 'OpenCode models').click();
  await page.locator('.model-choice').filter({hasText: 'lmstudio/gpt-6.2-copy'}).click();
  assert.ok(await page.getByRole('region', {name: 'Current model settings'}).getByText('lmstudio/gpt-6.2-copy', {exact: true}).isVisible());
  assert.ok(await page.getByRole('status').filter({hasText: 'lmstudio/gpt-6.2-copy'}).count());
  await page.keyboard.press('Escape');
  assert.match(await trigger(page).getAttribute('aria-label') ?? '', /OpenCode.*GPT 6.2.*lmstudio\/gpt-6.2-copy/);
  assert.match(await trigger(page).innerText(), /lmstudio\/gpt-6.2-copy/);
});
test('failed provider detection retains clear search and offline favorites actions', async t => {
  const page = await picker(t, 'provider-check-failed');
  await search(page).fill('not a model');
  assert.ok(await page.getByText('No matching models', {exact: true}).isVisible());
  assert.ok(await choice(page, 'Check providers').isVisible());
  await choice(page, 'Clear search').click();
  await choice(page, 'Favorite models').click();
  const remove = choice(page, 'Remove legacy from favorites');
  await remove.click();
  assert.equal(await remove.count(), 0);
  assert.ok(await choice(page, 'Check providers').isVisible());
});

test('OpenCode reloads the current project after being unavailable', async t => {
  const page = await picker(t);
  await choice(page, 'OpenCode models').click();
  await choice(page, 'GPT 6.2').click();
  await page.evaluate(() => {
    const w = window as unknown as PickerFixture;
    const select = document.querySelector<HTMLSelectElement>('#chat-provider')!;
    const option = Array.from(select.options).find(option => option.value === 'opencode')!;
    option.disabled = true;
    select.dispatchEvent(new Event('providerstate'));
    w.store.discoverOpenCodeModels = project => new Promise(resolve => {
      w.releaseCatalog = () => resolve({ok: true, models: [{id: 'lmstudio/project', name: project, variants: []}]});
    });
    window.dispatchEvent(new CustomEvent('picker-project', {detail: 'Current project model'}));
    option.disabled = false;
    select.dispatchEvent(new Event('providerstate'));
  });
  assert.equal(await choice(page, 'GPT 6.2').count(), 0);
  assert.match(await page.locator('#chat-provider').getAttribute('data-picker-block') ?? '', /Checking OpenCode models/);
  await page.evaluate(() => (window as unknown as PickerFixture).releaseCatalog(null));
  await choice(page, 'Current project model').waitFor();
});

test('send stops waiting once the selection write finishes, even with a favorite save queued', async t => {
  const page = await picker(t);
  await page.evaluate(() => (window as unknown as PickerFixture).holdSave = true);
  await reasoning(page, 'High').click();
  await choice(page, 'Add Research to favorites').click();
  await page.evaluate(() => {
    const w = window as unknown as PickerFixture;
    w.ready = w.codexPickerReady();
    w.releaseSave?.();
  });
  await page.waitForFunction(() => (window as unknown as PickerFixture).saves.length === 2);
  const selection = await page.evaluate(() => {
    const w = window as unknown as PickerFixture;
    return Promise.race([w.ready, new Promise(resolve => setTimeout(() => resolve('waiting'), 500))]);
  });
  assert.deepEqual(selection, {model: 'gpt-6.2', reasoningEffort: 'high', fastMode: false});
  await page.evaluate(() => (window as unknown as PickerFixture).releaseSave?.());
});
test('same-name offline favorites stay distinguishable and the last failed removal can be retried', async t => {
  const page = await picker(t, 'offline-duplicates');
  await choice(page, 'Favorite models').click();
  const openCode = providerRows(page, 'OpenCode');
  const stars = openCode.getByRole('button', {name: 'Remove shared from favorites', exact: true});
  await stars.last().waitFor();
  assert.deepEqual(await stars.evaluateAll(controls => controls.map(control => control.getAttribute('aria-description'))), [
    'OpenCode, lmstudio/vendor-a/shared', 'OpenCode, lmstudio/vendor-b/shared'
  ]);
  assert.ok(await openCode.getByText('lmstudio/vendor-a/shared · Unavailable', {exact: true}).isVisible());
  assert.ok(await openCode.getByText('lmstudio/vendor-b/shared · Unavailable', {exact: true}).isVisible());
  await stars.first().click();
  await page.evaluate(() => (window as unknown as PickerFixture).failSave = true);
  await stars.first().click();
  const retry = choice(page, 'Retry saving');
  await retry.waitFor();
  await choice(page, 'Close model picker').click();
  await choice(page, 'OpenCode settings weren’t saved').click();
  assert.ok(await retry.evaluate(el => document.activeElement === el));
  await page.evaluate(() => (window as unknown as PickerFixture).failSave = false);
  await page.keyboard.press('Enter');
  await retry.waitFor({state: 'hidden'});
  const saved = await page.evaluate(() => (window as unknown as PickerFixture).saves.at(-1));
  assert.deepEqual(saved?.value.favorites, []);
});

test('a removed provider cannot save an old project default while its catalog is in flight', async t => {
  const page = await picker(t, 'empty-opencode');
  await choice(page, 'OpenCode models').click();
  await page.getByText('No models available', {exact: true}).waitFor();
  await page.evaluate(() => {
    const w = window as unknown as PickerFixture;
    w.store.discoverOpenCodeModels = project => project === 'first'
      ? new Promise(resolve => w.releaseCatalog = resolve)
      : Promise.resolve({ok: true, models: [{id: 'lmstudio/second', name: 'Second project model', variants: []}]});
    window.dispatchEvent(new CustomEvent('picker-project', {detail: 'first'}));
    const select = document.querySelector<HTMLSelectElement>('#chat-provider')!;
    const option = Array.from(select.options).find(option => option.value === 'opencode')!;
    select.dataset.pickerStatus = 'loading';
    select.dispatchEvent(new Event('providerstate'));
    option.disabled = true;
    select.dataset.pickerStatus = 'ready';
    select.dispatchEvent(new Event('providerstate'));
    window.dispatchEvent(new CustomEvent('picker-project', {detail: 'second'}));
    w.releaseCatalog({ok: true, models: [{id: 'lmstudio/first', name: 'First project model', variants: []}]});
  });
  assert.deepEqual(await page.evaluate(() => (window as unknown as PickerFixture).saves), []);
  await page.evaluate(() => {
    const select = document.querySelector<HTMLSelectElement>('#chat-provider')!;
    Array.from(select.options).find(option => option.value === 'opencode')!.disabled = false;
    select.dispatchEvent(new Event('providerstate'));
  });
  await choice(page, 'Second project model').waitFor();
  await page.waitForFunction(() => (window as unknown as PickerFixture).saves.length > 0);
  assert.deepEqual(await page.evaluate(() => (window as unknown as PickerFixture).saves.map(write => write.value.model)), ['lmstudio/second']);
});
test('numbered shortcuts select the visible models and reasoning rows support arrow keys', async t => {
  const page = await picker(t);
  const codex = choice(page, 'Codex models');
  const openCode = choice(page, 'OpenCode models');
  assert.equal(await search(page).getAttribute('placeholder'), 'Search models…');
  await codex.focus();
  await page.keyboard.press('ArrowDown');
  assert.ok(await openCode.evaluate(el => el === document.activeElement));
  await page.keyboard.press('Enter');
  assert.ok(await search(page).evaluate(el => el === document.activeElement));
  assert.equal(await activeProvider(page), 'codex');
  await search(page).fill('6.2');
  assert.equal(await choice(page, 'GPT 6.2').count(), 2);
  assert.equal(await openCode.isVisible(), false);
  await search(page).clear();
  assert.equal(await openCode.getAttribute('aria-pressed'), 'true');
  await codex.click();
  const first = choice(page, 'GPT 6.2');
  await first.waitFor();
  assert.equal(await first.getAttribute('aria-keyshortcuts'), 'Control+1');
  const low = reasoning(page, 'Low');
  await low.focus();
  await page.keyboard.press('ArrowDown');
  const high = reasoning(page, 'High');
  assert.equal(await high.getAttribute('aria-checked'), 'true');
  assert.ok(await high.evaluate(el => el === document.activeElement));
  await search(page).focus();
  await search(page).press('Control+2');
  assert.equal(await choice(page, 'Research').getAttribute('aria-pressed'), 'true');
  assert.equal(await activeProvider(page), 'codex');
});

test('switching Codex connections isolates pending saves and model selections', async t => {
  const page = await picker(t);
  await page.evaluate(() => {
    const w = window as unknown as PickerFixture;
    w.store.codexPreferencesLoad = async mode => ({
      model: mode === 'cli' ? 'gpt-6.2' : 'research',
      reasoningEffort: mode === 'cli' ? 'low' : null,
      favorites: []
    });
    w.store.codexPreferencesSave = async (value, mode) => {
      w.saves.push({provider: mode, value});
      await new Promise<void>(resolve => { w.releaseSave = resolve; });
    };
    const select = document.querySelector<HTMLSelectElement>('#chat-provider')!;
    select.dataset.codexMode = 'cli';
    select.dispatchEvent(new Event('providerstate'));
  });
  await reasoning(page, 'High').click();
  await page.waitForFunction(() => Boolean((window as unknown as PickerFixture).releaseSave));
  await page.evaluate(() => {
    const select = document.querySelector<HTMLSelectElement>('#chat-provider')!;
    select.dataset.codexMode = 'account';
    select.dispatchEvent(new Event('providerstate'));
  });
  await page.waitForFunction(() => document.querySelector('.model-trigger')?.textContent.includes('Research'));
  assert.deepEqual(await page.evaluate(() => (window as unknown as PickerFixture).codexPickerReady()),
    {model: 'research', reasoningEffort: null, fastMode: false});
  const saves = await page.evaluate(() => {
    const w = window as unknown as PickerFixture;
    w.releaseSave();
    return w.saves;
  });
  assert.deepEqual(saves, [{provider: 'cli', value: {model: 'gpt-6.2', reasoningEffort: 'high', favorites: []}}]);
});

test('signed-out Codex prompts for sign-in from the model picker trigger', async t => {
  const page = await picker(t, 'signed-out', false);
  const label = trigger(page).locator('.model-trigger-label');
  assert.equal(await label.textContent(), 'Sign in');
  await page.locator('#chat-provider').evaluate((provider: HTMLSelectElement) => {
    provider.value = 'codex';
    provider.dispatchEvent(new Event('providerstate'));
  });
  assert.equal(await label.textContent(), 'Sign in');
  await trigger(page).click();
  const signIn = choice(page, 'Sign in with ChatGPT');
  assert.ok(await signIn.isVisible());
  await page.waitForFunction(() => document.querySelector<HTMLImageElement>('.model-sign-in img')?.naturalWidth);
  await page.locator('#chat-provider').evaluate(provider => {
    provider.addEventListener('picker-sign-in', () => provider.setAttribute('data-sign-in-requested', 'true'));
  });
  if (process.env.REFRACT_PICKER_CAPTURE) {
    const panel = (await page.locator('.model-panel').boundingBox())!;
    const button = (await trigger(page).boundingBox())!;
    const x = Math.min(panel.x, button.x) - 12;
    const y = panel.y - 12;
    await page.screenshot({path: process.env.REFRACT_PICKER_CAPTURE, clip: {
      x, y, width: Math.max(panel.x + panel.width, button.x + button.width) + 12 - x,
      height: button.y + button.height + 12 - y
    }});
  }
  await signIn.click();
  assert.equal(await page.locator('#chat-provider').getAttribute('data-sign-in-requested'), 'true');
  await page.locator('#chat-provider').evaluate(provider => {
    delete provider.dataset.codexSignInRequired;
    provider.dispatchEvent(new Event('providerstate'));
  });
  assert.equal(await label.textContent(), 'Codex unavailable');
});
