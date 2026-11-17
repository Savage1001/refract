import fs = require('node:fs/promises');
import path = require('node:path');

// Explicit application inputs exclude vendor, resources/synctex, test fixtures,
// source-audit caches, and arbitrary stale files in build/.
const applicationFiles = [
  'build/main.cjs', 'build/preload.cjs', 'build/renderer.js', 'build/model-picker.js',
  'build/src/**/*.cjs', 'build/index.html', 'build/style.css', 'build/logo.png',
  'build/provider-logos/**/*', 'build/fonts/**/*', 'node_modules/**/*',
  '!node_modules/@napi-rs/canvas/**', '!node_modules/@napi-rs/canvas-*/**',
  'LICENSE', 'package.json',
];

async function assertDistributionConfig(config: Record<string, unknown>): Promise<void> {
  const files = config.files as (string | { from?: string; to?: string; filter?: string[] })[] | undefined;
  const patterns = files?.flatMap(item => typeof item === 'string' ? [item] :
    item.from || item.to ? ['unapproved file mapping'] : item.filter || []);
  const win = (config.win || {}) as Record<string, unknown>;
  if (JSON.stringify(patterns) !== JSON.stringify(applicationFiles) ||
      (config.extraResources as unknown[] | undefined)?.length ||
      (config.extraFiles as unknown[] | undefined)?.length || config.asar !== false ||
      win.files || win.extraResources || win.extraFiles) {
    throw new Error('Distribution boundary: package only the approved application files; TeX redistribution is disabled.');
  }
}

/** Scan the actual staging tree too, before electron-builder makes an installer. */
async function assertNoTex(root: string): Promise<void> {
  for (const entry of await fs.readdir(root, { withFileTypes: true })) {
    const file = path.join(root, entry.name);
    if (entry.isSymbolicLink()) throw new Error('Distribution boundary: links in staging are not allowed.');
    if (/^(?:tinytex(?:[.-].*)?|texlive|texmf(?:-.*)?|tlpkg|synctex(?:\.exe|\.windows.*|\.tar.*)?|(?:pdftex|pdflatex|xetex|xelatex|luatex|lualatex|luahbtex|kpsewhich|biber|bibtex|tlmgr|gswin(?:32|64)c?)(?:\.exe)?)$/i.test(entry.name)) {
      throw new Error(`Distribution boundary: TeX material in staging: ${path.relative(root, file)}`);
    }
    if (entry.isDirectory()) await assertNoTex(file);
  }
}

async function beforePack(context: { packager: { projectDir: string; config: Record<string, unknown> } }): Promise<void> {
  const pkg = JSON.parse(await fs.readFile(path.join(context.packager.projectDir, 'package.json'), 'utf8')) as { build: Record<string, unknown> };
  await assertDistributionConfig(pkg.build);
  await assertDistributionConfig(context.packager.config);
}

async function afterPack(context: { appOutDir: string }): Promise<void> {
  await assertNoTex(context.appOutDir);
}

export = { applicationFiles, assertDistributionConfig, assertNoTex, beforePack, afterPack };
