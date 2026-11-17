import nodeTest = require('node:test');
import assert = require('node:assert/strict');
import fs = require('node:fs/promises');
import os = require('node:os');
import path = require('node:path');
import playwright = require('playwright');
import type { CodexDiscoveryApi } from '../src/codex-models.cjs';

const root = path.resolve(__dirname, '../..');
nodeTest.test('preload exposes discovery results and contains unexpected failures', { timeout: 30000 }, async t => {
  const data = await fs.mkdtemp(path.join(os.tmpdir(), 'refract-model-ipc-'));
  const env: NodeJS.ProcessEnv = { ...process.env, REFRACT_DATA_DIR: data, REFRACT_TEST_HIDDEN: '1' };
  delete env.ELECTRON_RUN_AS_NODE;
  const app = await playwright._electron.launch({ args: [root], env });
  t.after(async () => { await app.close(); await fs.rm(data, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 }); });
  const page = await app.firstWindow();
  await app.evaluate(({ app }) => {
    const requireFromApp = process.getBuiltinModule('module').createRequire(app.getAppPath() + '/build/main.cjs');
    const discovery = requireFromApp('./src/codex-models.cjs');
    const original = discovery.discoverCodexModels;
    discovery.discoverCodexModels = () => original({ script: app.getAppPath() + '/build/test/codex-model-fixture.cjs' });
  });
  const read = () => page.evaluate(() => (window as unknown as { store: CodexDiscoveryApi }).store.discoverCodexModels());
  const success = await read();
  assert.equal(success.ok, true);
  if (success.ok) {
    assert.deepEqual(success.models.map(model => model.model), ['one', 'two']);
    assert.deepEqual(success.models[0].supportedReasoningEfforts, [{ effort: 'high', description: 'Thorough' }]);
  }
  await app.evaluate(() => { process.env.REFRACT_MODEL_TEST = 'error'; });
  assert.deepEqual(await read(), { ok: false, error: 'Codex model discovery request failed' });
  await app.evaluate(({ app }) => {
    const requireFromApp = process.getBuiltinModule('module').createRequire(app.getAppPath() + '/build/main.cjs');
    requireFromApp('./src/codex-models.cjs').discoverCodexModels = async () => { throw new Error('private details'); };
  });
  assert.deepEqual(await read(), { ok: false, error: 'Codex model discovery failed' });
});
