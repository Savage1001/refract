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
    await page.locator('#settings').focus();
    await page.keyboard.press('Enter');
    await page.locator('#settings-dialog[open]').waitFor();
    const states = await page.evaluate(async () => {
      const dialog = document.querySelector<HTMLDialogElement>('#settings-dialog')!;
      const opacityOnOpen = Number(getComputedStyle(dialog).opacity);
      const focusedOnOpen = dialog.contains(document.activeElement);
      await Promise.allSettled(dialog.getAnimations().map(animation => animation.finished));
      dialog.close();
      const style = getComputedStyle(dialog);
      return {open: dialog.open, display: style.display, pointer: style.pointerEvents,
        focusRestored: document.activeElement?.id === 'settings', opacityOnOpen, focusedOnOpen,
        fading: dialog.getAnimations().some(animation => animation instanceof CSSTransition && animation.transitionProperty === 'opacity')};
    });
    const {opacityOnOpen, ...dialogState} = states;
    assert.ok(opacityOnOpen >= .77);
    assert.deepEqual(dialogState, {open:false, display:'block', pointer:'none', focusRestored:true, focusedOnOpen:true, fading:true});
    await page.locator('#settings').click();
    await page.locator('#settings-dialog[open]').waitFor();
    await page.keyboard.press('Escape');
    await openProject(page);

    const trigger = page.locator('.model-trigger');
    const picker = page.locator('.model-panel');
    await trigger.focus();
    await page.keyboard.press('Enter');
    const search = page.getByRole('searchbox', {name:'Search models'});
    assert.deepEqual(await search.evaluate(input => {
      const outline = getComputedStyle(input.closest('.model-search')!);
      return {focused:input === document.activeElement, visible:outline.outlineStyle === 'solid' && outline.outlineWidth === '2px'};
    }), {focused:true, visible:true});
    assert.ok(await picker.evaluate(panel => Number(getComputedStyle(panel).opacity) >= .77));
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
      return {reversed, hit, fading: root.getAnimations().some(animation => animation.id === 'refract-motion')};
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
        fading: shell.getAnimations().some(animation => animation.id === 'refract-motion')};
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
        transitioning:document.getAnimations().some(animation => animation.id === 'refract-motion'),
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
    const editorTop = () => source.evaluate(editor => editor.getBoundingClientRect().top);
    const withFind = await editorTop();
    await page.keyboard.press('Escape');
    const afterClose = await editorTop();
    assert.ok(afterClose < withFind, 'Closing Find must immediately release its layout space');
    assert.equal(await source.evaluate(editor => editor === document.activeElement), true);
    assert.equal(await page.locator('.editor-find').evaluate(bar => getComputedStyle(bar).pointerEvents), 'none');
    await page.locator('.editor-find').waitFor({state:'hidden'});
    assert.equal(await editorTop(), afterClose, 'The editor must stay in place when the exit finishes');
    await page.keyboard.press('Control+f');
    await page.keyboard.press('Escape');
    await page.keyboard.press('Control+f');
    await find.waitFor();
    assert.equal(await find.evaluate(input => input === document.activeElement), true);
    await page.keyboard.press('Escape');
    await page.locator('#chat-input').focus();
    assert.match(await page.locator('.composer').evaluate(composer => getComputedStyle(composer).boxShadow), /27, 78, 216/);
    await page.locator('#pane-preview').evaluate((button: HTMLButtonElement) => button.click());
    assert.equal(await page.locator('.workspace-shell').evaluate(shell =>
      shell.getAnimations().some(animation => animation.id === 'refract-motion')), true);
    await page.emulateMedia({reducedMotion:'reduce'});
    await page.waitForFunction(() => !document.getAnimations().some(animation => animation.id === 'refract-motion'));
    assert.equal(await page.locator('.workspace-shell').evaluate(shell => getComputedStyle(shell).opacity), '1');
    assert.equal(await page.locator('#pane-preview').getAttribute('aria-pressed'), 'false');
    assert.deepEqual(errors, []);
  } finally {
    await browser.close();
  }
});

test('folder keyboard order and sidebar exit hit testing match the live layout', async () => {
  const {browser, page, errors} = await fixture.openHeadlessUi();
  try {
    await openProject(page);
    const folder = page.locator('#file-list summary').first();
    if (await folder.evaluate(summary => (summary.parentElement as HTMLDetailsElement).open)) await folder.press('Enter');
    await folder.press('Enter');
    await page.keyboard.press('Tab');
    assert.equal(await page.locator('#file-list button[title="sections/abstract.tex"]')
      .evaluate(file => file === document.activeElement), true);
    const outline = page.locator('#outline-list');
    await page.locator('#outline-toggle').click();
    assert.equal(await outline.evaluate(list => (list as HTMLElement).hidden && getComputedStyle(list).pointerEvents === 'none'), true);
    await page.locator('#outline-toggle').click();
    assert.equal(await outline.isVisible(), true);
    const files = await page.evaluate(() => {
      const list = document.getElementById('file-list')!;
      const before = list.getBoundingClientRect();
      document.getElementById('chats-tab')!.click();
      const after = list.getBoundingClientRect();
      const chats = document.getElementById('chat-list')!.getBoundingClientRect();
      return {before:before.toJSON(), after:after.toJSON(), inert:getComputedStyle(list).getPropertyValue('interactivity'),
        hidden:list.hidden, chatsVisible:chats.height > 0};
    });
    assert.deepEqual(files.after, files.before, 'Outgoing file controls retain their painted bounds only');
    assert.equal(files.hidden, true);
    assert.equal(files.inert, 'inert');
    assert.equal(files.chatsVisible, true);
    const reversal = await page.evaluate(() => {
      const files = document.getElementById('file-list')!;
      const chats = document.getElementById('chat-list')!;
      for (const item of [files, chats]) {
        item.getAnimations().find(animation => animation.id === 'refract-motion')!.currentTime = 45;
      }
      const before = [files,chats].map(item => Number(getComputedStyle(item).opacity));
      document.getElementById('files-tab')!.click();
      const after = [files,chats].map(item => {
        const effect = item.getAnimations().find(animation => animation.id === 'refract-motion')!.effect as KeyframeEffect;
        return Number(effect.getKeyframes()[0].opacity);
      });
      return {before,after};
    });
    assert.deepEqual(reversal.after, reversal.before);
    assert.ok(reversal.before[0] < .9);
    assert.equal(await page.locator('#file-list').isVisible(), true);
    assert.deepEqual(errors, []);
  } finally {
    await browser.close();
  }
});

test('a pane change takes over an unfinished PDF reveal and leaves one live fade', {timeout:15000}, async () => {
  const {browser, page, errors} = await fixture.openHeadlessUi();
  try {
    await page.locator(project).first().waitFor();
    const reveal = await page.evaluate(() => new Promise<{initial:number; reversed:number[]}>(resolve => {
      const animations = (selector: string) => document.querySelector(selector)!.getAnimations()
        .filter(animation => animation.id === 'refract-motion').length;
      const observer = new MutationObserver(() => {
        if (!document.querySelector('.pdf-pages')) return;
        observer.disconnect();
        const initial = animations('.pdf-pages');
        const preview = document.getElementById('pane-preview')!;
        preview.click();
        preview.click();
        resolve({initial, reversed:['.workspace-shell','#pdf-content','.pdf-pages'].map(animations)});
      });
      observer.observe(document.getElementById('pdf-area')!, {childList:true});
      document.querySelector<HTMLButtonElement>('#list .project-open')!.click();
    }));
    assert.deepEqual(reveal, {initial:1, reversed:[1,0,0]});
    assert.equal(await page.locator('.pdf-pages').isVisible(), true);
    assert.deepEqual(errors, []);
  } finally {
    await browser.close();
  }
});

test('Tools tabs give the new body a single entry fade and retain keyboard focus', {timeout:15000}, async () => {
  const {browser, page, errors} = await fixture.openHeadlessUi();
  try {
    await openProject(page);
    await page.locator('#tools').click();
    const result = await page.evaluate(() => new Promise<{pressed:string|null; focused:boolean; animations:number[]}>(resolve => {
      const body = document.getElementById('tool-body')!;
      const tab = document.getElementById('tool-comments')!;
      const observer = new MutationObserver(() => {
        if (!body.querySelector('.comment-card')) return;
        observer.disconnect();
        const animations = ['.workspace-shell','#tools-panel','#tool-body'].map(selector =>
          document.querySelector(selector)!.getAnimations().filter(animation => animation.id === 'refract-motion').length);
        resolve({pressed:tab.getAttribute('aria-pressed'), focused:document.activeElement === tab, animations});
      });
      observer.observe(body, {childList:true});
      tab.focus();
      tab.click();
    }));
    assert.deepEqual(result, {pressed:'true', focused:true, animations:[0,0,1]});
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
    await page.locator('.pdf-pages').evaluate(async pages => {
      await Promise.allSettled(pages.getAnimations().map(animation => animation.finished));
    });
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
        const edge = sidebar.querySelector('.sidebar-motion-edge')!;
        const oldEdge = edge.getBoundingClientRect().left;
        button.click();
        const collapsed = sidebar.clientWidth;
        const movingEdge = edge.getAnimations().some(animation => animation.id === 'refract-motion');
        const startingEdge = edge.getBoundingClientRect().left;
        const collapseFrames = await monitor();
        button.click();
        const expanded = sidebar.clientWidth;
        const expandFrames = await monitor();
        const content = document.querySelector('#project-side')!;
        return {collapsed, expanded, movingEdge, edgeStartsAtOldSide:Math.abs(startingEdge - oldEdge) < 3,
          overflow:content.scrollWidth > content.clientWidth,
          longestFrame: Math.round(Math.max(...collapseFrames, ...expandFrames))};
      });
      return {...frames, layouts:(await layoutCount()) - before};
    };
    const toggleFolders = async () => {
      const before = await layoutCount();
      await page.locator('#file-list summary').first().evaluate(async (folder: HTMLElement) => {
        for (let toggle = 0; toggle < 8; toggle++) {
          folder.click();
          await Promise.allSettled((folder.parentElement?.getAnimations({subtree:true}) ?? [])
            .map(animation => animation.finished));
          await new Promise<number>(resolve => requestAnimationFrame(resolve));
        }
      });
      return (await layoutCount()) - before;
    };
    const animated = await cycle();
    const animatedFolders = await toggleFolders();
    await page.emulateMedia({reducedMotion:'reduce'});
    const reduced = await cycle();
    const reducedFolders = await toggleFolders();
    context.diagnostic(`24-page PDF, collapse + expand: ${animated.layouts} animated layouts / ${reduced.layouts} reduced-motion layouts. Longest sampled frames: ${animated.longestFrame} ms / ${reduced.longestFrame} ms.`);
    assert.ok(animated.expanded > animated.collapsed);
    assert.equal(animated.movingEdge, true);
    assert.equal(animated.edgeStartsAtOldSide, true);
    assert.equal(animated.overflow, false);
    assert.ok(animated.layouts <= reduced.layouts + 6, 'Motion must not drive renderer layout on every frame');
    context.diagnostic(`Eight folder toggles: ${animatedFolders} animated layouts / ${reducedFolders} reduced-motion layouts.`);
    assert.ok(animatedFolders <= reducedFolders + 6, 'Folder motion must leave native disclosure layout unanimated');
    assert.deepEqual(errors, []);
  } finally {
    await browser.close();
  }
});
