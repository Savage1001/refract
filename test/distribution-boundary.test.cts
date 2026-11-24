import nodeTest = require('node:test');
const { test } = nodeTest;
import type { TestContext } from 'node:test';
import assert = require('node:assert/strict');
import fs = require('node:fs/promises');
import os = require('node:os');
import path = require('node:path');
import childProcess = require('node:child_process');
const { spawnSync } = childProcess;
import boundary = require('../scripts/distribution-boundary.cjs');
const { applicationFiles, assertDistributionConfig, assertNoTex, afterPack } = boundary;
import toolchain = require('../src/tex-toolchain.cjs');
const { texBin } = toolchain;
const { configuredBuild } = require('../src/assistant-build-bridge.cjs');
const project = path.resolve(__dirname, '../..');

async function fixture(t: TestContext): Promise<string> {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), 'refract-distribution-'));
  t.after(async () => {
    const resolved = await fs.realpath(root), parent = await fs.realpath(os.tmpdir());
    assert.equal(path.dirname(resolved).toLowerCase(), parent.toLowerCase());
    assert.ok(path.basename(resolved).startsWith('refract-distribution-'));
    await fs.rm(resolved, { recursive: true, force: true });
  });
  return root;
}

test('packaging hooks resolve through electron-builder and reject resource overrides', async () => {
  const pkg = JSON.parse(await fs.readFile(path.join(project, 'package.json'), 'utf8'));
  await assertDistributionConfig(pkg.build);
  const { resolveFunction } = require('app-builder-lib/out/util/resolve');
  const { getConfig } = require('app-builder-lib/out/util/config/config');
  const effective = await getConfig(project, null, null, { getValue: () => pkg });
  for (const name of ['beforePack', 'afterPack']) {
    const hook = await resolveFunction('commonjs', pkg.build[name], name, project);
    assert.equal(typeof hook, 'function');
    if (name === 'beforePack') await hook({ packager: { projectDir: project, config: effective } });
  }
  for (const override of [
    { extraResources: [{ from: 'vendor/TinyTeX', to: 'TinyTeX' }] },
    { extraFiles: ['vendor/**/*'] }, { files: ['**/*'] }, { asar: true },
    { win: { extraResources: ['vendor/**/*'] } },
    { mac: { extraResources: ['vendor/**/*'] } },
  ]) await assert.rejects(assertDistributionConfig({ ...pkg.build, ...override }), /Distribution boundary/);
});

test('staging scan rejects TeX in nested resources and accepts application SyncTeX code', async t => {
  const root = await fixture(t), nested = path.join(root, 'resources/app/build/src');
  await fs.mkdir(nested, { recursive: true });
  await fs.writeFile(path.join(nested, 'synctex.cjs'), 'inert application fixture');
  await afterPack({ appOutDir: root });
  for (const name of ['TinyTeX', 'texmf-dist', 'pdflatex.exe', 'synctex.exe', 'synctex.windows.r78097.tar.xz', 'TinyTeX-v2026.10.zip', 'biber.exe', 'gswin64c.exe']) {
    const file = path.join(nested, name);
    await fs.writeFile(file, 'inert fixture');
    await assert.rejects(assertNoTex(root), /Distribution boundary/);
    await fs.unlink(file);
  }
});

test('file allowlist excludes old vendor trees, source caches and stale build archives', () => {
  const { createRequire } = require('node:module') as typeof import('node:module');
  const { minimatch } = createRequire(require.resolve('app-builder-lib'))('minimatch') as {
    minimatch(file: string, pattern: string): boolean;
  };
  for (const file of ['vendor/TinyTeX/bin/windows/pdflatex.exe', 'vendor/refract-tex-source-audit/source.tar.xz', 'resources/synctex/synctex.windows.r78097.tar.xz', 'build/TinyTeX/bin/windows/pdflatex.exe', 'build/test/fixture.exe']) {
    assert.equal(applicationFiles.some(pattern => !pattern.startsWith('!') && minimatch(file, pattern)), false, file);
  }
});

test('macOS staging accepts only links confined to an Electron framework', { skip: process.platform === 'win32' }, async t => {
  const root = await fixture(t);
  const framework = path.join(root, 'Refract.app', 'Contents', 'Frameworks', 'Squirrel.framework');
  await fs.mkdir(path.join(framework, 'Versions', 'A'), { recursive: true });
  await fs.symlink('A', path.join(framework, 'Versions', 'Current'));
  await afterPack({ appOutDir: root, electronPlatformName: 'darwin' });
  await assert.rejects(afterPack({ appOutDir: root }), /unexpected link/);
  await fs.symlink('..', path.join(framework, 'outside'));
  await assert.rejects(afterPack({ appOutDir: root, electronPlatformName: 'darwin' }), /unexpected link/);
});

test('external configuration has no implicit bundled fallback and selects every engine', async () => {
  assert.throws(() => texBin({}), /REFRACT_TEX_BIN/);
  assert.throws(() => texBin({ REFRACT_TEX_BIN: 'relative/bin' }), /absolute/);
  const bin = path.join(os.tmpdir(), 'external TeX', 'bin', 'windows');
  assert.equal(texBin({ REFRACT_TEX_BIN: bin }), bin);
  for (const engine of ['pdflatex', 'xelatex', 'lualatex']) {
    const options = await configuredBuild({ metadata: async () => ({ engine }) }, 'fixture', bin);
    assert.equal(options.executable, path.join(bin, process.platform === 'win32' ? `${engine}.exe` : engine));
  }
});

test('missing external configuration reports setup instructions without replacing an existing PDF', async t => {
  const root = await fixture(t), output = path.join(root, '.refract-build');
  await fs.mkdir(output);
  await fs.writeFile(path.join(root, 'main.tex'), 'inert source fixture');
  await fs.writeFile(path.join(output, 'main.pdf'), 'previous PDF');
  const previous = process.env.REFRACT_TEX_BIN;
  const localAppData = process.env.LOCALAPPDATA;
  delete process.env.REFRACT_TEX_BIN;
  process.env.LOCALAPPDATA = root;
  try {
    const { compile } = require('../src/compiler.cjs');
    await assert.rejects(compile(root), /REFRACT_TEX_BIN/);
    assert.equal(await fs.readFile(path.join(output, 'main.pdf'), 'utf8'), 'previous PDF');
  } finally {
    if (previous === undefined) delete process.env.REFRACT_TEX_BIN;
    else process.env.REFRACT_TEX_BIN = previous;
    if (localAppData === undefined) delete process.env.LOCALAPPDATA;
    else process.env.LOCALAPPDATA = localAppData;
  }
});

test('setup validates inert external files without executing them or downloading TeX', { skip: process.platform !== 'win32' }, async t => {
  const root = await fixture(t);
  const run = () => spawnSync('powershell.exe', ['-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File', path.join(project, 'scripts/setup-tex.ps1'), '-TexBin', root], { windowsHide: true, encoding: 'utf8', timeout: 15000 });
  assert.notEqual(run().status, 0);
  for (const name of ['pdflatex', 'xelatex', 'lualatex', 'kpsewhich', 'synctex', 'bibtex', 'biber']) await fs.writeFile(path.join(root, `${name}.exe`), 'not an executable');
  const result = run();
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /executables were not run/);
  assert.equal((await fs.readdir(root)).length, 7);
});
