import fs = require('node:fs/promises');
import path = require('node:path');
import crypto = require('node:crypto');
import childProcess = require('node:child_process');
import timers = require('node:timers/promises');
import toolchain = require('../src/tex-toolchain.cjs');

const archives = {
  win32: {
    bin: 'bin/windows',
    extension: '.exe',
    archive: 'tex.zip',
    texUrl: 'https://github.com/rstudio/tinytex-releases/releases/download/v2026.10/TinyTeX-v2026.10.zip',
    texHash: 'dc1d6e63b21af62130ede1dcc4113bffdc184544690a23e3f489d75340f4ff61',
    syncUrl: 'https://ctan.math.illinois.edu/systems/texlive/tlnet/archive/synctex.windows.r78097.tar.xz',
    syncHash: '09ff70bf3a0de6b1c81c5aa3eb136473fce83314dc407a9ad6c4592376a6a83c',
  },
  darwin: {
    bin: toolchain.macTexSubdirectory,
    extension: '',
    archive: 'tex.tar.xz',
    texUrl: 'https://github.com/rstudio/tinytex-releases/releases/download/v2026.10/TinyTeX-darwin-v2026.10.tar.xz',
    texHash: '9962e445b56f06feb2c28f73561e523e89d550e98abf910ba60b0ffb3856921f',
    syncUrl: 'https://ctan.math.illinois.edu/systems/texlive/tlnet/archive/synctex.universal-darwin.r78096.tar.xz',
    syncHash: '1f247bb78b7ddd43e32b4adc7995c41c594b0eef012a2abc545deadf9c2f7f47',
  },
} as const;
type InstallerPlatform = keyof typeof archives;

async function complete(root: string, platform: InstallerPlatform = 'win32'): Promise<boolean> {
  const { bin, extension } = archives[platform];
  return (await Promise.all(toolchain.requiredTexTools.map(async tool => {
    const file = path.join(root, bin, tool + extension);
    try {
      if (platform === 'darwin') await fs.access(file, fs.constants.X_OK);
      return (await fs.stat(file)).isFile();
    } catch {
      return false;
    }
  }))).every(Boolean);
}
async function download(url: string, hash: string, file: string): Promise<void> {
  const response = await fetch(url, { signal: AbortSignal.timeout(600000) });
  if (!response.ok) throw new Error(`Download failed: ${response.status} ${url}`);
  const bytes = Buffer.from(await response.arrayBuffer());
  if (crypto.createHash('sha256').update(bytes).digest('hex') !== hash) throw new Error(`Checksum mismatch: ${url}`);
  await fs.writeFile(file, bytes);
}
function extract(archive: string, destination: string, files: string[] = []): void {
  const tar = process.platform === 'darwin' ? '/usr/bin/tar' : 'tar.exe';
  const result = childProcess.spawnSync(tar, ['--no-same-owner', '-xf', archive, '-C', destination, ...files], { windowsHide: true, encoding: 'utf8', timeout: 600000 });
  if (result.error || result.status !== 0) throw new Error(`Extraction failed: ${result.error?.message || result.stderr}`);
}
async function renameDirectory(source: string, destination: string): Promise<void> {
  // Windows can briefly lock extracted files while scanning them.
  const delays = [100, 200, 400, 800, 1600, 3200, 5000];
  for (const delay of delays) {
    try { await fs.rename(source, destination); return; }
    catch (error) {
      if (process.platform !== 'win32' || !['EPERM', 'EACCES', 'EBUSY'].includes((error as NodeJS.ErrnoException).code ?? '')) throw error;
      await timers.setTimeout(delay);
    }
  }
  await fs.rename(source, destination);
}
interface InstallOptions {
  localAppData?: string;
  download?: typeof download;
  extract?: typeof extract;
}
async function ensureMacRoot(root: string): Promise<void> {
  // The package hook runs as root. Never extract through a user-controlled path.
  for (const dir of [path.dirname(root), root]) {
    await fs.mkdir(dir).catch(error => { if (error.code !== 'EEXIST') throw error; });
    const stat = await fs.lstat(dir);
    if (!stat.isDirectory() || stat.uid !== 0 || (stat.mode & 0o022) !== 0) {
      throw new Error(`Unsafe macOS dependency directory: ${dir}`);
    }
  }
}
async function install(options: InstallOptions = {}): Promise<void> {
  const platform = process.platform === 'darwin' ? 'darwin' : 'win32';
  const config = archives[platform];
  const localAppData = options.localAppData ?? process.env.LOCALAPPDATA;
  let root: string;
  if (platform === 'darwin') {
    root = toolchain.macDependencies;
    await ensureMacRoot(root);
  } else {
    if (!localAppData || !path.isAbsolute(localAppData)) throw new Error('LOCALAPPDATA must be an absolute directory');
    root = path.join(localAppData, 'Refract', 'dependencies');
    await fs.mkdir(root, { recursive: true });
  }
  const fetchArchive = options.download ?? download, unpack = options.extract ?? extract;
  const target = path.join(root, 'TinyTeX');
  if (await complete(target, platform)) return;
  // Unique staging keeps failed downloads from becoming the active toolchain.
  const staging = await fs.mkdtemp(path.join(root, 'setup-'));
  try {
    const archive = path.join(staging, config.archive);
    await fetchArchive(config.texUrl, config.texHash, archive);
    unpack(archive, staging);
    const sync = path.join(staging, 'sync.tar.xz');
    // Pin the upstream revision as well as its checksum.
    await fetchArchive(config.syncUrl, config.syncHash, sync);
    unpack(sync, path.join(staging, 'TinyTeX'), [`${config.bin}/synctex${config.extension}`]);
    const extracted = path.join(staging, 'TinyTeX');
    if (!await complete(extracted, platform)) throw new Error('Downloaded TeX is missing required tools');
    if (await complete(target, platform)) return;
    // Preserve any incomplete previous installation rather than deleting user files.
    try { await renameDirectory(target, path.join(root, `previous-${path.basename(staging)}`)); } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== 'ENOENT') throw error;
    }
    try { await renameDirectory(extracted, target); } catch (error) {
      if (!await complete(target, platform)) throw error;
    }
  } finally {
    // Only remove this function's unique staging directory, inside our data root.
    if (path.dirname(staging) !== root || !path.basename(staging).startsWith('setup-')) throw new Error('Unsafe staging path');
    await fs.rm(staging, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 });
  }
}
if (require.main === module) install().catch(async (error: unknown) => {
  const message = error instanceof Error ? error.message : String(error);
  console.error(message);
  if (process.env.LOCALAPPDATA) await fs.writeFile(path.join(process.env.LOCALAPPDATA, 'Refract', 'dependencies', 'setup-error.log'), message).catch(() => {});
  process.exitCode = 1;
});
export = { complete, install, download };
