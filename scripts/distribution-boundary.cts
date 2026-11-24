import fs = require('node:fs/promises');
import path = require('node:path');

// Explicit application inputs exclude vendor, resources/synctex, test fixtures,
// source-audit caches, and arbitrary stale files in build/.
const applicationFiles = [
  'build/main.cjs', 'build/preload.cjs', 'build/renderer.js', 'build/model-picker.js',
  'build/src/**/*.cjs', 'build/scripts/install-dependencies.cjs', 'build/index.html', 'build/style.css', 'build/logo.png',
  'build/provider-logos/**/*', 'build/fonts/**/*', 'node_modules/**/*',
  '!node_modules/@napi-rs/canvas/**', '!node_modules/@napi-rs/canvas-*/**',
  'LICENSE', 'package.json',
];

async function assertDistributionConfig(config: Record<string, unknown>): Promise<void> {
  const files = config.files as (string | { from?: string; to?: string; filter?: string[] })[] | undefined;
  const patterns = files?.flatMap(item => typeof item === 'string' ? [item] :
    item.from || item.to ? ['unapproved file mapping'] : item.filter || []);
  const platformOverrides = [config.win, config.mac].some(value => {
    const platform = (value || {}) as Record<string, unknown>;
    return platform.files || platform.extraResources || platform.extraFiles;
  });
  if (JSON.stringify(patterns) !== JSON.stringify(applicationFiles) ||
      (config.extraResources as unknown[] | undefined)?.length ||
      (config.extraFiles as unknown[] | undefined)?.length || config.asar !== false ||
      platformOverrides) {
    throw new Error('Distribution boundary: package only the approved application files; TeX redistribution is disabled.');
  }
}

/** Scan the actual staging tree too, before electron-builder makes an installer. */
async function assertNoTex(root: string, electronFramework?: string): Promise<void> {
  for (const entry of await fs.readdir(root, { withFileTypes: true })) {
    const file = path.join(root, entry.name);
    if (/^(?:tinytex(?:[.-].*)?|texlive|texmf(?:-.*)?|tlpkg|synctex(?:\.exe|\.windows.*|\.tar.*)?|(?:pdftex|pdflatex|xetex|xelatex|luatex|lualatex|luahbtex|kpsewhich|biber|bibtex|tlmgr|gswin(?:32|64)c?)(?:\.exe)?)$/i.test(entry.name)) {
      throw new Error(`Distribution boundary: TeX material in staging: ${path.relative(root, file)}`);
    }
    if (entry.isSymbolicLink()) {
      // Electron's macOS framework uses relative links to its versioned files.
      // Their real targets must be inside the framework and are scanned separately.
      if (electronFramework && file.startsWith(electronFramework + path.sep) &&
          !path.isAbsolute(await fs.readlink(file)) &&
          (await fs.realpath(file)).startsWith(electronFramework + path.sep)) continue;
      throw new Error('Distribution boundary: unexpected link in staging.');
    }
    if (entry.isDirectory()) await assertNoTex(file, electronFramework);
  }
}

async function beforePack(context: { packager: { projectDir: string; config: Record<string, unknown> } }): Promise<void> {
  const pkg = JSON.parse(await fs.readFile(path.join(context.packager.projectDir, 'package.json'), 'utf8')) as { build: Record<string, unknown> };
  await assertDistributionConfig(pkg.build);
  await assertDistributionConfig(context.packager.config);
}

async function afterPack(context: { appOutDir: string; electronPlatformName?: string }): Promise<void> {
  const root = await fs.realpath(context.appOutDir);
  const framework = context.electronPlatformName === 'darwin'
    ? path.join(root, 'Refract.app', 'Contents', 'Frameworks', 'Electron Framework.framework')
    : undefined;
  await assertNoTex(root, framework);
}

export = { applicationFiles, assertDistributionConfig, assertNoTex, beforePack, afterPack };
