import assert = require('node:assert/strict');
import fs = require('node:fs/promises');
import os = require('node:os');
import path = require('node:path');
import childProcess = require('node:child_process');
import fixture = require('../test/fixtures/headless-ui.cjs');
import type { Page } from 'playwright';

const output = process.env.REFRACT_MOTION_ARTIFACTS ?? path.join(os.tmpdir(), 'refract-motion-clips');
const pause = (page: Page, ms = 610) => page.waitForTimeout(ms);
const project = '#list .project-open';
async function click(page: Page, target: string): Promise<void> {
  await page.locator(target).first().click();
  await pause(page);
}
async function open(page: Page, waitForPdf = true): Promise<void> {
  await page.locator(project).first().click();
  await page.locator('#source').waitFor();
  if (waitForPdf) await page.locator('#pdf-area canvas').first().waitFor();
  await pause(page);
}

type Scene = [name: string, run: (page: Page) => Promise<void>, theme?: 'dark', reduced?: 'reduce',
  viewport?: {width:number;height:number}];
const scenes: Scene[] = [
  ['01-control-feedback', async page => {
    const settings = page.locator('#settings');
    await settings.hover();
    await pause(page);
    await page.mouse.down();
    await pause(page, 300);
    await page.mouse.move(20, 90);
    await page.mouse.up();
    await page.locator('#search').click();
    await pause(page);
    await page.keyboard.press('Tab');
    await pause(page);
    const create = (await page.locator('#new').boundingBox())!;
    await page.mouse.move(create.x + .5, create.y + create.height / 2);
    await page.mouse.down();
    await pause(page, 155);
    await page.mouse.up();
    await page.locator('#source').waitFor();
    await pause(page);
  }],
  ['02-project-navigation', async page => {
    await open(page);
    await click(page, '#home');
    await page.locator(project).first().focus();
    await page.keyboard.press('Enter');
    await page.locator('#source').waitFor();
    await pause(page);
    await page.locator('#home').focus();
    await page.keyboard.press('Enter');
    await page.locator(project).first().waitFor();
  }],
  ['03-list-and-grid', async page => {
    await click(page, '#grid-view');
    await click(page, '#list-view');
  }],
  ['04-dialog-and-backdrop', async page => {
    await page.locator('#settings').focus();
    await page.keyboard.press('Enter');
    await pause(page);
    await page.keyboard.press('Escape');
    await pause(page);
    await click(page, '#deleted-projects');
    await page.keyboard.press('Escape');
  }, 'dark'],
  ['05-project-and-file-menus', async page => {
    await click(page, '.project-actions summary');
    await page.keyboard.press('Escape');
    await pause(page);
    await open(page);
    const file = page.locator('#file-list button[title="main.tex"]');
    await file.click({button:'right'});
    await pause(page);
    await page.keyboard.press('Escape');
  }],
  ['06-project-search-states', async page => {
    await page.locator('#search').fill('Unmatched project');
    await pause(page, 900);
    await click(page, '#clear-search');
  }],
  ['07-sidebar-collapse', async page => {
    await open(page);
    await click(page, '#collapse');
    await click(page, '#collapse');
    await page.locator('#collapse').click();
    await pause(page, 95);
    await page.locator('#collapse').click();
    await pause(page);
  }, 'dark'],
  ['08-file-tree-outline-and-disclosures', async page => {
    await page.evaluate(() => {
      const {files} = (window as unknown as {fixture:{files:Record<string,string>}}).fixture;
      files['z-results.tex'] = '% Results';
      files['z-review.tex'] = '% Review notes';
      for (let index = 0; index < 4; index++) files[`sections/notes-${index}.tex`] = '% Nested notes';
    });
    await open(page);
    const folder = page.locator('#file-list summary').first();
    const point = await folder.evaluate((summary:HTMLElement) => {
      const first = document.querySelector('#file-list button[title="z-results.tex"]')!;
      const second = document.querySelector('#file-list button[title="z-review.tex"]')!;
      summary.click();
      for (const file of [first, second]) {
        const animation = file.parentElement!.getAnimations().find(motion => motion.id === 'refract-motion')!;
        animation.currentTime = 0;
        animation.playbackRate = .3;
      }
      const {left,top,width,height} = first.getBoundingClientRect();
      return {x:left+width/2,y:top+height/2};
    });
    await page.mouse.move(point.x, point.y);
    await page.mouse.down();
    await pause(page, 170);
    await page.mouse.up();
    await page.waitForFunction(() => (document.getElementById('source') as HTMLTextAreaElement).value === '% Results');
    await pause(page);
    await folder.press('Enter');
    await pause(page);
    await folder.press('Enter');
    await pause(page, 45);
    await folder.press('Enter');
    await page.keyboard.press('Tab');
    await pause(page);
    await page.keyboard.press('Tab');
    await pause(page, 400);
    await click(page, '#file-list button[title="main.tex"]');
    await page.locator('#outline-list button').first().waitFor();
    await click(page, '#outline-toggle');
    await click(page, '#outline-toggle');
    await click(page, '#add-file');
    await page.locator('#file-name').press('Enter');
    await pause(page);
    await click(page, '.file-alternatives summary');
    await click(page, '.file-alternatives summary');
    await page.keyboard.press('Escape');
  }],
  ['09-sidebar-tabs-and-preview', async page => {
    await page.evaluate(() => {
      const w = window as unknown as {
        store:{openFile:(id:string,name:string)=>Promise<unknown>};
        fixture:{files:Record<string,string>};
      };
      const image = 'figures/spectral-curve.svg';
      const svg = '<svg xmlns="http://www.w3.org/2000/svg" width="320" height="220"><rect width="320" height="220" rx="12" fill="#dde8f6"/><path d="M32 182 L32 36 M32 182 L286 182 M38 161 Q112 164 152 101 T280 52" fill="none" stroke="#234b7c" stroke-width="4"/></svg>';
      w.fixture.files[image] = svg;
      const openFile = w.store.openFile;
      w.store.openFile = (id, name) => name === image ? Promise.resolve({
        kind:'image', data:`data:image/svg+xml,${encodeURIComponent(svg)}`
      }) : openFile(id, name);
    });
    await open(page);
    await click(page, '#chats-tab');
    await click(page, '#files-tab');
    await page.locator('#chats-tab').click();
    await pause(page, 45);
    await page.locator('#files-tab').click();
    await pause(page);
    await click(page, '#file-list button[title="figures/spectral-curve.svg"]');
  }],
  ['10-find-and-command-completions', async page => {
    await open(page);
    await page.locator('#source').focus();
    await page.keyboard.press('Control+f');
    await pause(page);
    await page.keyboard.press('Escape');
    const editor = page.locator('#source');
    await editor.fill('\\sec');
    await editor.press('End');
    await editor.press('Control+Space');
    await pause(page);
    await editor.press('Escape');
  }],
  ['11-workspace-panes', async page => {
    await open(page);
    await click(page, '#pane-preview');
    await click(page, '#pane-agent');
    await click(page, '#pane-agent');
    await click(page, '#pane-preview');
    await click(page, '.expand-chat');
    await click(page, '.expand-chat');
  }, 'dark'],
  ['12-model-picker', async page => {
    await open(page);
    await page.locator('.model-trigger').focus();
    await page.keyboard.press('Enter');
    await pause(page);
    await page.getByRole('searchbox', {name:'Search models'}).fill('Research');
    await pause(page);
    await page.keyboard.press('Escape');
    await pause(page);
    await click(page, '.model-trigger');
    await page.keyboard.press('Escape');
  }],
  ['13-assistant-messages-and-progress', async page => {
    await open(page);
    await page.locator('#new-chat').hover();
    await pause(page);
    await page.evaluate(() => {
      const {fixture, store} = window as unknown as {fixture:{delay:number};
        store:{chatSend:()=>Promise<{messages:{text:string}[]}>}};
      fixture.delay = 900;
      const send = store.chatSend;
      store.chatSend = async () => {
        const reply = await send();
        reply.messages.at(-1)!.text += '\n\nRead the [method reference](https://example.com/method).';
        return reply;
      };
    });
    const composer = page.locator('#chat-input');
    await composer.fill('How should I structure the introduction?');
    await click(page, '#send');
    await page.getByRole('link', {name:'method reference'}).waitFor({state:'attached'});
    await composer.press('Shift+Tab');
    assert.equal(await page.getByRole('link', {name:'method reference'}).evaluate(link => link === document.activeElement), true);
    await pause(page, 950);
  }, 'dark'],
  ['14-pdf-compile-and-preview', async page => {
    await page.evaluate(() => {
      const w = window as unknown as {store:{compile:()=>Promise<unknown>}};
      const compile = w.store.compile;
      w.store.compile = async () => {
        await new Promise(resolve => setTimeout(resolve, 850));
        return compile();
      };
    });
    await open(page, false);
    await page.locator('#pdf-area canvas').first().waitFor();
    await pause(page, 950);
  }],
  ['15-project-tools-and-replies', async page => {
    await open(page);
    await click(page, '#tools');
    await page.evaluate(() => {
      const w = window as unknown as {store:{commentList:(id:string)=>Promise<unknown[]>}; finishComments?:()=>void};
      const load = w.store.commentList;
      w.store.commentList = id => new Promise(resolve => { w.finishComments = () => { void load(id).then(resolve); }; });
    });
    await page.locator('#tool-comments').click();
    await page.waitForFunction(() => typeof (window as unknown as {finishComments?:()=>void}).finishComments === 'function');
    await pause(page, 320);
    await page.locator('.expand-tools').click();
    await pause(page, 90);
    await page.evaluate(() => (window as unknown as {finishComments:()=>void}).finishComments());
    await page.locator('.comment-card').first().waitFor();
    await pause(page);
    await click(page, '.comment-reply-form summary');
    await page.locator('.expand-tools').focus();
    await page.keyboard.press('Enter');
    await pause(page);
    await click(page, '#tool-versions');
    await click(page, '.version-comparison summary');
    await click(page, '#tool-info');
    await click(page, '#tool-logs');
    await page.locator('.raw-output summary').press('Enter');
    await page.keyboard.press('Tab');
    assert.equal(await page.getByRole('searchbox', {name:'Filter build output'}).evaluate(input => input === document.activeElement), true);
    await pause(page);
    await click(page, '.raw-output summary');
    await click(page, '#tools');
  }],
  ['16-update-progress', async page => {
    const status = (phase: string, percent?: number) => page.evaluate(({phase,percent}) => {
      const w=window as unknown as {store:{updateStatus:()=>Promise<unknown>}};
      w.store.updateStatus=async()=>({track:'stable',phase,downloadPercent:percent,
        message:phase==='ready' ? 'An update is ready to install.' : 'Downloading an update…'});
    }, {phase,percent});
    await status('downloading', 18);
    await click(page, '#settings');
    await status('downloading', 72);
    await page.getByText('72%', {exact:true}).waitFor();
    await pause(page);
    await status('ready');
    await page.locator('#restart-update').waitFor();
    await pause(page);
  }, 'dark'],
  ['17-reduced-motion', async page => {
    await click(page, '#grid-view');
    await click(page, '#settings');
    await page.keyboard.press('Escape');
    await open(page);
    await click(page, '#pane-agent');
    await click(page, '#collapse');
  }, 'dark', 'reduce'],
  ['18-compact-workspace', async page => {
    await open(page, false);
    await click(page, '#pane-preview');
    await page.locator('#pdf-area canvas').first().waitFor();
    await click(page, '#pane-editor');
    await click(page, '#tools');
    await click(page, '#pane-editor');
  }, 'dark', undefined, {width:1000,height:680}],
  ['19-rapid-reversal', async page => {
    await page.locator('#grid-view').click();
    await page.locator('#list-view').click();
    await pause(page);
    await page.evaluate(() => {
      const w = window as unknown as {recordedPreviewFade?: Animation};
      const observer = new MutationObserver(() => {
        const pdf = document.querySelector('.pdf-pages');
        if (!pdf) return;
        observer.disconnect();
        w.recordedPreviewFade = pdf.getAnimations().find(animation => animation.id === 'refract-motion');
        w.recordedPreviewFade?.pause();
      });
      observer.observe(document.getElementById('pdf-area')!, {childList:true});
    });
    await page.locator(project).first().click();
    await page.waitForFunction(() => Boolean((window as unknown as {recordedPreviewFade?:Animation}).recordedPreviewFade));
    const preview = (await page.locator('#pane-preview').boundingBox())!;
    await page.mouse.move(preview.x + preview.width / 2, preview.y + preview.height / 2);
    const firstClick = page.waitForFunction(() => document.getElementById('pane-preview')?.getAttribute('aria-pressed') === 'false');
    await page.evaluate(() => (window as unknown as {recordedPreviewFade:Animation}).recordedPreviewFade.play());
    await page.mouse.down();
    await page.mouse.up();
    await firstClick;
    await pause(page, 45);
    await page.mouse.click(preview.x + preview.width / 2, preview.y + preview.height / 2);
    await page.locator('#pane-preview[aria-pressed=true]').waitFor();
    await page.evaluate(() => document.dispatchEvent(new CustomEvent('refract-recorded-caption',
      {detail:'Preview toggled twice with the pointer'})));
    await pause(page);
    const collapse = page.locator('#collapse');
    await collapse.click();
    await collapse.click();
    await pause(page);
    await click(page, '#tools');
    await page.locator('.expand-tools').click();
    await pause(page, 65);
    await page.locator('#tools').click();
    await page.evaluate(() => document.dispatchEvent(new CustomEvent('refract-recorded-caption',
      {detail:'Tools returned to Preview during expansion'})));
    await pause(page, 800);
  }],
  ['20-content-search-results', async page => {
    await page.evaluate(() => {
      const w = window as unknown as {store:{search:(id:string,query:string)=>Promise<unknown[]>}};
      w.store.search = async (_, query) => query === 'stable'
        ? [{file:'sections/abstract.tex',line:2,column:2,text:'A stable spectral approximation.'}]
        : [];
    });
    await open(page);
    const input = page.locator('#file-search-input');
    await input.pressSequentially('stable', {delay:55});
    await page.locator('#file-search-results').getByText('1 result', {exact:true}).waitFor();
    await pause(page);
    await input.fill('no match');
    await page.getByText('No matches. Try another search.', {exact:true}).waitFor();
    await pause(page);
  }],
];

async function record([name, run, theme, reducedMotion, viewport]: Scene): Promise<void> {
  const size = viewport ?? {width:1152,height:720};
  const {browser, page, errors} = await fixture.openHeadlessUi(theme, undefined, {
    reducedMotion, recordVideo:{dir:output, size}, ...(viewport ? {viewport} : {})
  });
  const video = page.video()!;
  try {
    await page.evaluate(async () => {
      await document.fonts.ready;
      const dot = document.createElement('span');
      dot.style.cssText = 'position:fixed;z-index:1000;pointer-events:none;width:12px;height:12px;border:2px solid white;border-radius:50%;background:#265ede;box-shadow:0 0 0 1px #1b2743;transform:translate(5px,5px);display:none;';
      const keyCue = document.createElement('div');
      keyCue.popover = 'manual';
      keyCue.setAttribute('aria-hidden', 'true');
      keyCue.style.cssText = 'position:fixed;inset:auto 16px 16px auto;margin:0;pointer-events:none;color:white;background:#182236;border:1px solid #647da6;border-radius:6px;padding:7px 12px;font:bold 18px system-ui;';
      document.body.append(dot, keyCue);
      const movePointer = (event: PointerEvent) => {
        dot.style.display = 'block';
        dot.style.left = `${event.clientX}px`;
        dot.style.top = `${event.clientY}px`;
      };
      const clickPointer = (event: PointerEvent) => {
        movePointer(event);
        if (!matchMedia('(prefers-reduced-motion: reduce)').matches)
          dot.animate([{scale:1.8},{scale:1}], {duration:230,easing:'ease-out'});
      };
      document.addEventListener('pointermove', movePointer);
      document.addEventListener('pointerdown', clickPointer);
      let cueTimer: number | undefined;
      const showCue = (label: string, duration = 650) => {
        keyCue.textContent = label;
        if (!keyCue.matches(':popover-open')) keyCue.showPopover();
        clearTimeout(cueTimer);
        cueTimer = window.setTimeout(() => keyCue.hidePopover(), duration);
      };
      document.addEventListener('refract-recorded-caption', event => showCue((event as CustomEvent<string>).detail, 1200));
      document.addEventListener('keydown', event => {
        if (!event.ctrlKey && !['Escape','Enter','Tab','F3'].includes(event.key)) return;
        if (['Control','Shift','Alt','Meta'].includes(event.key)) return;
        const key = event.key === ' ' ? 'Space' : event.key.length === 1 ? event.key.toUpperCase() : event.key;
        showCue([event.ctrlKey && 'Ctrl', event.shiftKey && 'Shift', key].filter(Boolean).join(' + '));
      }, true);
    });
    await page.locator(project).first().waitFor();
    await pause(page, 400);
    await run(page);
    await pause(page, 550);
    assert.deepEqual(errors, []);
  } finally {
    await page.context().close();
    await browser.close();
  }
  const mp4 = path.join(output, `${name}.mp4`);
  const result = childProcess.spawnSync('ffmpeg', ['-v','error','-y','-i',await video.path(),'-c:v','libx264',
    '-crf','19','-pix_fmt','yuv420p','-movflags','+faststart',mp4], {stdio:'inherit',windowsHide:true});
  assert.equal(result.status, 0, `Could not encode ${name}`);
  const info = await fs.stat(mp4);
  assert.ok(info.size < 10_000_000, `${name} exceeds GitHub's upload limit`);
  console.log(`${name} ${info.size} bytes`);
}

async function main(): Promise<void> {
  await fs.mkdir(output, {recursive:true});
  const filter = process.argv[2];
  const selected = scenes.filter(([name]) => !filter || new RegExp(filter).test(name));
  assert.ok(selected.length, 'No recording matches the scene filter');
  for (const scene of selected) await record(scene);
}

void main().catch((error: unknown) => { console.error(error); process.exitCode = 1; });
