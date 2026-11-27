import assert = require('node:assert/strict');
import fs = require('node:fs');
import path = require('node:path');
const { app, safeStorage } = require('electron') as typeof import('electron');

const operation = process.env.REFRACT_TEST_ACCOUNT_FIXTURE_ACTION;
const profile = process.env.REFRACT_TEST_ACCOUNT_FIXTURE_DIR;
assert.ok(process.platform === 'win32' && profile && path.isAbsolute(profile));
assert.ok(operation === 'write' || operation === 'verify');
app.setPath('userData', profile);

void app.whenReady().then(() => {
  assert.ok(safeStorage.isEncryptionAvailable());
  const directory = path.join(profile, 'codex-account');
  const fixture = path.join(directory, 'connection.bin');
  const registration = { clientId: 'fixture-client', subject: 'fixture-subject' };
  const record = {
    hostId: 'urn:uuid:00000000-0000-4000-8000-000000000001',
    registration,
    session: {
      ...registration, accessToken: 'fixture-electron', refreshToken: 'fixture-refresh', expiresAt: 200_000,
    },
  };
  if (operation === 'write') {
    fs.mkdirSync(directory);
    fs.writeFileSync(fixture, safeStorage.encryptString(JSON.stringify(record)), { flag: 'wx' });
  } else {
    record.session.expiresAt = 400_000;
    const decoded: unknown = JSON.parse(safeStorage.decryptString(fs.readFileSync(fixture)));
    assert.deepEqual(decoded, record);
  }
  app.quit();
}).catch(() => {
  console.error('Synthetic Electron account storage fixture failed.');
  app.exit(1);
});
