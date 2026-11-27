import assert = require('node:assert/strict');
import fs = require('node:fs');
import path = require('node:path');
const { app, BrowserWindow } = require('electron') as typeof import('electron');

const profile = process.env.REFRACT_TEST_LEGACY_PROFILE_DIR;
const project = process.env.REFRACT_TEST_LEGACY_PROFILE_PROJECT;
assert.ok(profile && path.isAbsolute(profile));
assert.ok(project && /^[0-9a-f-]{36}$/.test(project));
app.setPath('userData', profile);

void app.whenReady().then(async () => {
  const page = path.join(profile, 'legacy-profile-fixture.html');
  fs.writeFileSync(page, '<!doctype html><meta charset="utf-8"><title>Refract migration fixture</title>', { flag: 'wx' });
  const window = new BrowserWindow({ show: false, webPreferences: { backgroundThrottling: false } });
  await window.loadFile(page);
  const saved = {
    'refract-project-sort': 'edited',
    'refract-project-view': 'grid',
    'refract.workspace-panes.v1': JSON.stringify({ agent: false, editor: true, preview: false }),
    'refract-outline-list-height': '144',
    [`refract-chat-draft:${project}:default`]: JSON.stringify({
      text: 'Résumé π 🧪',
      attachments: [{ token: 'fixture.png', name: 'Sketch.png' }],
      provider: 'opencode',
    }),
  };
  await window.webContents.executeJavaScript(
    `for (const [key, value] of Object.entries(${JSON.stringify(saved)})) localStorage.setItem(key, value)`
  );
  window.webContents.session.flushStorageData();
  app.quit();
}).catch(() => {
  console.error('Synthetic Electron local storage fixture failed.');
  app.exit(1);
});
