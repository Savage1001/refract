import fs = require('node:fs/promises');
const { createReadStream, createWriteStream } = require('node:fs') as typeof import('node:fs');
const { spawn } = require('node:child_process') as typeof import('node:child_process');
const { createHash } = require('node:crypto') as typeof import('node:crypto');
import path = require('node:path');
const { Readable, Transform } = require('node:stream') as typeof import('node:stream');
const { pipeline } = require('node:stream/promises') as typeof import('node:stream/promises');
const { once } = require('node:events') as typeof import('node:events');
const { createZstdDecompress } = require('node:zlib') as typeof import('node:zlib');

const version = '0.170.0';
const windowsAssets = {
  x64: { target: 'x86_64', bytes: 127199313, hash: 'a409c0bf32e35ba0b0902d235048957503a534153ba3b0a9e5caaef53ad574e0' },
  arm64: { target: 'aarch64', bytes: 118098720, hash: 'b17d1377a84a1a5b695da414ecb843b6c1951d6b1523e1dd3c07d15077ec61f2' },
};
const runtimeFiles = [
  'bin/codex-app-server.exe', 'bin/codex-code-mode-host.exe', 'bin/codex-exec-server.exe',
  'codex-path/rg.exe', 'codex-resources/codex-command-runner.exe',
  'codex-resources/codex-windows-sandbox-setup.exe',
];
export type RuntimeProgress = { phase: 'downloading'; percent: number } | { phase: 'installing' };

class ManagedCodex {
  private readonly asset = process.platform === 'win32' ? windowsAssets[process.arch as keyof typeof windowsAssets] : undefined;
  private readonly destination: string;
  private installation?: Promise<string>;

  constructor(root: string) {
    this.destination = path.join(root, version + '-' + process.arch);
  }

  async executable(): Promise<string | null> {
    if (!this.asset) return null;
    const active = await fs.readFile(path.join(this.destination, 'active'), 'utf8').catch(() => '');
    if (!/^setup-[A-Za-z0-9]+$/.test(active)) return null;
    const packageDir = path.join(this.destination, active, 'package');
    const complete = await Promise.all(runtimeFiles.map(file =>
      fs.lstat(path.join(packageDir, file)).then(info => info.isFile() && info.size > 0, () => false)));
    return complete.every(Boolean) ? path.join(packageDir, runtimeFiles[0]) : null;
  }

  install(report: (progress: RuntimeProgress) => void, signal: AbortSignal): Promise<string> {
    this.installation ??= this.download(report, signal).finally(() => { this.installation = undefined; });
    return this.installation;
  }

  private async download(report: (progress: RuntimeProgress) => void, signal: AbortSignal): Promise<string> {
    const existing = await this.executable();
    if (existing) return existing;
    const asset = this.asset;
    if (!asset) throw new Error('ChatGPT account mode requires Windows on x64 or ARM64.');

    await fs.mkdir(this.destination, { recursive: true });
    const stage = await fs.mkdtemp(path.join(this.destination, 'setup-'));
    const archive = path.join(stage, 'runtime.tar.zst');
    let activated = false;
    try {
      const url = `https://github.com/openai/codex/releases/download/rust-v${version}/codex-app-server-package-${asset.target}-pc-windows-msvc.tar.zst`;
      const transfer = AbortSignal.any([signal, AbortSignal.timeout(15 * 60_000)]);
      report({ phase: 'downloading', percent: 0 });
      const response = await fetch(url, { signal: transfer });
      if (!response.ok || !response.body) throw new Error('Could not download the Codex runtime.');
      const hash = createHash('sha256');
      let received = 0;
      let percent = 0;
      await pipeline(Readable.fromWeb(response.body), new Transform({
        transform(chunk: Buffer, _, done) {
          received += chunk.length;
          if (received > asset.bytes) return done(new Error('Unexpected Codex runtime download size.'));
          hash.update(chunk);
          const next = Math.min(99, Math.floor(received * 100 / asset.bytes));
          if (next !== percent) {
            percent = next;
            report({ phase: 'downloading', percent });
          }
          done(null, chunk);
        },
      }), createWriteStream(archive, { flags: 'wx' }), { signal: transfer });
      if (received !== asset.bytes || hash.digest('hex') !== asset.hash) throw new Error('Codex runtime checksum verification failed.');

      report({ phase: 'installing' });
      const packageDir = path.join(stage, 'package');
      await fs.mkdir(packageDir);
      const systemTar = path.join(process.env.SystemRoot || 'C:/Windows', 'System32', 'tar.exe');
      const extraction = AbortSignal.any([signal, AbortSignal.timeout(120_000)]);
      const tar = spawn(systemTar, ['-xf', '-', '-C', packageDir], {
        windowsHide: true, signal: extraction, stdio: ['pipe', 'ignore', 'ignore'],
      });
      const exited = once(tar, 'close').then(([code]) => {
        if (code !== 0) throw new Error('Could not extract the Codex runtime.');
      });
      try {
        await Promise.all([exited, pipeline(createReadStream(archive), createZstdDecompress(), tar.stdin, { signal: extraction })]);
      } finally {
        if (tar.exitCode === null && tar.signalCode === null) tar.kill();
      }
      for (const file of runtimeFiles) {
        const info = await fs.lstat(path.join(packageDir, file));
        if (!info.isFile() || info.size === 0) throw new Error('The Codex runtime package is incomplete.');
      }
      signal.throwIfAborted();
      // Publishing a marker keeps an incomplete download from ever becoming the active runtime.
      const marker = path.join(stage, 'ready');
      await fs.writeFile(marker, path.basename(stage), { flag: 'wx' });
      await fs.rename(marker, path.join(this.destination, 'active'));
      activated = true;
      return path.join(packageDir, runtimeFiles[0]);
    } finally {
      await fs.rm(activated ? archive : stage, { recursive: !activated, force: true, maxRetries: 5, retryDelay: 200 }).catch(() => {});
    }
  }
}

export = { ManagedCodex };
