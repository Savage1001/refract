import test = require('node:test');
import assert = require('node:assert/strict');
import fixture = require('./fixtures/headless-ui.cjs');

test('renderer displays compile status and control symbols as Unicode', {timeout: 30000}, async () => {
 const {browser, page, errors} = await fixture.openHeadlessUi('dark', {pdf: '', source: '% Sample'});
 try {
  await page.evaluate(() => {
   const fixtureWindow = window as unknown as {store: {compile: () => Promise<unknown>}};
   fixtureWindow.store.compile = () => new Promise(() => {});
  });

  const project = page.getByRole('button', {name: 'Open Spectral methods for fluid dynamics'});
  assert.equal(await project.locator('.project-open-hint').textContent(), 'Open project →');
  await project.click();

  const compile = page.locator('#compile');
  await compile.locator('xpath=self::*[@aria-disabled="true"]').waitFor();
  assert.equal(await compile.textContent(), 'Compiling…');

  await page.getByRole('button', {name: 'Chats', exact: true}).click();
  const rename = page.getByRole('button', {name: 'Rename Research conversation'});
  assert.equal(await rename.textContent(), '✎');

  await page.getByRole('button', {name: 'Find', exact: true}).click();
  for (const [name, symbol] of [['Previous match', '↑'], ['Next match', '↓'], ['Close find', '×']]) {
   assert.equal(await page.getByRole('button', {name, exact: true}).textContent(), symbol);
  }

  await page.getByRole('button', {name: 'Close find', exact: true}).click();
  await page.locator('#tools').click();
  await page.locator('#tool-versions').click();
  await page.getByText(/ · 2 files/).waitFor();
  assert.deepEqual(errors, []);
 } finally {
  await browser.close();
 }
});
