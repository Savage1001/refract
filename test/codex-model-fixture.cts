import readline = require('node:readline');
const { createInterface } = readline;
const mode = process.env.REFRACT_MODEL_TEST;
createInterface({ input: process.stdin }).on('line', line => {
 const msg = JSON.parse(line);
 if (mode === 'hang') return;
 if (msg.method === 'initialize') console.log(JSON.stringify({ id: 1, result: {} }));
 if (msg.method === 'model/list') {
  if (mode === 'error') return console.log(JSON.stringify({ id: 2, error: { message: 'offline' } }));
  const model = { id: 'one', model: 'one', displayName: 'One', description: 'Example', supportedReasoningEfforts: [{ effort: 'high', description: 'Thorough' }], defaultReasoningEffort: 'high', isDefault: true };
  if (mode === 'current') return console.log(JSON.stringify({id: 2, result: {data: [{...model, supportedReasoningEfforts: [{reasoningEffort: 'ultra', description: 'Maximum'}]}], nextCursor: null}}));
  if (mode === 'identifiers') return console.log(JSON.stringify({id: 2, result: {data: [{...model, id: '--vendor/model with space', model: '--vendor/model with space'}, {...model, id: 'bad\nmodel'}, {...model, model: ''}], nextCursor: null}}));
  console.log(JSON.stringify({ id: 2, result: { data: msg.params.cursor ? [{...model, id: 'two', model: 'two'}] : [model], nextCursor: msg.params.cursor ? null : 'next' } }));
 }
});

