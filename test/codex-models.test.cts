import nodeTest = require('node:test');
const { test } = nodeTest;
import assert = require('node:assert/strict');
import path = require('node:path');
const { join } = path;
import models = require('../src/codex-models.cjs');
const { discoverCodexModels } = models;
const script = join(__dirname, 'codex-model-fixture.cjs');
test('normalizes the current CLI reasoningEffort catalog field', async () => {
 process.env.REFRACT_MODEL_TEST = 'current';
 try {
  const result = await discoverCodexModels({script});
  assert.equal(result.ok, true);
  if (result.ok) assert.deepEqual(result.models[0].supportedReasoningEfforts, [{effort: 'ultra', description: 'Maximum'}]);
 } finally { delete process.env.REFRACT_MODEL_TEST; }
});
test('discovers all pages and preserves supported efforts', async () => {
 delete process.env.REFRACT_MODEL_TEST;
 const result = await discoverCodexModels({ script });
 assert.equal(result.ok, true);
 if (result.ok) { assert.deepEqual(result.models.map(m => m.id), ['one', 'two']); assert.deepEqual(result.models[0].supportedReasoningEfforts, [{ effort: 'high', description: 'Thorough' }]); }
});
test('reports failed discovery and timeout without throwing', async () => {
 process.env.REFRACT_MODEL_TEST = 'error';
 assert.equal((await discoverCodexModels({ script })).ok, false);
 process.env.REFRACT_MODEL_TEST = 'hang';
 assert.equal((await discoverCodexModels({ script, timeoutMs: 100 })).ok, false);
 delete process.env.REFRACT_MODEL_TEST;
});


test('discovery exposes identifiers accepted by preferences and execution', async () => {
 process.env.REFRACT_MODEL_TEST = 'identifiers';
 try {
  const result = await discoverCodexModels({script});
  assert.equal(result.ok, true);
  if (result.ok) assert.deepEqual(result.models.map(m => m.id), ['--vendor/model with space']);
 } finally { delete process.env.REFRACT_MODEL_TEST; }
});
