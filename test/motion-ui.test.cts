import test = require('node:test');
import assert = require('node:assert/strict');
import fixture = require('./fixtures/headless-ui.cjs');
import type { Page } from 'playwright';

const project = '#list .project-open';

async function openProject(page: Page): Promise<void> {
  await page.locator(project).first().click();
  await page.locator('#source').waitFor();
}

test('overlay fades leave the app immediately interactive and preserve focus on reversal', async () => {
  const {browser, page, errors} = await fixture.openHeadlessUi();
  try {
    await page.locator('#settings').click();
    await page.locator('#settings-dialog[open]').waitFor();
    const states = await page.evaluate(async () => {
      const dialog = document.querySelector<HTMLDialogElement>('#settings-dialog')!;
      await Promise.allSettled(dialog.getAnimations().map(animation => animation.finished));
      dialog.close();
      const style = getComputedStyle(dialog);
      return {open: dialog.open, display: style.display, pointer: style.pointerEvents,
        focusRestored: document.activeElement?.id === 'settings',
        fading: dialog.getAnimations().some(animation => animation instanceof CSSTransition && animation.transitionProperty === 'opacity')};
    });
    assert.deepEqual(states, {open:false, display:'block', pointer:'none', focusRestored:true, fading:true});
    await page.locator('#settings').click();
    await page.locator('#settings-dialog[open]').waitFor();
    await page.keyboard.press('Escape');
    await openProject(page);

    const trigger = page.locator('.model-trigger');
    const picker = page.locator('.model-panel');
    await trigger.click();
    await page.getByRole('searchbox', {name:'Search models'}).waitFor();
    await picker.evaluate(async panel => {
      await Promise.allSettled(panel.getAnimations().map(animation => animation.finished));
    });
    await page.keyboard.press('Escape');
    assert.equal(await trigger.getAttribute('aria-expanded'), 'false');
    assert.equal(await trigger.evaluate(button => document.activeElement === button), true);
    assert.equal(await picker.evaluate(panel => getComputedStyle(panel).pointerEvents), 'none');
    await trigger.click();
    await page.getByRole('searchbox', {name:'Search models'}).waitFor();
    assert.equal(await trigger.getAttribute('aria-expanded'), 'true');
    assert.deepEqual(errors, []);
  } finally {
    await browser.close();
  }
});

test('rapid layout changes apply immediately and leave live pointer targets available', async () => {
  const {browser, page, errors} = await fixture.openHeadlessUi();
  try {
    await page.locator(project).first().waitFor();
    const cards = await page.evaluate(() => {
      const grid = document.querySelector<HTMLButtonElement>('#grid-view')!;
      const list = document.querySelector<HTMLButtonElement>('#list-view')!;
      grid.click();
      list.click();
      const reversed = list.getAttribute('aria-pressed') === 'true';
      grid.click();
      const root = document.querySelector('#list')!;
      const hit = Array.from(root.querySelectorAll('.project-open'), card => {
        const {left, top, width, height} = card.getBoundingClientRect();
        return card.contains(document.elementFromPoint(left + width / 2, top + height / 2));
      });
      return {reversed, hit, fading: root.getAnimations().some(animation => animation.id === 'layout-fade')};
    });
    assert.deepEqual(cards, {reversed:true, hit:[true, true, true], fading:true});
    await openProject(page);
    const source = page.locator('#source');
    const original = await source.inputValue();
    const layout = await source.evaluate((editor: HTMLTextAreaElement) => {
      editor.setSelectionRange(2, 6);
      const button = document.querySelector<HTMLButtonElement>('#pane-agent')!;
      button.click();
      button.click();
      document.querySelector<HTMLButtonElement>('#pane-preview')!.click();
      const shell = document.querySelector('.workspace-shell')!;
      const {right, top} = editor.getBoundingClientRect();
      const x = right - 28, y = top + 24;
      return {x, y, reversed: button.getAttribute('aria-pressed') === 'true',
        hitsEditor: document.elementFromPoint(x, y) === editor,
        fading: shell.getAnimations().some(animation => animation.id === 'layout-fade')};
    });
    assert.equal(layout.reversed, true);
    assert.equal(layout.hitsEditor, true);
    assert.equal(layout.fading, true);
    assert.equal(await source.inputValue(), original);
    assert.deepEqual(await source.evaluate((editor: HTMLTextAreaElement) => [editor.selectionStart, editor.selectionEnd]), [2, 6]);
    await page.mouse.click(layout.x, layout.y);
    assert.equal(await source.evaluate(editor => editor === document.activeElement), true);
    await page.evaluate(() => {
      document.querySelector<HTMLButtonElement>('#tools')!.click();
      document.querySelector<HTMLButtonElement>('#home')!.click();
    });
    await page.locator(project).nth(1).click();
    await source.waitFor();
    assert.equal(await page.locator('#tools').getAttribute('aria-expanded'), 'false');
    assert.equal(await page.locator('#tools-panel').isHidden(), true);
    assert.deepEqual(errors, []);
  } finally {
    await browser.close();
  }
});

test('reduced motion skips pane, overlay and control animations', async () => {
  const {browser, page, errors} = await fixture.openHeadlessUi('dark', undefined, {reducedMotion:'reduce'});
  try {
    assert.equal(await page.locator('#settings').evaluate(button => getComputedStyle(button).transitionDuration), '0s');
    await page.locator('#settings').click();
    assert.equal(await page.locator('#settings-dialog').evaluate(dialog => dialog.getAnimations().length), 0);
    await page.keyboard.press('Escape');
    await openProject(page);
    const states = await page.evaluate(() => {
      document.querySelector<HTMLButtonElement>('#pane-agent')!.click();
      document.querySelector<HTMLButtonElement>('#collapse')!.click();
      return {pane:document.querySelector('#pane-agent')?.getAttribute('aria-pressed'),
        transitioning:document.getAnimations().some(animation => animation.id === 'layout-fade'),
        sidebar:document.querySelector('.sidebar')?.getAnimations().length};
    });
    assert.deepEqual(states, {pane:'false', transitioning:false, sidebar:0});
    assert.deepEqual(errors, []);
  } finally {
    await browser.close();
  }
});

test('Find and the composer show keyboard focus immediately and motion can be disabled mid-flight', async () => {
  const {browser, page, errors} = await fixture.openHeadlessUi();
  try {
    await openProject(page);
    const source = page.locator('#source');
    await source.focus();
    await page.keyboard.press('Control+f');
    const find = page.getByRole('textbox', {name:'Find in file'});
    assert.deepEqual(await find.evaluate(input => ({
      focused: input === document.activeElement,
      barOpacity: getComputedStyle(input.closest('.editor-find')!).opacity,
      outline: getComputedStyle(input).outlineStyle
    })), {focused:true, barOpacity:'1', outline:'solid'});
    await page.keyboard.press('Escape');
    assert.equal(await source.evaluate(editor => editor === document.activeElement), true);
    assert.equal(await page.locator('.editor-find').evaluate(bar => getComputedStyle(bar).pointerEvents), 'none');
    await page.keyboard.press('Control+f');
    await find.waitFor();
    assert.equal(await find.evaluate(input => input === document.activeElement), true);
    await page.keyboard.press('Escape');
    await page.locator('#chat-input').focus();
    assert.match(await page.locator('.composer').evaluate(composer => getComputedStyle(composer).boxShadow), /27, 78, 216/);
    await page.locator('#pane-preview').evaluate((button: HTMLButtonElement) => button.click());
    assert.equal(await page.locator('.workspace-shell').evaluate(shell =>
      shell.getAnimations().some(animation => animation.id === 'layout-fade')), true);
    await page.emulateMedia({reducedMotion:'reduce'});
    await page.waitForFunction(() => !document.getAnimations().some(animation => animation.id === 'layout-fade'));
    assert.equal(await page.locator('.workspace-shell').evaluate(shell => getComputedStyle(shell).opacity), '1');
    assert.equal(await page.locator('#pane-preview').getAttribute('aria-pressed'), 'false');
    assert.deepEqual(errors, []);
  } finally {
    await browser.close();
  }
});

test('sidebar motion does not repeatedly lay out a 24-page PDF', {timeout:30000}, async context => {
  const {browser, page, errors} = await fixture.openHeadlessUi('light', undefined, {pageCount:24});
  try {
    await openProject(page);
    await page.waitForFunction(() => document.querySelectorAll('.pdf-pages canvas').length === 24);
    const client = await page.context().newCDPSession(page);
    await client.send('Performance.enable');
    const layoutCount = async () => (await client.send('Performance.getMetrics')).metrics
      .find(metric => metric.name === 'LayoutCount')!.value;
    const cycle = async () => {
      const before = await layoutCount();
      const frames = await page.evaluate(async () => {
        const sidebar = document.querySelector('.sidebar')!;
        const button = document.querySelector<HTMLButtonElement>('#collapse')!;
        const monitor = async () => {
          const frameTimes: number[] = [];
          let previous = performance.now();
          const start = previous;
          while (previous - start < 240) {
            const next = await new Promise<number>(resolve => requestAnimationFrame(resolve));
            frameTimes.push(next - previous);
            previous = next;
          }
          return frameTimes;
        };
        button.click();
        const collapsed = sidebar.clientWidth;
        const collapseFrames = await monitor();
        button.click();
        const expanded = sidebar.clientWidth;
        const expandFrames = await monitor();
        const content = document.querySelector('#project-side')!;
        return {collapsed, expanded, overflow:content.scrollWidth > content.clientWidth,
          longestFrame: Math.round(Math.max(...collapseFrames, ...expandFrames))};
      });
      return {...frames, layouts:(await layoutCount()) - before};
    };
    const animated = await cycle();
    await page.emulateMedia({reducedMotion:'reduce'});
    const reduced = await cycle();
    context.diagnostic(`24-page PDF, collapse + expand: ${animated.layouts} animated layouts / ${reduced.layouts} reduced-motion layouts. Longest sampled frames: ${animated.longestFrame} ms / ${reduced.longestFrame} ms.`);
    assert.ok(animated.expanded > animated.collapsed);
    assert.equal(animated.overflow, false);
    assert.ok(animated.layouts <= reduced.layouts + 6, 'Motion must not drive renderer layout on every frame');
    assert.deepEqual(errors, []);
  } finally {
    await browser.close();
  }
});
