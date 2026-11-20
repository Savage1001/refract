import test = require('node:test');
import assert = require('node:assert/strict');
import fixture = require('./fixtures/headless-ui.cjs');
import type { Page } from 'playwright';

const project = '#list .project-open';

async function openProject(page: Page): Promise<void> {
  await page.locator(project).first().click();
  await page.locator('#source').waitFor();
}

test('press feedback keeps a stationary edge click on its button', {timeout:15000}, async () => {
  const {browser, page, errors} = await fixture.openHeadlessUi();
  try {
    const button = page.locator('#new');
    const bounds = (await button.boundingBox())!;
    const x = bounds.x + .5, y = bounds.y + bounds.height / 2;
    await page.mouse.move(x, y);
    await page.mouse.down();
    await page.waitForFunction(() => getComputedStyle(document.getElementById('new')!).filter !== 'none');
    await page.waitForTimeout(155);
    assert.equal(await page.evaluate(({x,y}) => {
      const button = document.getElementById('new')!;
      return button.contains(document.elementFromPoint(x, y));
    }, {x,y}), true);
    await page.mouse.up();
    await page.locator('#source').waitFor();
    assert.deepEqual(errors, []);
  } finally {
    await browser.close();
  }
});

test('keyboard project navigation leaves the new focus indicator fully opaque', async () => {
  const {browser, page, errors} = await fixture.openHeadlessUi();
  try {
    const card = page.locator(project).first();
    await card.focus();
    await page.keyboard.press('Enter');
    const name = page.locator('#project-name');
    await name.waitFor();
    assert.deepEqual(await name.evaluate(button => ({
      focused:button === document.activeElement && button.matches(':focus-visible'),
      opacity:getComputedStyle(button.closest('.workspace-topbar')!).opacity
    })), {focused:true, opacity:'1'});
    await page.locator('#home').focus();
    await page.keyboard.press('Enter');
    await card.waitFor();
    assert.deepEqual(await card.evaluate(button => ({
      focused:button === document.activeElement && button.matches(':focus-visible'),
      opacity:getComputedStyle(button.closest('#projects')!).opacity
    })), {focused:true, opacity:'1'});
    assert.deepEqual(errors, []);
  } finally {
    await browser.close();
  }
});

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
    assert.equal(opacityOnOpen, 1);
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
    assert.equal(await picker.evaluate(panel => getComputedStyle(panel).opacity), '1');
    await picker.evaluate(async panel => {
      await Promise.allSettled(panel.getAnimations().map(animation => animation.finished));
    });
    await page.keyboard.press('Escape');
    assert.equal(await trigger.getAttribute('aria-expanded'), 'false');
    assert.equal(await trigger.evaluate(button => document.activeElement === button), true);
    assert.equal(await picker.evaluate(panel => getComputedStyle(panel).pointerEvents), 'none');
    const exiting = await picker.evaluate(panel => {
      for (const animation of panel.getAnimations()) {
        animation.pause();
        animation.currentTime = 140;
      }
      return Number(getComputedStyle(panel).opacity);
    });
    assert.ok(exiting < .25, 'The picker must be close to transparent before reversing');
    await trigger.press('Enter');
    await search.waitFor();
    assert.equal(await picker.evaluate(panel => getComputedStyle(panel).opacity), '1');
    assert.equal(await search.evaluate(input => input === document.activeElement), true);
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
      const shell = document.querySelector('.workspace-shell')!;
      const preview = document.querySelector<HTMLButtonElement>('#pane-preview')!;
      preview.click();
      shell.getAnimations().find(animation => animation.id === 'refract-motion')!.currentTime = 125;
      const opacityBefore = Number(getComputedStyle(shell).opacity);
      preview.click();
      const effect = shell.getAnimations().find(animation => animation.id === 'refract-motion')!.effect as KeyframeEffect;
      const opacityAfter = Number(effect.getKeyframes()[0].opacity);
      preview.click();
      const {right, top} = editor.getBoundingClientRect();
      const x = right - 28, y = top + 24;
      return {x, y, opacityBefore, opacityAfter, reversed: button.getAttribute('aria-pressed') === 'true',
        hitsEditor: document.elementFromPoint(x, y) === editor,
        fading: shell.getAnimations().some(animation => animation.id === 'refract-motion')};
    });
    assert.equal(layout.reversed, true);
    assert.equal(layout.hitsEditor, true);
    assert.equal(layout.fading, true);
    assert.ok(layout.opacityBefore > .95);
    assert.equal(layout.opacityAfter, layout.opacityBefore, 'Reversing a late fade must preserve the sampled opacity');
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
    const box = (await find.boundingBox())!;
    await page.keyboard.press('Escape');
    const afterClose = await editorTop();
    assert.ok(afterClose < withFind, 'Closing Find must immediately release its layout space');
    assert.equal(await source.evaluate(editor => editor === document.activeElement), true);
    const exposed = await page.evaluate(({x,y}) => ({
      findDisplay:getComputedStyle(document.querySelector('.editor-find')!).display,
      liveTarget:document.elementFromPoint(x,y)?.id,
      exitEffect:document.querySelector('.find-exit-edge')!.getAnimations().some(animation => animation.id === 'refract-motion')
    }), {x:box.x + box.width / 2,y:box.y + box.height / 2});
    assert.deepEqual(exposed, {findDisplay:'none',liveTarget:'source',exitEffect:true});
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
    await page.locator('#pane-preview').evaluate((button: HTMLButtonElement) => {
      button.focus();
      button.click();
    });
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

test('folder keyboard order and sidebar exit hit testing match the live layout', {timeout:20000}, async () => {
  const {browser, page, errors} = await fixture.openHeadlessUi();
  try {
    await page.evaluate(() => {
      const state = (window as unknown as {fixture:{files:Record<string,string>}}).fixture;
      state.files['z-last.tex'] = '% A file after the folder';
      state.files['z-next.tex'] = '% The next moving sibling';
      for (let index = 0; index < 4; index++) state.files[`sections/notes-${index}.tex`] = '% Nested notes';
    });
    await openProject(page);
    const folder = page.locator('#file-list summary').first();
    const exit = await folder.evaluate((summary: HTMLElement) => {
      const details = summary.parentElement as HTMLDetailsElement;
      if (!details.open) summary.click();
      getComputedStyle(details, '::details-content').clipPath;
      const next = document.querySelector<HTMLButtonElement>('#file-list button[title="z-last.tex"]')!;
      const oldTop = next.getBoundingClientRect().top;
      summary.click();
      const style = getComputedStyle(details, '::details-content');
      const {left,top,width,height} = next.getBoundingClientRect();
      const state = {position:style.position, pointer:style.pointerEvents,
        paintingExit:!details.open && style.contentVisibility === 'visible' && style.clipPath !== 'inset(0px 0px 100%)',
        continuousTop:Math.abs(oldTop-top) < 1,
        siblingMoving:next.parentElement!.getAnimations().some(animation => animation.id === 'refract-motion'),
        liveFileOwnsPointer:next.contains(document.elementFromPoint(left+width/2, top+height/2)),
        liveFileHasBacking:getComputedStyle(next.parentElement!).backgroundColor !== 'rgba(0, 0, 0, 0)'};
      // Reopen during an observable mid-fold frame, independently of test runner timing.
      for (const animation of document.getElementById('file-list')!.getAnimations({subtree:true})) {
        if (animation.id !== 'refract-motion') continue;
        animation.pause();
        animation.currentTime = 50;
      }
      return state;
    });
    assert.deepEqual(exit, {position:'absolute', pointer:'none', paintingExit:true, continuousTop:true,
      siblingMoving:true, liveFileOwnsPointer:true, liveFileHasBacking:true});
    await folder.press('Enter');
    await page.keyboard.press('Tab');
    assert.deepEqual(await page.locator('#file-list button[title="sections/abstract.tex"]').evaluate(file => {
      const {left,top,width,height} = file.getBoundingClientRect();
      return {focused:file === document.activeElement, opacity:getComputedStyle(file.closest('details')!, '::details-content').opacity,
        ownsPointer:file.contains(document.elementFromPoint(left+width/2, top+height/2)),
        siblingSettled:document.querySelector('#file-list button[title="z-last.tex"]')!.parentElement!
          .getAnimations().every(animation => animation.id !== 'refract-motion')};
    }), {focused:true, opacity:'1', ownsPointer:true, siblingSettled:true});
    await page.keyboard.press('Tab');
    const action = page.getByRole('button', {name:'Actions for sections/abstract.tex',exact:true});
    assert.deepEqual(await action.evaluate(button => ({
      focused:button === document.activeElement, opacity:getComputedStyle(button).opacity
    })), {focused:true, opacity:'1'});
    await page.locator('#file-list button[title="main.tex"]').click();
    await page.locator('#outline-list button').first().waitFor();
    const outline = page.locator('#outline-list');
    await page.locator('#outline-toggle').click();
    assert.deepEqual(await outline.evaluate(list => {
      const toggle = document.getElementById('outline-toggle')!;
      const {left,top,width,height} = toggle.getBoundingClientRect();
      return {hidden:(list as HTMLElement).hidden, display:getComputedStyle(list).display,
        rowRectangles:list.querySelector('button')!.getClientRects().length,
        ownsPointer:toggle.contains(document.elementFromPoint(left+width/2, top+height/2))};
    }), {hidden:true, display:'none', rowRectangles:0, ownsPointer:true});
    await page.locator('#outline-toggle').click();
    assert.equal(await outline.isVisible(), true);
    const files = await page.evaluate(() => {
      const files = document.getElementById('files-surface')!;
      const chats = document.getElementById('chat-list')!;
      document.getElementById('chats-tab')!.click();
      const button = chats.querySelector('button')!;
      const {left,top,width,height} = button.getBoundingClientRect();
      const hit = document.elementFromPoint(left + width / 2, top + height / 2);
      return {filesDisplayed:getComputedStyle(files).display, chatsVisible:height > 0, chatOwnsPointer:button.contains(hit)};
    });
    assert.deepEqual(files, {filesDisplayed:'none', chatsVisible:true, chatOwnsPointer:true});
    const reversal = await page.evaluate(() => {
      const surface = document.getElementById('sidebar-content')!;
      const fade = () => surface.getAnimations().find(animation => animation.id === 'refract-motion')!;
      fade().currentTime = 45;
      const before = Number(getComputedStyle(surface).opacity);
      document.getElementById('files-tab')!.click();
      const effect = fade().effect as KeyframeEffect;
      return {before,after:Number(effect.getKeyframes()[0].opacity)};
    });
    assert.equal(reversal.after, reversal.before);
    assert.ok(reversal.before > .8 && reversal.before < 1);
    assert.equal(await page.locator('#file-list').isVisible(), true);
    assert.deepEqual(errors, []);
  } finally {
    await browser.close();
  }
});

test('normal-speed file row edge presses keep the pointer-down frame and open the same file', {timeout:20000}, async () => {
  const {browser, page, errors} = await fixture.openHeadlessUi('dark', undefined, {viewport:{width:1040,height:720}});
  try {
    await page.evaluate(() => {
      const {files} = (window as unknown as {fixture:{files:Record<string,string>}}).fixture;
      files['z-first.tex'] = '% First moving file';
      files['z-second.tex'] = '% Second moving file';
      for (let index = 0; index < 5; index++) files[`sections/notes-${index}.tex`] = '% Nested notes';
    });
    await openProject(page);
    const folder = page.locator('#file-list summary').first();
    const point = await folder.evaluate(async (summary:HTMLElement) => {
      summary.click();
      const list = document.getElementById('file-list')!;
      const first = list.querySelector<HTMLButtonElement>('button[title="z-first.tex"]')!;
      const rows = list.getAnimations({subtree:true}).filter(animation => animation.id === 'refract-motion');
      // Seek into the closing motion so the press never depends on how soon the
      // CI runner delivers the first animation frame. Leave playback at 1×.
      for (const animation of rows) {
        animation.pause();
        animation.currentTime = 120;
      }
      await Promise.all(rows.map(animation => animation.ready));
      window.addEventListener('pointermove', () => {
        for (const animation of rows) animation.play();
      }, {capture:true, once:true});
      const read = (event:PointerEvent) => {
        if (!(event.target instanceof Element)) return;
        const file = event.target.closest<HTMLButtonElement>('button[data-path]');
        list.dataset[`${event.type}File`] = file?.dataset.path ?? '';
        list.dataset[`${event.type}Times`] = JSON.stringify(rows.map(animation => animation.currentTime));
        list.dataset[`${event.type}Speeds`] = JSON.stringify(rows.map(animation => animation.playbackRate));
      };
      // Window capture samples the live frame before the renderer's document listener.
      window.addEventListener('pointerdown', read, {capture:true, once:true});
      window.addEventListener('pointerup', read, {capture:true, once:true});
      const {left,width,bottom} = first.getBoundingClientRect();
      return {x:Math.floor(left + width / 2), y:Math.floor(bottom - 8)};
    });
    await page.mouse.click(point.x, point.y, {delay:50});
    const result = await page.locator('#file-list').evaluate(list => {
      const values = (list as HTMLElement).dataset;
      return {down:values.pointerdownFile, up:values.pointerupFile,
        downTimes:JSON.parse(values.pointerdownTimes ?? '[]') as number[],
        upTimes:JSON.parse(values.pointerupTimes ?? '[]') as number[],
        speeds:JSON.parse(values.pointerdownSpeeds ?? '[]') as number[]};
    });
    assert.equal(result.down, 'z-first.tex');
    assert.equal(result.up, result.down);
    assert.deepEqual(result.speeds, [1,1]);
    assert.ok(result.downTimes.every(time => time >= 120 && time < 200));
    assert.deepEqual(result.upTimes, result.downTimes, 'Both rows must freeze at their pointer-down frame');
    await page.waitForFunction(() => document.querySelector('#file-list button.active-file')?.getAttribute('data-path') === 'z-first.tex');
    assert.equal(await page.locator('#source').inputValue(), '% First moving file');
    assert.deepEqual(errors, []);
  } finally {
    await browser.close();
  }
});

test('keyboard focus stays fully visible as assistant replies and build disclosures appear', {timeout:20000}, async () => {
  const {browser, page, errors} = await fixture.openHeadlessUi();
  try {
    await page.evaluate(() => {
      const {store} = window as unknown as {store:{chatSend:()=>Promise<unknown>}};
      store.chatSend = async () => ({sessionId:null, messages:[
        {role:'user', text:'Where is the reference?'},
        {role:'assistant', text:'Read the [method reference](https://example.com/method).'}
      ]});
    });
    await openProject(page);
    // Keep the entrances observable while sending real keyboard events.
    await page.evaluate(() => {
      const history = document.getElementById('chat-messages')!;
      const observer = new MutationObserver(() => {
        const animation = history.querySelector('.assistant.message-arriving')?.getAnimations()[0];
        if (!animation) return;
        animation.pause();
        animation.currentTime = 0;
        observer.disconnect();
      });
      observer.observe(history, {childList:true});
    });
    const composer = page.locator('#chat-input');
    await composer.fill('Where is the reference?');
    await page.locator('#send').click();
    const message = page.locator('.message.assistant.message-arriving');
    const link = message.getByRole('link', {name:'method reference'});
    await link.waitFor({state:'attached'});
    assert.deepEqual(await message.evaluate(reply => {
      const {opacity,translate} = getComputedStyle(reply);
      return {opacity,translate};
    }), {opacity:'0', translate:'none'});
    await composer.press('Shift+Tab');
    assert.deepEqual(await link.evaluate(anchor => {
      const style = getComputedStyle(anchor.closest('.message')!);
      return {focused:anchor === document.activeElement && anchor.matches(':focus-visible'), opacity:style.opacity, translate:style.translate};
    }), {focused:true, opacity:'1', translate:'none'});

    await page.locator('#tools').click();
    const disclosure = page.locator('.raw-output summary');
    await disclosure.evaluate(summary => {
      const details = summary.closest('details')!;
      const observer = new MutationObserver(() => {
        const animation = details.getAnimations({subtree:true}).find(animation =>
          animation instanceof CSSAnimation && animation.animationName === 'motion-surface');
        if (!animation) return;
        animation.pause();
        animation.currentTime = 0;
        observer.disconnect();
      });
      observer.observe(details, {attributes:true,attributeFilter:['open']});
    });
    await disclosure.press('Enter');
    assert.ok(Number(await disclosure.evaluate(summary => getComputedStyle(summary.parentElement!, '::details-content').opacity)) < 1);
    await page.keyboard.press('Tab');
    assert.deepEqual(await page.getByRole('searchbox', {name:'Filter build output'}).evaluate(input => ({
      focused:input === document.activeElement && input.matches(':focus-visible'),
      disclosureOpacity:getComputedStyle(input.closest('details')!, '::details-content').opacity
    })), {focused:true, disclosureOpacity:'1'});
    assert.deepEqual(errors, []);
  } finally {
    await browser.close();
  }
});

test('Tools preserves fade continuity while keeping a keyboard-focused expansion control opaque', {timeout:15000}, async () => {
  const {browser, page, errors} = await fixture.openHeadlessUi();
  try {
    await openProject(page);
    await page.locator('#tools').click();
    const handoff = await page.evaluate(() => {
      const shell = document.querySelector('.workspace-shell')!;
      const fade = () => shell.getAnimations().find(animation => animation.id === 'refract-motion')!;
      document.querySelector<HTMLButtonElement>('.expand-tools')!.click();
      fade().currentTime = 125;
      const before = Number(getComputedStyle(shell).opacity);
      document.getElementById('tools')!.click();
      const effect = fade().effect as KeyframeEffect;
      return {before, after:Number(effect.getKeyframes()[0].opacity)};
    });
    assert.equal(handoff.after, handoff.before);
    await page.locator('#tools').click();
    const expand = page.getByRole('button', {name:'Expand tools panel',exact:true});
    await expand.focus();
    await page.keyboard.press('Enter');
    const control = page.getByRole('button', {name:'Collapse tools panel',exact:true});
    const focused = await control.evaluate(button => ({
      active: document.activeElement === button && button.matches(':focus-visible'),
      shellOpacity: getComputedStyle(button.closest('.workspace-shell')!).opacity,
      outline: getComputedStyle(button).outlineStyle
    }));
    assert.deepEqual(focused, {active:true, shellOpacity:'1', outline:'solid'});
    assert.deepEqual(errors, []);
  } finally {
    await browser.close();
  }
});

test('a pane change takes over an unfinished PDF reveal and leaves one live fade', {timeout:15000}, async () => {
  const {browser, page, errors} = await fixture.openHeadlessUi();
  try {
    await page.locator(project).first().waitFor();
    const reveal = await page.evaluate(() => new Promise<{initial:number; reversed:number[]; before:number; after:number}>(resolve => {
      const animations = (selector: string) => document.querySelector(selector)!.getAnimations()
        .filter(animation => animation.id === 'refract-motion').length;
      const observer = new MutationObserver(() => {
        const pdf = document.querySelector('.pdf-pages');
        if (!pdf) return;
        observer.disconnect();
        const initial = animations('.pdf-pages');
        const fade = pdf.getAnimations().find(animation => animation.id === 'refract-motion')!;
        fade.currentTime = 90;
        const before = Number(getComputedStyle(pdf).opacity);
        const preview = document.getElementById('pane-preview')!;
        preview.click();
        const shell = document.querySelector('.workspace-shell')!;
        const effect = shell.getAnimations().find(animation => animation.id === 'refract-motion')!.effect as KeyframeEffect;
        const after = Number(effect.getKeyframes()[0].opacity);
        preview.click();
        resolve({initial, before, after, reversed:['.workspace-shell','#pdf-content','.pdf-pages'].map(animations)});
      });
      observer.observe(document.getElementById('pdf-area')!, {childList:true});
      document.querySelector<HTMLButtonElement>('#list .project-open')!.click();
    }));
    assert.equal(reveal.initial, 1);
    assert.deepEqual(reveal.reversed, [1,0,0]);
    assert.equal(reveal.after, reveal.before);
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
    const raw = page.locator('.raw-output summary');
    await raw.click();
    await page.waitForFunction(() => document.querySelector('.workspace-shell')?.getAnimations()
      .some(animation => animation.id === 'refract-motion'));
    assert.equal(await page.locator('#workspace').evaluate(workspace => workspace.classList.contains('tools-expanded')), true);
    const outputExit = await raw.evaluate((summary: HTMLElement) => {
      getComputedStyle(summary.parentElement!, '::details-content').clipPath;
      summary.click();
      const style = getComputedStyle(summary.parentElement!, '::details-content');
      return {position:style.position, visible:style.contentVisibility === 'visible',
        controlsHidden:getComputedStyle(summary.parentElement!.querySelector('.log-controls')!).visibility === 'hidden',
        pointer:style.pointerEvents};
    });
    assert.deepEqual(outputExit, {position:'absolute', visible:true, controlsHidden:true, pointer:'none'});
    await page.waitForFunction(() => !document.querySelector('#workspace')?.classList.contains('tools-expanded'));
    await page.locator('.workspace-shell').evaluate(async shell => {
      await Promise.allSettled(shell.getAnimations().map(animation => animation.finished));
    });
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

test('a late sidebar reversal preserves the tint already on screen', async () => {
  const {browser, page, errors} = await fixture.openHeadlessUi();
  try {
    await openProject(page);
    const reversal = await page.evaluate(() => {
      const surface = document.querySelector('.sidebar-motion-surface')!;
      const edge = document.querySelector('.sidebar-motion-edge')!;
      const button = document.getElementById('collapse')!;
      const fade = () => surface.getAnimations().find(animation => animation.id === 'refract-motion')!;
      button.click();
      fade().currentTime = 140;
      edge.getAnimations().find(animation => animation.id === 'refract-motion')!.currentTime = 140;
      const before = Number(getComputedStyle(surface).opacity);
      button.click();
      const effect = fade().effect as KeyframeEffect;
      return {before,after:Number(effect.getKeyframes()[0].opacity)};
    });
    assert.ok(reversal.before < .05);
    assert.equal(reversal.after, reversal.before);
    assert.deepEqual(errors, []);
  } finally {
    await browser.close();
  }
});

test('a delayed Tools result leaves a newer layout fade in charge', {timeout:15000}, async () => {
  const {browser, page, errors} = await fixture.openHeadlessUi();
  try {
    await openProject(page);
    await page.locator('#tools').click();
    const layout = await page.evaluate(() => new Promise<{same:boolean; bodyMotion:boolean; opacity:number}>(resolve => {
      const w = window as unknown as {store:{commentList:(id:string) => Promise<unknown[]>}};
      const load = w.store.commentList;
      const shell = document.querySelector('.workspace-shell')!;
      const body = document.getElementById('tool-body')!;
      const getFade = (element: Element) => element.getAnimations().find(animation => animation.id === 'refract-motion');
      w.store.commentList = id => new Promise(resolveLoad => {
        queueMicrotask(async () => {
          const comments = await load(id);
          document.querySelector<HTMLButtonElement>('.expand-tools')!.click();
          const expansion = getFade(shell)!;
          expansion.currentTime = 125;
          const observer = new MutationObserver(() => {
            if (!body.querySelector('.comment-card')) return;
            observer.disconnect();
            resolve({same:expansion === getFade(shell), bodyMotion:!!getFade(body), opacity:Number(getComputedStyle(shell).opacity)});
          });
          observer.observe(body, {childList:true});
          resolveLoad(comments);
        });
      });
      document.getElementById('tool-comments')!.click();
    }));
    assert.equal(layout.same, true);
    assert.equal(layout.bodyMotion, false);
    assert.ok(layout.opacity > .95);
    assert.deepEqual(errors, []);
  } finally {
    await browser.close();
  }
});

test('large folder collapse leaves native layout and paints no retained rows', {timeout:30000}, async context => {
  const {browser, page, errors} = await fixture.openHeadlessUi();
  try {
    await page.evaluate(() => {
      const {files} = (window as unknown as {fixture:{files:Record<string,string>}}).fixture;
      for (let index = 0; index < 1400; index++) {
        const name = `z-${String(index).padStart(4, '0')}.tex`;
        files[`sections/${name}`] = '% Nested sample';
        files[name] = '% Following sample';
      }
    });
    await openProject(page);
    const folder = page.locator('#file-list summary').first();
    const collapse = () => folder.evaluate(async (summary:HTMLElement) => {
      const details = summary.closest('details')!;
      const following = document.querySelector('#file-list button[title="z-0000.tex"]')!;
      const start = await new Promise<number>(resolve => requestAnimationFrame(resolve));
      summary.click();
      const disclosure = getComputedStyle(details, '::details-content').contentVisibility;
      const {left,top,width,height} = following.getBoundingClientRect();
      const ownsPointer = following.contains(document.elementFromPoint(left+width/2, top+height/2));
      const moving = following.parentElement!.getAnimations().some(animation => animation.id === 'refract-motion');
      const next = await new Promise<number>(resolve => requestAnimationFrame(resolve));
      return {closed:!details.open, disclosure, ownsPointer, moving, firstFrame:Math.round(next-start)};
    });
    const animated = await collapse();
    await page.emulateMedia({reducedMotion:'reduce'});
    await folder.press('Enter');
    const reduced = await collapse();
    for (const {closed, disclosure, ownsPointer, moving} of [animated, reduced])
      assert.deepEqual({closed, disclosure, ownsPointer, moving}, {closed:true, disclosure:'hidden', ownsPointer:true, moving:false});
    context.diagnostic(`Large folder collapse first sampled frame: ${animated.firstFrame} ms with motion, ${reduced.firstFrame} ms with reduced motion.`);
    assert.deepEqual(errors, []);
  } finally {
    await browser.close();
  }
});

test('sidebar motion does not repeatedly lay out a 24-page PDF', {timeout:30000}, async context => {
  const {browser, page, errors} = await fixture.openHeadlessUi('light', undefined, {pageCount:24});
  try {
    await page.evaluate(() => {
      (window as unknown as {fixture:{files:Record<string,string>}}).fixture.files['z-last.tex'] = '% Moving sibling';
    });
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
        const panelMoving = sidebar.querySelector('.sidebar-motion-surface')!.getAnimations()
          .some(animation => animation.id === 'refract-motion');
        const startingEdge = edge.getBoundingClientRect().left;
        const collapseFrames = await monitor();
        button.click();
        const expanded = sidebar.clientWidth;
        const expandFrames = await monitor();
        const content = document.querySelector('#project-side')!;
        return {collapsed, expanded, movingEdge, panelMoving, edgeStartsAtOldSide:Math.abs(startingEdge - oldEdge) < 3,
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
          await new Promise(resolve => setTimeout(resolve, 165));
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
    assert.equal(animated.panelMoving, true);
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
