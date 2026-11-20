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

test('rapid pane toggles settle in order and never replace live editor state', async () => {
  const {browser, page, errors} = await fixture.openHeadlessUi();
  try {
    await openProject(page);
    const source = page.locator('#source');
    const original = await source.inputValue();
    await source.evaluate((editor: HTMLTextAreaElement) => {
      editor.setSelectionRange(2, 6);
      const button = document.querySelector<HTMLButtonElement>('#pane-agent')!;
      button.click();
      button.click();
    });
    await page.waitForFunction(() => document.querySelector('#pane-agent')?.getAttribute('aria-pressed') === 'true');
    assert.equal(await source.inputValue(), original);
    assert.deepEqual(await source.evaluate((editor: HTMLTextAreaElement) => [editor.selectionStart, editor.selectionEnd]), [2, 6]);
    const alive = await page.locator('.workspace-shell').evaluate(async shell => {
      document.querySelector<HTMLButtonElement>('#pane-preview')!.click();
      // Wait one paint for the update and another for the snapshot animation.
      for (let frame = 0; frame < 4; frame++) {
        await new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
        if (shell.matches(':active-view-transition')) return true;
      }
      return false;
    });
    assert.equal(alive, true);
    await page.locator('#pane-preview[aria-pressed=false]').waitFor();
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
        transitioning:!!document.querySelector(':active-view-transition'),
        sidebar:document.querySelector('.sidebar')?.getAnimations().length};
    });
    assert.deepEqual(states, {pane:'false', transitioning:false, sidebar:0});
    assert.deepEqual(errors, []);
  } finally {
    await browser.close();
  }
});
