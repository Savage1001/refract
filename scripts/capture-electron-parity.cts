import assert = require('node:assert/strict');
import fs = require('node:fs/promises');
import path = require('node:path');
import fixture = require('../test/fixtures/headless-ui.cjs');
import type { Page } from 'playwright';

const desktop = { width: 1440, height: 900 };
const compact = { width: 900, height: 650 };

async function captureTheme(output: string, theme: 'light' | 'dark'): Promise<void> {
  const { browser, page, errors } = await fixture.openHeadlessUi(theme, undefined, {
    viewport: desktop,
    reducedMotion: 'reduce',
  });
  page.setDefaultTimeout(10_000);
  try {
    await page.getByRole('button', { name: 'Open Spectral methods for fluid dynamics', exact: true }).waitFor();
    await capture(page, output, theme, 'library-list');
    await page.locator('#grid-view').click();
    await capture(page, output, theme, 'library-grid');

    await page.getByRole('button', { name: 'Settings', exact: true }).click();
    await page.getByRole('dialog', { name: 'Settings', exact: true }).waitFor();
    await capture(page, output, theme, 'settings');
    await page.getByRole('button', { name: 'Close', exact: true }).click();

    await page.getByRole('button', { name: 'Open Spectral methods for fluid dynamics', exact: true }).press('Enter');
    await page.locator('.pdf-pages canvas[data-render-scale]').first().waitFor({ state: 'visible' });
    await page.locator('.pdf-text-layer').first().waitFor({ state: 'attached' });
    await capture(page, output, theme, 'workspace');

    await page.locator('#tools').click();
    await page.locator('#tool-comments').click();
    await page.getByText('Clarify the assumptions before stating the main result.', { exact: true }).waitFor();
    await capture(page, output, theme, 'workspace-comments');
    await page.locator('#tools').click();

    await page.setViewportSize(compact);
    await capture(page, output, theme, 'workspace-compact');
    assert.deepEqual(errors, []);
  } finally {
    await browser.close();
  }
}

async function capture(page: Page, output: string, theme: string, screen: string): Promise<void> {
  await page.evaluate(async () => { await document.fonts.ready; });
  await page.screenshot({
    path: path.join(output, `${theme}-${screen}.png`),
    animations: 'disabled',
    caret: 'hide',
    scale: 'css',
  });
}

async function main(): Promise<void> {
  const output = process.argv[2];
  if (!output) throw new Error('Pass an output folder for the screenshots.');
  await fs.mkdir(output, { recursive: true });
  for (const theme of ['light', 'dark'] as const) {
    await captureTheme(output, theme);
  }
}

void main().catch(error => {
  console.error(error);
  process.exitCode = 1;
});
