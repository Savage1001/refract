import test = require('node:test');
import assert = require('node:assert/strict');
import fsModule = require('node:fs');
const fs = fsModule.promises;
import os = require('node:os');
import path = require('node:path');
const { CodexPreferencesStore, validateCodexPreferences } = require('../src/codex-preferences.cjs');

test('preferences survive a new store and ordered writes', async () => {
  const dir = await fs.mkdtemp(path.join(os.tmpdir(), 'refract-preferences-'));
  try {
    const file = path.join(dir, 'preferences.json');
    const store = new CodexPreferencesStore(file);
    assert.deepEqual(await store.load(), { model: null, reasoningEffort: null, favorites: [] });
    await Promise.all([store.save({ model: 'first', reasoningEffort: null, favorites: [] }), store.save({ model: 'second', reasoningEffort: 'high', favorites: ['second'] })]);
    assert.deepEqual(await new CodexPreferencesStore(file).load(), { model: 'second', reasoningEffort: 'high', favorites: ['second'] });
    await fs.writeFile(file, '{');
    await assert.rejects(store.load());
  } finally { await fs.rm(dir, { recursive: true, force: true }); }
});
test('rejects invalid snapshots', () => {
  for (const value of [null, {}, { model: '', reasoningEffort: null, favorites: [] }, { model: null, reasoningEffort: 'bogus', favorites: [] }, { model: null, reasoningEffort: null, favorites: ['a', 'a'] }, { model: null, reasoningEffort: null, favorites: ['a\n'] }]) assert.throws(() => validateCodexPreferences(value));
});
test('accepts current maximum reasoning efforts', () => {
  for (const reasoningEffort of ['max', 'ultra']) assert.equal(validateCodexPreferences({model: 'example', reasoningEffort, favorites: []}).reasoningEffort, reasoningEffort);
});



test('OpenCode custom variants persist while Codex validation remains strict', async () => {
 const dir = await fs.mkdtemp(path.join(os.tmpdir(), 'refract-variants-'));
 try { const file = path.join(dir, 'preferences.json'); const prefs = {model:'vendor/model',reasoningEffort:'deep',favorites:[]}; await new CodexPreferencesStore(file, true).save(prefs); assert.deepEqual(await new CodexPreferencesStore(file, true).load(), prefs); assert.throws(() => validateCodexPreferences(prefs)); assert.throws(() => validateCodexPreferences({...prefs,reasoningEffort:'bad#variant'},true)); }
 finally { await fs.rm(dir,{recursive:true,force:true}); }
});

test('speed preference survives restart and validates its type', async () => {
 const dir = await fs.mkdtemp(path.join(os.tmpdir(),'refract-speed-'));
 try { const file = path.join(dir,'prefs.json'); const prefs = {model:null,reasoningEffort:null,favorites:[],fastMode:true};
 await new CodexPreferencesStore(file).save(prefs); assert.deepEqual(await new CodexPreferencesStore(file).load(),prefs);
 assert.throws(() => validateCodexPreferences({...prefs,fastMode:'true'}));
 assert.throws(() => validateCodexPreferences(prefs,true));
 } finally { await fs.rm(dir,{recursive:true,force:true}); }
});
