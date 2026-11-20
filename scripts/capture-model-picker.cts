import fs = require('node:fs/promises');
import path = require('node:path');
import fixture = require('../test/fixtures/headless-ui.cjs');
import type {Page} from 'playwright';

const output = path.resolve('docs/model-picker');

async function openPicker(page: Page): Promise<void> {
  await page.evaluate(() => {
    interface FixtureWindow extends Window {store: Record<string, unknown>}
    const w = window as unknown as FixtureWindow;
    const model = (id: string, label: string, description: string, levels: string[]) => ({
      id, model: id, displayName: label, description,
      supportedReasoningEfforts: levels.map(effort => ({effort, description: ''})),
      defaultReasoningEffort: 'medium', isDefault: false
    });
    w.store.discoverCodexModels = async () => ({ok: true, models: [
      model('gpt-6.2-sol', 'GPT-6.2 Sol', 'For everyday writing, research, and coding.', ['low', 'medium', 'high', 'xhigh']),
      model('gpt-6.2-astra', 'GPT-6.2 Astra', 'For complex work and multi-step reasoning.', ['medium', 'high', 'xhigh']),
      model('gpt-6.1-sol', 'GPT-6.1 Sol', 'For drafting and routine tasks.', ['low', 'medium', 'high'])
    ]});
    w.store.codexPreferencesLoad = async () => ({model: 'gpt-6.2-sol', reasoningEffort: 'high', favorites: ['gpt-6.2-sol']});
    w.store.discoverOpenCodeModels = async () => ({ok: true, models: [
      {id: 'lmstudio/qwen-3.5-9b', name: 'Qwen 3.5 9B', variants: ['high', 'max']},
      {id: 'opencode/deepseek-v4-flash', name: 'DeepSeek V4 Flash', variants: ['low', 'high']},
      {id: 'anthropic/claude-sonnet-4', name: 'Claude Sonnet 4', variants: ['low', 'medium', 'high']}
    ]});
    w.store.openCodePreferencesLoad = async () => ({model: 'lmstudio/qwen-3.5-9b', reasoningEffort: 'high', favorites: ['lmstudio/qwen-3.5-9b']});
  });
  await page.getByRole('button', {name: 'Open Spectral methods for fluid dynamics', exact: true}).click();
  await page.locator('#source').waitFor({state: 'visible'});
  await page.getByText('Compiled', {exact: true}).waitFor();
  await page.waitForFunction(() => document.querySelector('.model-trigger-label')?.textContent === 'GPT-6.2 Sol');
  await page.locator('.model-trigger').click();
  await page.getByRole('button', {name: 'Qwen 3.5 9B', exact: true}).waitFor();
  await page.evaluate(() => document.fonts.ready);
}

async function main(): Promise<void> {
  await fs.mkdir(output, {recursive: true});
  for (const theme of ['light', 'dark'] as const) {
    const {browser, page, errors} = await fixture.openHeadlessUi(theme);
    try {
      await openPicker(page);
      if (theme === 'light') {
        await page.screenshot({path: path.join(output, 'codex-light.png')});
      } else {
        await page.getByRole('button', {name: 'OpenCode models'}).click();
        await page.getByRole('button', {name: 'Qwen 3.5 9B', exact: true}).click();
        await page.getByRole('button', {name: 'Favorite models'}).click();
        await page.locator('.model-panel').screenshot({path: path.join(output, 'favorites-dark.png')});
        await page.keyboard.press('Escape');
        await page.setViewportSize({width: 445, height: 650});
        await page.locator('.model-trigger').click();
        await page.getByRole('searchbox', {name: 'Search models'}).fill('qwen');
        await page.locator('.model-panel').screenshot({path: path.join(output, 'search-compact.png')});
      }
      if (errors.length) throw new Error(errors.join('\n'));
    } finally {
      await browser.close();
    }
  }
}
void main().catch(error => {console.error(error); process.exitCode = 1;});
