import childProcess = require('node:child_process');
import fixture = require('../test/fixtures/headless-ui.cjs');
import type { Browser, Page } from 'playwright';

// Build first, then run this script with Node on Windows. It runs one headless
// browser at a time and uses synthetic PDFs, with no Electron or TeX process.
async function measure(page: Page, browser: Browser, scenario: string) {
  const renderer = await page.context().newCDPSession(page);
  const browserSession = await browser.newBrowserCDPSession();
  try {
    const { usedSize } = await renderer.send('Runtime.getHeapUsage');
    const { processInfo } = await browserSession.send('SystemInfo.getProcessInfo');
    const pids = processInfo.map(process => process.id).filter(id => id > 0);
    const command = `ConvertTo-Json -Compress -InputObject @(Get-Process -Id ${pids.join(',')} -ErrorAction SilentlyContinue | Select-Object WorkingSet64,PrivateMemorySize64)`;
    const processes = JSON.parse(childProcess.execFileSync('powershell.exe', ['-NoProfile', '-NonInteractive', '-Command', command], {
      encoding: 'utf8', windowsHide: true, timeout: 10_000
    })) as { WorkingSet64: number; PrivateMemorySize64: number }[];
    const surfaces = await page.locator('.pdf-pages canvas').evaluateAll(elements => {
      const pixels = (elements as HTMLCanvasElement[]).map(canvas => canvas.width * canvas.height);
      return { rasters: pixels.filter(Boolean).length, rasterBytes: pixels.reduce((sum, count) => sum + count, 0) * 4 };
    });
    console.log(JSON.stringify({
      scenario, ...surfaces, heapBytes: usedSize,
      workingSetBytes: processes.reduce((sum, process) => sum + process.WorkingSet64, 0),
      privateBytes: processes.reduce((sum, process) => sum + process.PrivateMemorySize64, 0)
    }));
  } finally {
    await renderer.detach();
    await browserSession.detach();
  }
}

async function openProject(page: Page) {
  await page.locator('.project-open').first().click();
  await page.getByText('Compiled', { exact: true }).waitFor();
  await page.waitForFunction(() => (document.querySelector<HTMLCanvasElement>('.pdf-pages canvas')?.width ?? 0) > 0);
}

async function run() {
  const repetitions = Number(process.argv[2] ?? 3);
  if (!Number.isSafeInteger(repetitions) || repetitions < 1 || repetitions > 10) throw new Error('Use 1 to 10 repetitions');
  for (let index = 0; index < repetitions; index++) {
    for (const pageCount of [1, 100]) {
      const { browser, page, errors } = await fixture.openHeadlessUi('light', undefined, { pageCount, reducedMotion: 'reduce' });
      try {
        await page.evaluate(() => Object.defineProperty(window, 'devicePixelRatio', { configurable: true, value: 2 }));
        if (pageCount === 1) {
          await page.locator('.project-open').first().waitFor();
          await page.waitForTimeout(600);
          await measure(page, browser, 'Home');
        }
        await openProject(page);
        await page.waitForTimeout(600);
        await measure(page, browser, `${pageCount} page(s) at fit width`);
        if (errors.length) throw new Error(errors.join('\n'));
        if (pageCount === 1) continue;
        for (let step = 0; step < 2; step++) await page.getByLabel('Zoom in compiled PDF').click();
        const canvases = page.locator('.pdf-pages canvas');
        for (const number of [12, 24, 36, 48, 60, 72, 84, 96]) {
          await canvases.nth(number - 1).scrollIntoViewIfNeeded();
          await page.waitForFunction(pageNumber => {
            const canvas = document.querySelectorAll<HTMLCanvasElement>('.pdf-pages canvas')[pageNumber - 1];
            return Number(canvas?.dataset.renderScale) >= 2.09;
          }, number);
        }
        await page.waitForTimeout(600);
        await measure(page, browser, '100 pages after eight jumps at 150%');
        await page.locator('#home').click();
        await page.locator('.project-open').first().waitFor();
        await page.waitForTimeout(600);
        await measure(page, browser, 'Home after closing 100 pages');
        if (errors.length) throw new Error(errors.join('\n'));
      } finally { await browser.close(); }
    }
  }
}

void run().catch(error => { console.error(error); process.exitCode = 1; });
