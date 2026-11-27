import assert = require('node:assert/strict');
import fs = require('node:fs/promises');
import path = require('node:path');
import fixture = require('../test/fixtures/headless-ui.cjs');
import type { Page } from 'playwright';

const desktop = { width: 1440, height: 900 };
const compact = { width: 900, height: 650 };

interface CaptureDocument {
  pdf: string;
  source: string;
}

async function captureTheme(output: string, theme: 'light' | 'dark', document: CaptureDocument): Promise<void> {
  const { browser, page, errors } = await fixture.openHeadlessUi(theme, document, {
    viewport: desktop,
    reducedMotion: 'reduce',
  });
  page.setDefaultTimeout(10_000);
  const project = page.getByRole('button', { name: 'Open Spectral methods for fluid dynamics', exact: true });
  const settings = page.getByRole('dialog', { name: 'Settings', exact: true });
  try {
    await project.waitFor();
    await capture(page, output, theme, 'library-list');
    await page.locator('#grid-view').click();
    await capture(page, output, theme, 'library-grid');

    await page.getByRole('button', { name: 'Settings', exact: true }).click();
    await settings.waitFor();
    await capture(page, output, theme, 'settings');
    await settings.getByRole('button', { name: 'Close', exact: true }).click();
    await settings.waitFor({ state: 'hidden' });

    await project.click();
    await page.mouse.move(0, 0);
    await page.locator('#workspace').waitFor({ state: 'visible' });
    await page.getByRole('button', { name: 'Compiled', exact: true }).waitFor();
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
  } catch (error) {
    const diagnostics = errors.length ? ` Renderer errors: ${errors.join(' | ')}` : '';
    throw new Error(`Failed to capture the ${theme} reference screenshots.${diagnostics}`, { cause: error });
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
  const documentsDir = path.resolve(__dirname, '../../test/fixtures');
  const [source, pdf] = await Promise.all([
    fs.readFile(path.join(documentsDir, 'parity-document.tex'), 'utf8'),
    fs.readFile(path.join(documentsDir, 'parity-document.pdf.b64'), 'ascii'),
  ]);
  const document = { source: source.replace(/\r\n/g, '\n'), pdf: pdf.replace(/\s+/g, '') };
  await fs.mkdir(output, { recursive: true });
  for (const theme of ['light', 'dark'] as const) {
    await captureTheme(output, theme, document);
  }
}

void main().catch(error => {
  console.error(error);
  process.exitCode = 1;
});
