import childProcess = require('node:child_process');
import type { ChildProcess } from 'node:child_process';
import filesystem = require('node:fs/promises');
import os = require('node:os');
import path = require('node:path');
import timers = require('node:timers/promises');
import util = require('node:util');

const { execFile, spawn } = childProcess;
const { access, copyFile, mkdir, mkdtemp, open, readdir, readFile, rm, writeFile } = filesystem;
const { arch, release, tmpdir } = os;
const { setTimeout: delay } = timers;
const { promisify } = util;

const run = promisify(execFile);
const root = path.resolve(__dirname, '../..');
const fixture = path.join(root, 'test/fixtures/parity-document');
const projectId = '2b98e215-1a9f-4f24-972b-000000000000';
const names = ['Spectral methods for fluid dynamics', 'Conference paper - autumn notes', 'A short introduction to graph theory'];
const width = 1440;
const height = 900;
const idleMs = 10_000;
const rounds = 3;

type Runtime = 'native' | 'electron';
type View = 'library' | 'workspace';
type ProcessSample = { pid: number; parent: number; cpuTicks: number; startTicks: number };
type Measurement = { pssKiB: number[]; processCounts: number[]; idleDurationMs: number; idleCpuMs: number };
type Trial = { runtime: Runtime; library: Measurement; workspace: Measurement };

async function x11(command: string, args: string[]): Promise<string> {
  return (await run(command, args, { timeout: 5_000 })).stdout.trim();
}

async function until<T>(description: string, app: ChildProcess, attempt: () => Promise<T | undefined>): Promise<T> {
  const deadline = performance.now() + 30_000;
  do {
    if (app.exitCode !== null || app.signalCode !== null) throw new Error(`Refract exited while waiting for ${description}. See app.log.`);
    const result = await attempt();
    if (result !== undefined) return result;
    await delay(250);
  } while (performance.now() < deadline);
  throw new Error(`Timed out waiting for ${description}. See the trial screenshot and app.log.`);
}

async function readProcess(pid: number): Promise<ProcessSample | undefined> {
  const stat = await readFile(`/proc/${pid}/stat`, 'utf8').catch(missingProcess);
  if (stat === undefined) return;
  // The parenthesized process name can contain spaces and parentheses.
  const fields = stat.slice(stat.lastIndexOf(')') + 2).split(' ');
  const [parent, userTicks, systemTicks, startTicks] = [fields[1], fields[11], fields[12], fields[19]].map(Number);
  return { pid, parent, cpuTicks: userTicks + systemTicks, startTicks };
}

function missingProcess(error: NodeJS.ErrnoException): undefined {
  if (error.code !== 'ENOENT' && error.code !== 'ESRCH') throw error;
}

async function processTree(pid: number): Promise<ProcessSample[]> {
  const entries = await readdir('/proc');
  const samples = (await Promise.all(entries.filter(entry => /^\d+$/.test(entry)).map(entry => readProcess(Number(entry)))))
    .filter((sample): sample is ProcessSample => sample !== undefined);
  const children = new Map<number, ProcessSample[]>();
  for (const sample of samples) {
    const siblings = children.get(sample.parent) ?? [];
    siblings.push(sample);
    children.set(sample.parent, siblings);
  }
  const main = samples.find(sample => sample.pid === pid);
  if (!main) throw new Error('Refract exited during sampling. See app.log.');
  const tree = [main];
  for (const process of tree) tree.push(...(children.get(process.pid) ?? []));
  return tree;
}

async function memory(tree: ProcessSample[]): Promise<number> {
  const values = await Promise.all(tree.map(async ({ pid }) => {
    const smaps = await readFile(`/proc/${pid}/smaps_rollup`, 'utf8').catch(missingProcess);
    if (smaps === undefined) return 0;
    const pss = smaps.match(/^Pss:\s+(\d+)\s+kB$/m)?.[1];
    if (pss === undefined) throw new Error(`Missing PSS in /proc/${pid}/smaps_rollup.`);
    return Number(pss);
  }));
  return values.reduce((sum, value) => sum + value, 0);
}

async function measure(pid: number, ticksPerSecond: number): Promise<Measurement> {
  // Leave initial layout, PDF rendering, and renderer scheduling out of the idle window.
  await delay(2_000);
  const before = await processTree(pid);
  const start = performance.now();
  const first = await memory(before);
  await delay(Math.max(0, start + idleMs / 2 - performance.now()));
  const middle = await processTree(pid);
  const second = await memory(middle);
  await delay(Math.max(0, start + idleMs - performance.now()));
  const after = await processTree(pid);
  const idleDurationMs = performance.now() - start;
  const earlier = new Map(before.map(sample => [sample.pid, sample]));
  const idleCpuTicks = after.reduce((sum, sample) => {
    const previous = earlier.get(sample.pid);
    return sum + sample.cpuTicks - (previous?.startTicks === sample.startTicks ? previous.cpuTicks : 0);
  }, 0);
  return {
    pssKiB: [first, second, await memory(after)],
    processCounts: [before.length, middle.length, after.length],
    idleDurationMs,
    idleCpuMs: idleCpuTicks * 1_000 / ticksPerSecond,
  };
}

async function containsInk(screenshot: string, crop: string, minimum: number): Promise<boolean> {
  const { stdout } = await run('convert', [screenshot, '-crop', crop, '+repage', '-colorspace', 'gray', '-depth', '8', 'gray:-'], {
    encoding: 'buffer', timeout: 5_000,
  });
  let darkPixels = 0;
  for (const pixel of stdout) {
    if (pixel < 110 && ++darkPixels >= minimum) return true;
  }
  return false;
}

async function verifyView(app: ChildProcess, client: string, shot: string, view: View): Promise<void> {
  // Both desktop parity fixtures place their three titles and the rendered PDF ink in these regions.
  const crops = view === 'library' ? ['270x26+225+432', '270x26+225+517', '270x26+225+602'] : ['340x152+915+183'];
  await until(`the ${width}×${height} ${view}`, app, async () => {
    await x11('import', ['-window', client, shot]);
    if (await x11('identify', ['-format', '%wx%h', shot]) !== `${width}x${height}`) return;
    const matches = await Promise.all(crops.map(crop => containsInk(shot, crop, view === 'library' ? 35 : 180)));
    if (matches.every(Boolean)) return true;
  });
}

async function seedProfile(profile: string): Promise<{ pdf: Buffer; tools: string }> {
  const pdf = Buffer.from(await readFile(`${fixture}.pdf.b64`, 'utf8'), 'base64');
  const pdfPath = path.join(profile, 'fixture.pdf');
  const tools = path.join(profile, 'fixture-tex');
  await mkdir(tools, { recursive: true });
  await writeFile(pdfPath, pdf);
  // Feed the same precompiled PDF to both renderers and keep TeX execution outside the comparison.
  await writeFile(path.join(tools, 'pdflatex'), `#!/bin/sh
set -eu
for argument do
  case "$argument" in
    -output-directory=*) output_directory=\${argument#*=} ;;
  esac
done
: "\${output_directory:?Missing fixture output directory}"
cp "$(dirname "$0")/../fixture.pdf" "$output_directory/main.pdf"
`, { mode: 0o755 });
  await Promise.all(names.map(async (name, index) => {
    const id = `${projectId.slice(0, -1)}${index}`;
    const project = path.join(profile, 'projects', id);
    await mkdir(path.join(project, 'sections'), { recursive: true });
    await Promise.all([
      copyFile(`${fixture}.tex`, path.join(project, 'main.tex')),
      writeFile(path.join(project, 'sections/abstract.tex'), '% Research notes\nA stable spectral approximation.\n'),
      writeFile(path.join(project, '.refract.json'), JSON.stringify({ id, name, created: '2026-01-01T12:00:00.000Z', rootFile: 'main.tex' })),
    ]);
  }));
  return { pdf, tools };
}

function signalGroup(pid: number, signal: NodeJS.Signals): void {
  try {
    process.kill(-pid, signal);
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code !== 'ESRCH') throw error;
  }
}

async function trial(runtime: Runtime, output: string): Promise<void> {
  if (!process.env.DISPLAY || !process.env.XAUTHORITY) throw new Error('The trial requires a private Xvfb display.');
  await mkdir(output, { recursive: true });
  const scratch = await mkdtemp(path.join(tmpdir(), 'refract-benchmark-'));
  try {
    const profile = path.join(scratch, 'profile');
    const { pdf, tools } = await seedProfile(profile);
    const runtimeDir = path.join(scratch, 'runtime');
    await mkdir(runtimeDir, { mode: 0o700 });
    const env: NodeJS.ProcessEnv = {
      ...process.env,
      REFRACT_DATA_DIR: profile,
      REFRACT_TEX_BIN: tools,
      XDG_CONFIG_HOME: path.join(scratch, 'config'),
      XDG_CACHE_HOME: path.join(scratch, 'cache'),
      XDG_DATA_HOME: path.join(scratch, 'data'),
      XDG_RUNTIME_DIR: runtimeDir,
      WAYLAND_DISPLAY: '',
      LIBGL_ALWAYS_SOFTWARE: '1',
      ZED_ALLOW_EMULATED_GPU: '1',
    };
    delete env.REFRACT_TEST_HIDDEN;
    delete env.ELECTRON_RUN_AS_NODE;
    const executable = runtime === 'native' ? 'native/target/release/refract-native' : 'node_modules/electron/dist/electron';
    const program = path.join(root, executable);
    await access(program);
    const log = await open(path.join(output, 'app.log'), 'w');
    const app = spawn(program, runtime === 'electron' ? ['--no-sandbox', root] : [], {
      cwd: root, env, detached: true, stdio: ['ignore', log.fd, log.fd],
    });
    app.on('error', error => console.error(error));
    const closed = new Promise<void>(resolve => app.once('close', () => resolve()));
    const pid = app.pid;
    try {
      await log.close();
      if (!pid) throw new Error(`Could not start ${runtime}.`);
      const ticksPerSecond = Number(await x11('getconf', ['CLK_TCK']));
      if (!(ticksPerSecond > 0)) throw new Error('Could not determine the /proc clock tick duration.');
      const client = await until('the Refract desktop window', app, async () => {
        const found = await x11('xdotool', ['search', '--onlyvisible', '--pid', String(pid)]).catch(error => {
          if (error.code === 1 && !error.stderr) return '';
          throw error;
        });
        return found.split('\n')[0] || undefined;
      });
      await x11('xdotool', ['windowmove', client, '0', '0', 'windowsize', client, String(width), String(height)]);
      await x11('xdotool', ['windowfocus', client, 'mousemove', '0', '0']);
      await verifyView(app, client, path.join(output, 'library.png'), 'library');
      const library = await measure(pid, ticksPerSecond);
      await x11('xdotool', ['mousemove', '--window', client, '340', '615', 'click', '1', 'mousemove', '0', '0']);
      await until('the shared document PDF', app, async () => {
        const rendered = await readFile(path.join(profile, 'projects', projectId, '.refract-build/main.pdf')).catch(missingProcess);
        if (rendered?.equals(pdf)) return true;
      });
      await verifyView(app, client, path.join(output, 'workspace.png'), 'workspace');
      const workspace = await measure(pid, ticksPerSecond);
      const sample: Trial = { runtime, library, workspace };
      await writeFile(path.join(output, 'sample.json'), JSON.stringify(sample, null, 2));
    } finally {
      if (pid) {
        signalGroup(pid, 'SIGTERM');
        await Promise.race([closed, delay(3_000, undefined, { ref: false })]);
        signalGroup(pid, 'SIGKILL');
      }
    }
  } finally {
    await rm(scratch, { recursive: true, force: true });
  }
}

function median(values: number[]): number {
  const sorted = [...values].sort((left, right) => left - right);
  const middle = Math.floor(sorted.length / 2);
  return (sorted[middle] + sorted[Math.floor((sorted.length - 1) / 2)]) / 2;
}

async function benchmark(output: string): Promise<void> {
  const trials: Trial[] = [];
  for (let round = 1; round <= rounds; round++) {
    const order: Runtime[] = round % 2 === 1 ? ['native', 'electron'] : ['electron', 'native'];
    for (const runtime of order) {
      const folder = path.join(output, `${runtime}-${round}`);
      console.log(`Running ${runtime} trial ${round}/${rounds} in a private Xvfb display.`);
      await run('xvfb-run', ['-a', '-s', '-screen 0 1600x1000x24', process.execPath, __filename, '--trial', runtime, folder], { timeout: 135_000 });
      trials.push(JSON.parse(await readFile(path.join(folder, 'sample.json'), 'utf8')) as Trial);
    }
  }
  const commit = (await run('git', ['rev-parse', 'HEAD'], { cwd: root })).stdout.trim();
  await writeFile(path.join(output, 'benchmark.json'), JSON.stringify({ commit, arch: arch(), kernel: release(), width, height, idleMs, trials }, null, 2));
  const rows = (['library', 'workspace'] as const).map(view => {
    const compare = (runtime: Runtime) => {
      const samples = trials.filter(trial => trial.runtime === runtime).map(trial => trial[view]);
      return {
        memory: median(samples.map(sample => median(sample.pssKiB))) / 1024,
        cpu: median(samples.map(sample => 100 * sample.idleCpuMs / sample.idleDurationMs)),
      };
    };
    const electron = compare('electron');
    const native = compare('native');
    const saved = 100 * (electron.memory - native.memory) / electron.memory;
    return `| ${view} | ${electron.memory.toFixed(1)} | ${native.memory.toFixed(1)} | ${saved.toFixed(1)}% | ${electron.cpu.toFixed(2)}% | ${native.cpu.toFixed(2)}% |`;
  });
  const report = [
    '## Desktop resource benchmark',
    '',
    `Refract ${commit.slice(0, 12)} on one hosted Linux runner. Medians from ${rounds} fresh-profile trials per build.`,
    '',
    '| View | Electron PSS (MiB) | GPUI PSS (MiB) | PSS saved | Electron idle CPU | GPUI idle CPU |',
    '| --- | ---: | ---: | ---: | ---: | ---: | ---: |',
    ...rows,
    '',
    `Each build used a private Xvfb screen, software graphics, and the same ${width}×${height} library and PDF fixture. Electron ran with its CI sandbox disabled.`,
    'PSS apportions shared pages across processes. Idle CPU is sampled app-tree CPU time divided by window duration, as a percentage of one core. CPU from processes that exit within a sample can be missed.',
    'These results cover the library and PDF workspace during idle use. The run uses a precompiled PDF, fresh profiles, and no signed-in assistants.',
    '',
  ].join('\n');
  await writeFile(path.join(output, 'benchmark.md'), report);
  if (process.env.GITHUB_STEP_SUMMARY) await writeFile(process.env.GITHUB_STEP_SUMMARY, report, { flag: 'a' });
  console.log(report);
}

async function main(): Promise<void> {
  if (process.platform !== 'linux') throw new Error('Run the benchmark on the hosted Linux runner.');
  const [mode, runtime, folder] = process.argv.slice(2);
  if (mode === '--trial' && (runtime === 'native' || runtime === 'electron') && folder) return trial(runtime, folder);
  if (!mode || runtime) throw new Error('Pass one output folder for the benchmark.');
  return benchmark(path.resolve(mode));
}

void main().catch(error => {
  console.error(error);
  process.exitCode = 1;
});
