import assert = require('node:assert/strict');
import fs = require('node:fs/promises');
import path = require('node:path');
import fixture = require('../test/fixtures/headless-ui.cjs');
import type { Page } from 'playwright';

const desktop = { width: 1440, height: 900 };
const compact = { width: 900, height: 650 };
const workspaceText = ['.brand-home span', '#project-name', '.sidebar-label', '#file-search-input',
  '.file-tabs button', '#numbers', '#source', '.outline-label', '.chat-title', '#chat-messages > strong',
  '#chat-input', '.model-trigger-label', '.model-trigger-options', '#compile'];
const typography = {
  'library-list': ['.homepage-brand', '.project-header h1', '#import', '#new', '#search', '.table-head', '.project-label', '.project-date'],
  'library-grid': ['.homepage-brand', '.project-header h1', '#import', '#new', '.project-label', '.project-date'],
  settings: ['#settings-title', '#settings-dialog label', '#codex-connection', '#update-track', '.settings-description', '#update-status', '.dialog-actions button'],
  workspace: workspaceText,
  'workspace-comments': ['#tool-comments', '#tool-body', '#tool-body input', '#tool-body textarea'],
  'workspace-compact': workspaceText,
} satisfies Record<string, string[]>;

interface CaptureDocument {
  pdf: string;
  source: string;
  version: string;
}

async function captureTheme(output: string, theme: 'light' | 'dark', document: CaptureDocument, channel: 'msedge' | 'chromium'): Promise<void> {
  const { browser, page, errors } = await fixture.openHeadlessUi(theme, document, {
    viewport: desktop,
    reducedMotion: 'reduce',
    channel,
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

async function capture(page: Page, output: string, theme: string, screen: keyof typeof typography): Promise<void> {
  const samples = await page.evaluate(async selectors => {
    await document.fonts.ready;
    return selectors.flatMap(selector => {
      const element = Array.from(document.querySelectorAll(selector)).find(node => node.getClientRects().length > 0);
      if (!element) return [];
      const { fontFamily, fontSize, fontWeight, fontStyle, lineHeight, letterSpacing, color } = getComputedStyle(element);
      return [{ selector, fontFamily, fontSize, fontWeight, fontStyle, lineHeight, letterSpacing, color,
        bounds: element.getBoundingClientRect().toJSON() as Pick<DOMRectReadOnly, 'x' | 'y' | 'width' | 'height'> }];
    });
  }, typography[screen]);
  const filename = path.join(output, `${theme}-${screen}`);
  await Promise.all([
    fs.writeFile(`${filename}.json`, `${JSON.stringify(samples, null, 2)}\n`),
    page.screenshot({ path: `${filename}.png`, animations: 'disabled', caret: 'hide', scale: 'css' }),
  ]);
}

async function main(): Promise<void> {
  const output = process.argv[2];
  const browser = process.argv[3];
  if (!output || (browser !== undefined && browser !== '--chromium')) {
    throw new Error('Pass an output folder for the screenshots, followed by optional --chromium.');
  }
  const documentsDir = path.resolve(__dirname, '../../test/fixtures');
  const [source, pdf, manifest] = await Promise.all([
    fs.readFile(path.join(documentsDir, 'parity-document.tex'), 'utf8'),
    fs.readFile(path.join(documentsDir, 'parity-document.pdf.b64'), 'ascii'),
    fs.readFile(path.resolve(__dirname, '../../package.json'), 'utf8'),
  ]);
  const metadata = JSON.parse(manifest) as { version?: unknown };
  assert.equal(typeof metadata.version, 'string', 'Expected the Refract version in package.json');
  const document = { source: source.replace(/\r\n/g, '\n'), pdf: pdf.replace(/\s+/g, ''), version: metadata.version as string };
  await fs.mkdir(output, { recursive: true });
  for (const theme of ['light', 'dark'] as const) {
    await captureTheme(output, theme, document, browser === '--chromium' ? 'chromium' : 'msedge');
  }
}

void main().catch(error => {
  console.error(error);
  process.exitCode = 1;
});
