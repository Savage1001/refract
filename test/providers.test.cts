const { test } = require('node:test');
const assert = require('node:assert/strict');
const { probe } = require('../src/providers.cjs');

test('provider probes require a working executable and successful exit', async () => {
  assert.equal(await probe(process.execPath, ['--version']), true);
  assert.equal(await probe(process.execPath, ['--definitely-not-a-real-option']), false);
  assert.equal(await probe('refract-nonexistent-provider-executable', ['--version']), false);
});
