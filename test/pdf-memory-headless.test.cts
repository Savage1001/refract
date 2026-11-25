import test = require('node:test');
import assert = require('node:assert/strict');
import fixture = require('./fixtures/headless-ui.cjs');

test('compiled previews discard offscreen rasters and restore pages on demand', {timeout:30_000}, async () => {
  const {browser, page, errors} = await fixture.openHeadlessUi('light', undefined, {pageCount:100, reducedMotion:'reduce'});
  try {
    await page.locator('.project-open').first().click();
    await page.getByText('Compiled', {exact:true}).waitFor();
    const canvases = page.locator('.pdf-pages canvas');
    assert.equal(await canvases.count(), 100, 'Every page keeps its scroll and keyboard target');
    const rasterized = () => canvases.evaluateAll(elements => (elements as HTMLCanvasElement[])
      .flatMap((canvas, index) => canvas.dataset.renderScale ? [index + 1] : []));
    assert.ok((await rasterized()).includes(1));
    assert.equal(await canvases.last().evaluate((canvas: HTMLCanvasElement) => canvas.width), 0);

    const jump = async (number: number) => {
      await page.getByRole('button', {name:'Change compiled PDF page'}).click();
      const input = page.getByRole('textbox', {name:'Go to compiled PDF page'});
      await input.fill(String(number));
      await input.press('Enter');
      await page.waitForFunction(index => !!document.querySelectorAll<HTMLCanvasElement>('.pdf-pages canvas')[index]?.dataset.renderScale, number - 1);
    };
    await jump(100);
    assert.equal(await canvases.first().evaluate((canvas: HTMLCanvasElement) => canvas.width), 0);
    assert.ok(await canvases.last().evaluate((canvas: HTMLCanvasElement) => {
      const pixels = canvas.getContext('2d')!.getImageData(0, 0, canvas.width, canvas.height).data;
      return pixels.some((channel, index) => index % 4 === 0 && channel < 128 && pixels[index + 3] > 0);
    }), 'A jump to an unloaded page paints the document');
    assert.ok((await rasterized()).every(number => number >= 98));

    await jump(1);
    const first = canvases.first();
    assert.equal(await canvases.last().evaluate((canvas: HTMLCanvasElement) => canvas.width), 0);
    assert.equal(await page.locator('.pdf-page-frame').first().locator('.pdf-text-layer').count(), 1);
    const fitWidth = await first.evaluate((canvas: HTMLCanvasElement) => canvas.width);
    await page.getByRole('button', {name:'Zoom in compiled PDF'}).click();
    await page.waitForFunction(width => (document.querySelector<HTMLCanvasElement>('.pdf-pages canvas')?.width ?? 0) > width, fitWidth);
    await page.locator('#zoom-fit').click();
    await page.waitForFunction(width => document.querySelector<HTMLCanvasElement>('.pdf-pages canvas')?.width === width, fitWidth);

    await page.locator('#pane-preview').click();
    await page.waitForFunction(() => [...document.querySelectorAll<HTMLCanvasElement>('.pdf-pages canvas')].every(canvas => !canvas.width && !canvas.height));
    await page.locator('#pane-preview').click();
    await page.waitForFunction(() => !!document.querySelector('.pdf-pages canvas[data-render-scale]'));
    await page.locator('#home').click();
    assert.equal(await canvases.count(), 0);
    assert.deepEqual(errors, []);
  } finally { await browser.close(); }
});
