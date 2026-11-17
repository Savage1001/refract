const { test } = require('node:test');
const assert = require('node:assert/strict');
const { OpenCodeEvents } = require('../src/opencode.cjs');

test('OpenCode events preserve sessions and collect streamed response parts across chunks', () => {
  const sessions: string[] = []; const progress: string[] = [];
  const events = new OpenCodeEvents(null, (s: string) => progress.push(s), (s: string) => sessions.push(s));
  const data = Buffer.from('{"type":"step_start","sessionID":"ses_123"}\n{"type":"text","sessionID":"ses_123","part":{"text":"hé"}}\n{"type":"text","part":{"text":"llo"}}\n');
  for (const byte of data) events.write(Buffer.from([byte])); events.end();
  assert.equal(events.text, 'héllo'); assert.equal(events.sessionId, 'ses_123'); assert.deepEqual(sessions, ['ses_123']); assert.equal(progress.length, 1);
});
test('OpenCode errors and oversized events do not hide later session events', () => {
  const events = new OpenCodeEvents(null); events.write(Buffer.from('x'.repeat(4 * 1024 * 1024 + 1) + '\n{"type":"error","sessionID":"ses_1","error":{"data":{"message":"failed"}}}\n')); events.end();
  assert.equal(events.oversized, true); assert.equal(events.error, 'failed'); assert.equal(events.sessionId, 'ses_1');
});

test('OpenCode receives the selected model for new and resumed sessions', () => {
 const { openCodeArguments } = require('../src/opencode.cjs');
 assert.deepEqual(openCodeArguments(null, [], 'opencode/model-2'), ['run','--format','json','--model','opencode/model-2']);
 assert.deepEqual(openCodeArguments('ses_1', ['image.png'], 'opencode/model-2'), ['run','--format','json','--model','opencode/model-2','--session','ses_1','--file','image.png']);
 assert.deepEqual(openCodeArguments('ses_1', [], 'opencode/model-2',undefined,true), ['run','--format','json','--standalone','--model','opencode/model-2','--session','ses_1']);
});

test('OpenCode V2 ephemeral configuration preserves user servers without mutating the source',()=>{
 const {openCodeEnvironment}=require('../src/opencode.cjs');
 const base={OPENCODE_CONFIG_CONTENT:JSON.stringify({model:'existing',mcp:{timeout:{execution:90000},servers:{existing:{type:'remote',url:'https://example.com'}},legacy:{type:'local',command:['example']}}})};
 const config=JSON.parse(openCodeEnvironment({url:'http://127.0.0.1:1234/mcp',token:'test-token'},base,true).OPENCODE_CONFIG_CONTENT);
 assert.equal(config.model,'existing');assert.equal(config.mcp.timeout.execution,90000);assert.equal(config.mcp.servers.existing.url,'https://example.com');
 assert.deepEqual(config.mcp.servers.legacy.command,['example']);assert.equal(config.mcp.servers.refract.codemode,false);assert.equal(config.mcp.servers.refract.headers.Authorization,'Bearer test-token');
 assert.equal(JSON.parse(base.OPENCODE_CONFIG_CONTENT).mcp.servers.refract,undefined);
});


test('OpenCode catalog retains each model?s actual variants, including custom variants', () => {
 const { parseOpenCodeCatalog, openCodeArguments } = require('../src/opencode.cjs');
 assert.deepEqual(parseOpenCodeCatalog({data:[{providerID:'vendor',id:'family/model',name:'Family Model',variants:[{id:'low'},{id:'deep'},{id:'bad#value'}]},{providerID:'vendor',id:'plain',variants:[]}]}), [{id:'vendor/family/model',name:'Family Model',variants:['low','deep']},{id:'vendor/plain',name:'plain',variants:[]}]);
 assert.deepEqual(openCodeArguments('session', [], 'vendor/family/model', 'deep'), ['run','--format','json','--model','vendor/family/model#deep','--session','session']);
 assert.throws(() => parseOpenCodeCatalog({}));
});
