import test = require('node:test');
import assert = require('node:assert/strict');
import fsModule = require('node:fs');
import os = require('node:os');
import path = require('node:path');
const fs = fsModule.promises;
const { _electron: electron } = require('playwright');
const root = path.resolve(__dirname, '..', '..');
test('preferences IPC persists across application restarts and rejects invalid data', { timeout: 60000 }, async () => {
  const data = await fs.mkdtemp(path.join(os.tmpdir(), 'refract-prefs-ui-'));
  const env: NodeJS.ProcessEnv = { ...process.env, REFRACT_DATA_DIR: data, REFRACT_TEST_HIDDEN: '1' }; delete env.ELECTRON_RUN_AS_NODE;
  let app: any;
  try {
    app = await electron.launch({ args: [root], env });
    let page = await app.firstWindow();
    await page.evaluate(() => store.codexAccountSelect('cli'));
    const saved = { model: 'gpt-5.1-codex', reasoningEffort: 'high', favorites: ['gpt-5.1-codex'] };
    await page.evaluate(value => (window as any).store.codexPreferencesSave(value), saved);
    await assert.rejects(page.evaluate(() => (window as any).store.codexPreferencesSave({ model: 'bad\nmodel', reasoningEffort: 'high', favorites: [] })));
    await app.close(); app = null;
    app = await electron.launch({ args: [root], env }); page = await app.firstWindow();
    assert.deepEqual(await page.evaluate(() => (window as any).store.codexPreferencesLoad()), saved);
    await fs.writeFile(path.join(data, 'codex-preferences.json'), '{');
    await assert.rejects(page.evaluate(() => (window as any).store.codexPreferencesLoad()));
    assert.equal(await fs.readFile(path.join(data, 'codex-preferences.json'), 'utf8'), '{');
  } finally { if (app) await app.close(); await fs.rm(data, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 }); }
});

