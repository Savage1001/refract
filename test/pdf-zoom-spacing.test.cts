import test = require('node:test');
import assert = require('node:assert/strict');
import fs = require('node:fs/promises');
import path = require('node:path');
import playwright = require('playwright');

test('PDF zoom values stay compact and readable across labels and display sizes', { timeout: 30000 }, async t => {
  const root = path.resolve(__dirname, '../..');
  const env = { ...process.env };
  delete env.ELECTRON_RUN_AS_NODE;
  const app = await playwright._electron.launch({ args: [path.join(root, 'build/test/fixtures/picker-host.cjs')], env });
  t.after(() => app.close());
  const page = await app.firstWindow();
  const renderer = await fs.readFile(path.join(root, 'renderer.ts'), 'utf8');
  const controls = renderer.match(/pdfControls\.innerHTML='([^']+)'/)?.[1];
  assert.ok(controls, 'Use the actual PDF toolbar markup');
  await page.setContent(`<div class="pdf-pane" style="width:100%"><header><button>Compile</button><span></span><div class="pdf-controls">${controls}<div class="pdf-fit-controls"><button>Fit width</button><button>Fit page</button></div></div></header></div>`);
  await page.addStyleTag({ path: path.join(root, 'build/style.css') });
  for (const width of [420, 720, 1200]) {
    await app.evaluate(({ BrowserWindow }, width) => BrowserWindow.getAllWindows()[0]!.setSize(width, 900), width);
    for (const scale of [0.8, 1, 1.5, 2]) {
      await app.evaluate(({ BrowserWindow }, scale) => BrowserWindow.getAllWindows()[0]!.webContents.setZoomFactor(scale), scale);
      for (const label of ['Fit', '25%', '50%', '73%', '100%', '125%', '250%']) {
        const geometry = await page.locator('.pdf-zoom-value').evaluate((element, label) => {
          element.textContent = label;
          const range = document.createRange();
          range.selectNodeContents(element);
          const text = range.getBoundingClientRect();
          const value = element.getBoundingClientRect();
          const previous = element.previousElementSibling!.getBoundingClientRect();
          const next = element.nextElementSibling!.getBoundingClientRect();
          return {
            extraWidth: value.width - text.width,
            leftGap: text.left - previous.right,
            rightGap: next.left - text.right,
            previousWidth: previous.width,
            nextWidth: next.width,
            textWidth: text.width,
          };
        }, label);
        const context = `${label}, width ${width}, scale ${scale}: ${JSON.stringify(geometry)}`;
        assert.ok(geometry.textWidth > 0, context);
        assert.ok(geometry.extraWidth >= 3 && geometry.extraWidth <= 5, context);
        assert.ok(geometry.leftGap >= 3 && geometry.leftGap <= 6, context);
        assert.ok(geometry.rightGap >= 3 && geometry.rightGap <= 6, context);
        assert.ok(geometry.previousWidth >= 27.9 && geometry.nextWidth >= 27.9, context);
      }
    }
  }
});
