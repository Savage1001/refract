import test = require('node:test');
import assert = require('node:assert/strict');
import fs = require('node:fs/promises');
import path = require('node:path');
import os = require('node:os');
import diagnosticsModule = require('../src/update-diagnostics.cjs');
const { UpdateDiagnostics } = diagnosticsModule;

test('stalled shutdown stays pending, reports its stage, and can finish without killing work', async () => {
  const directory = await fs.mkdtemp(path.join(os.tmpdir(), 'refract-update-log-'));
  try {
    const log = new UpdateDiagnostics(directory, '1.0.0');
    let finish!: () => void;
    let report!: () => void;
    const reported = new Promise<void>(resolve => report = resolve);
    let completed = false;
    const running = log.stage('assistant', () => new Promise<void>(resolve => finish = resolve), report, 1).then(() => {completed = true;});
    await reported;
    assert.equal(completed, false);
    finish(); await running;
    await assert.rejects(log.stage('save', async () => {throw new Error('disk full');}, () => {}));
    const rows = (await fs.readFile(path.join(directory, 'updates.jsonl'), 'utf8')).trim().split('\n').map(line => JSON.parse(line));
    assert.deepEqual(rows.map(row => row.event), ['shutdown-stage-start', 'shutdown-stage-stalled', 'shutdown-stage-end', 'shutdown-stage-start', 'shutdown-stage-error']);
    assert.ok(rows.every(row => row.pid === process.pid && row.version === '1.0.0' && row.timestamp));
  } finally {await fs.rm(directory, {recursive: true, force: true});}
});
