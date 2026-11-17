import fs = require('node:fs/promises');
import path = require('node:path');
import crypto = require('node:crypto');
import childProcess = require('node:child_process');

const tools = ['pdflatex', 'xelatex', 'lualatex', 'kpsewhich', 'synctex', 'bibtex', 'biber'];
async function complete(root: string): Promise<boolean> {
  return (await Promise.all(tools.map(tool => fs.stat(path.join(root, 'bin/windows', `${tool}.exe`)).then(info => info.isFile(), () => false)))).every(Boolean);
}
async function download(url: string, hash: string, file: string): Promise<void> {
  const response = await fetch(url, { signal: AbortSignal.timeout(600000) });
  if (!response.ok) throw new Error(`Download failed: ${response.status} ${url}`);
  const bytes = Buffer.from(await response.arrayBuffer());
  if (crypto.createHash('sha256').update(bytes).digest('hex') !== hash) throw new Error(`Checksum mismatch: ${url}`);
  await fs.writeFile(file, bytes);
}
function extract(archive: string, destination: string, files: string[] = []): void {
  const result = childProcess.spawnSync('tar.exe', ['-xf', archive, '-C', destination, ...files], { windowsHide: true, encoding: 'utf8', timeout: 600000 });
  if (result.error || result.status !== 0) throw new Error(`Extraction failed: ${result.error?.message || result.stderr}`);
}
interface InstallOptions {
  localAppData?: string;
  download?: typeof download;
  extract?: typeof extract;
}
async function install(options: InstallOptions = {}): Promise<void> {
  const localAppData = options.localAppData ?? process.env.LOCALAPPDATA;
  if (!localAppData || !path.isAbsolute(localAppData)) throw new Error('LOCALAPPDATA must be an absolute directory');
  const root = path.join(localAppData, 'Refract', 'dependencies');
  const fetchArchive = options.download ?? download, unpack = options.extract ?? extract;
  await fs.mkdir(root, { recursive: true });
  const target = path.join(root, 'TinyTeX');
  if (await complete(target)) return;
  // Unique staging keeps failed downloads from becoming the active toolchain.
  const staging = await fs.mkdtemp(path.join(root, 'setup-'));
  try {
    const archive = path.join(staging, 'tex.zip');
    await fetchArchive('https://github.com/rstudio/tinytex-releases/releases/download/v2026.10/TinyTeX-v2026.10.zip', 'dc1d6e63b21af62130ede1dcc4113bffdc184544690a23e3f489d75340f4ff61', archive);
    unpack(archive, staging);
    const sync = path.join(staging, 'sync.tar.xz');
    // Pin the upstream revision as well as its checksum.
    await fetchArchive('https://mirrors.ctan.org/systems/texlive/tlnet/archive/synctex.windows.r78097.tar.xz', '09ff70bf3a0de6b1c81c5aa3eb136473fce83314dc407a9ad6c4592376a6a83c', sync);
    unpack(sync, path.join(staging, 'TinyTeX'), ['bin/windows/synctex.exe']);
    if (!await complete(path.join(staging, 'TinyTeX'))) throw new Error('Downloaded TeX is missing required tools');
    // Preserve any incomplete previous installation rather than deleting user files.
    try { await fs.rename(target, path.join(root, `previous-${path.basename(staging)}`)); } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== 'ENOENT') throw error;
    }
    await fs.rename(path.join(staging, 'TinyTeX'), target);
  } finally {
    // Only remove this function's unique staging directory, inside our data root.
    if (path.dirname(staging) !== root || !path.basename(staging).startsWith('setup-')) throw new Error('Unsafe staging path');
    await fs.rm(staging, { recursive: true, force: true });
  }
}
if (require.main === module) install().catch(async (error: unknown) => {
  const message = error instanceof Error ? error.message : String(error);
  console.error(message);
  if (process.env.LOCALAPPDATA) await fs.writeFile(path.join(process.env.LOCALAPPDATA, 'Refract', 'dependencies', 'setup-error.log'), message).catch(() => {});
  process.exitCode = 1;
});
export = { complete, install, download };
