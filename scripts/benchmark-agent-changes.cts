import assert = require('node:assert/strict');
import fs = require('node:fs/promises');
import os = require('node:os');
import path = require('node:path');
import childProcess = require('node:child_process');
import runtime = require('../src/codex-runtime.cjs');
import tools = require('../src/assistant-tools.cjs');
import performanceMetrics = require('../src/assistant-performance.cjs');
const chat = require('../src/chat.cjs');
const coordinator = require('../src/assistant-coordinator.cjs');
const projectStore = require('../src/project-store.cjs');
const versions = require('../src/versions.cjs');
const compiler = require('../src/compiler.cjs');
import buildCache = require('../src/build-cache.cjs');
import buildBridge = require('../src/assistant-build-bridge.cjs');
import toolchain = require('../src/tex-toolchain.cjs');

const model = 'gpt-6-luna';
const effort = 'medium';
type Files = Record<string, string>;
interface Turn { prompt: string; expected: Files; build?: boolean }
interface Scenario { id: string; files: Files; turns: Turn[] }
const article = (body: string, packages = '') => [
  '\\documentclass{article}', packages, '\\begin{document}', body, '\\end{document}', '',
].join('\n');
const methods = '\\section{Methods}\r\nThe instrument was calibrated daily.\r\nThe sample was heated.\r\n';
const results = '\\section{Results}\nThe measured threshold was 7 mV.\n';
const repeated = '\\section{Control}\r\nStatus: pending.\r\n\\section{Treatment}\r\nStatus: pending.\r\n';
const appendix = '% Calibration notes retained for audit.\n'.repeat(1024) + '\\section{Reference readings}\nReference voltage: 4.5 V.\n';
const scenarios: Scenario[] = [
  {id: 'title', files: {'main.tex': article('\\section{Initial measurements}\nA summary follows.')}, turns: [{
    prompt: 'In main.tex, rename the section Initial measurements to Baseline measurements. Preserve the rest of the file.',
    expected: {'main.tex': article('\\section{Baseline measurements}\nA summary follows.')},
  }]},
  {id: 'sections', files: {'main.tex': article('\\input{sections/methods}\n\\input{sections/results}'), 'sections/methods.tex': methods, 'sections/results.tex': results}, turns: [{
    prompt: 'In the methods section, change daily calibration to weekly calibration and change the sentence about heating to “The sample was cooled.” In the results section, update 7 mV to 9 mV. Preserve all other source and the existing line endings.',
    expected: {'sections/methods.tex': methods.replace('daily', 'weekly').replace('heated', 'cooled'), 'sections/results.tex': results.replace('7 mV', '9 mV')},
  }]},
  {id: 'duplicate', files: {'main.tex': article(repeated).replace(/(?<!\r)\n/g, '\r\n')}, turns: [{
    prompt: 'In main.tex, change only the Treatment section’s status from pending to approved. Leave the Control section and the existing CRLF line endings alone.',
    expected: {'main.tex': article(repeated.replace(/pending\.(?=\r\n$)/, 'approved.')).replace(/(?<!\r)\n/g, '\r\n')},
  }]},
  {id: 'appendix', files: {'main.tex': article('\\input{appendix}'), 'appendix.tex': appendix}, turns: [{
    prompt: 'In appendix.tex, change the reference voltage from 4.5 V to 5.0 V. Preserve the audit comments and all other source.',
    expected: {'appendix.tex': appendix.replace('4.5 V', '5.0 V')},
  }]},
  {id: 'build-repair', files: {'main.tex': article('\\section{Results}\nThe measurements are complete.', '\\usepackage{refract-benchmark-missing}')}, turns: [{
    prompt: 'Build the paper with Refract’s configured compiler. If it fails because an unnecessary package is missing, remove its declaration and verify a successful rebuild. Preserve all document content.',
    expected: {'main.tex': article('\\section{Results}\nThe measurements are complete.').replace('\n\n', '\n')}, build: true,
  }]},
  {id: 'follow-up', files: {'main.tex': article('Status: provisional.\nReview is scheduled for Monday.')}, turns: [{
    prompt: 'Update the status in main.tex from provisional to verified. Preserve the rest of the file.',
    expected: {'main.tex': article('Status: verified.\nReview is scheduled for Monday.')},
  }, {
    prompt: 'Move the review in that same file to Thursday and keep the previous change.',
    expected: {'main.tex': article('Status: verified.\nReview is scheduled for Thursday.')},
  }]},
];

interface Usage {inputTokens: number; cachedInputTokens: number; outputTokens: number; reasoningOutputTokens: number; totalTokens: number}
interface Notification {
  method?: string;
  params?: {tokenUsage?: {total?: Usage}; toModel?: string;
    item?: {type?: string; server?: string; tool?: string}};
}

const safeTools = new Set(['project_list', 'project_read', 'project_read_many', 'project_edit', 'project_edit_many', 'build', 'package_lookup', 'conversation_read']);

function usageDelta(current: Usage | undefined, previous: Usage): Usage | null {
  if (!current) return null;
  return {
    inputTokens: current.inputTokens - previous.inputTokens,
    cachedInputTokens: current.cachedInputTokens - previous.cachedInputTokens,
    outputTokens: current.outputTokens - previous.outputTokens,
    reasoningOutputTokens: current.reasoningOutputTokens - previous.reasoningOutputTokens,
    totalTokens: current.totalTokens - previous.totalTokens,
  };
}

async function runScenario(root: string, scenario: Scenario, bin: string) {
  const store = new projectStore.ProjectStore(root);
  const project = await store.create('Benchmark paper');
  const id: string = project.id;
  const cwd: string = await store.destinationDir(id);
  const snapshots = new versions.Versions(store);
  const cache = new buildCache.BuildCache(store, compiler.compile);
  const assistant = new coordinator.AssistantCoordinator((work: () => Promise<unknown>) => work(), async () => {}, (id: string) => snapshots.snapshot(id));
  const expected = {...scenario.files};
  for (const [file, content] of Object.entries(expected)) await store.write(id, file, content);
  await snapshots.snapshot(id);
  let server: InstanceType<typeof runtime.CodexRuntime> | undefined;
  let usage: Usage | undefined;
  let rerouted = false;
  let calls: string[];
  let started: number;
  let lastEditMs: number | undefined;
  let modelCompletedMs: number | undefined;
  const run: typeof runtime.runWarmCodex = async (directory, session, prompt, images, signal, progress, save, options) => {
    if (!server?.available) {
      server = new runtime.CodexRuntime(directory);
      server.on('notification', ({method, params}: Notification) => {
        if (method === 'thread/tokenUsage/updated') usage = params?.tokenUsage?.total;
        if (method === 'model/rerouted' && params?.toModel !== model) rerouted = true;
        if (method === 'turn/completed') modelCompletedMs = performance.now() - started;
        if (method === 'item/completed' && params?.item) {
          const {type, server: name, tool} = params.item;
          if (type === 'mcpToolCall') {
            calls.push(name === 'refract' && safeTools.has(tool) ? tool : 'other_mcp');
            if (name === 'refract' && (tool === 'project_edit' || tool === 'project_edit_many')) lastEditMs = performance.now() - started;
          } else if (type === 'fileChange' || type === 'commandExecution' || type === 'webSearch') calls.push(type);
        }
      });
    }
    return server.run(session, prompt, images, signal, progress, save, options);
  };
  const service = new chat.ChatService(store, run);
  let priorUsage: Usage = {inputTokens: 0, cachedInputTokens: 0, outputTokens: 0, reasoningOutputTokens: 0, totalTokens: 0};
  try {
    const rows = [];
    for (const turn of scenario.turns) {
      usage = undefined;
      rerouted = false;
      calls = [];
      lastEditMs = undefined;
      modelCompletedMs = undefined;
      const builds: boolean[] = [];
      started = performance.now();
      const metrics = new performanceMetrics.AssistantPerformance();
      const deadline = setTimeout(() => assistant.cancel(id), 90000);
      let role = 'error';
      let error: string | undefined;
      try {
        const conversation = await assistant.run(id, async (signal: AbortSignal) => {
          const bridge = await tools.startAssistantTools(store, id, async (force = false) => {
            const settings = await buildBridge.configuredBuild(store, id, bin, signal);
            const result = await metrics.measure('build', () => cache.run(id, cwd, settings, force));
            builds.push(result.ok);
            return {
              id: 'benchmark-build', cached: result.cached, engine: settings.engine, rootFile: settings.rootFile,
              cwd, outputDirectory: path.join(cwd, '.refract-build'), arguments: result.arguments,
              environment: 'Configured TeX bin prepended to PATH; BIBINPUTS includes the project recursively.',
              pdfPath: result.ok ? path.join(cwd, '.refract-build', 'main.pdf') : null,
              ok: result.ok, exitCode: result.exitCode, log: result.log,
            };
          }, name => buildBridge.lookupPackage(name, cwd, bin, signal), {performance: metrics, history: () => service.load(id, 'default')});
          try {
            return await service.send(id, turn.prompt, 'default', [], signal, undefined,
              {model, reasoningEffort: effort, fastMode: false, tools: bridge.connection, performance: metrics}, 'codex', bridge.instructions);
          } finally {
            await bridge.close();
          }
        }, metrics);
        role = conversation.messages.at(-1)?.role ?? 'error';
        if (role === 'error') error = classifyError(conversation.messages.at(-1)?.text ?? '');
      } catch (failure) {
        error = classifyError(String(failure));
      } finally {
        clearTimeout(deadline);
      }
      const timing = metrics.report();
      Object.assign(expected, turn.expected);
      const files: string[] = await store.files(id);
      const extra = files.filter(file => !(file in expected));
      const different: string[] = [];
      for (const [file, content] of Object.entries(expected)) {
        if (!(await fs.readFile(path.join(cwd, file)).catch(() => null))?.equals(Buffer.from(content))) different.push(file);
      }
      const pdf = turn.build && await fs.readFile(path.join(cwd, '.refract-build', 'main.pdf')).catch(() => null);
      const buildVerified = !turn.build || (builds[0] === false && builds.at(-1) === true && pdf && pdf.subarray(0, 5).toString() === '%PDF-');
      const tokens = usageDelta(usage, priorUsage);
      if (usage) priorUsage = usage;
      rows.push({pass: role === 'assistant' && !rerouted && !extra.length && !different.length && !!buildVerified,
        error, rerouted, different, extraCount: extra.length, builds, tokens, calls, lastEditMs, modelCompletedMs, ...timing});
    }
    return rows;
  } finally {
    await server?.close();
  }
}

function classifyError(error: string) {
  // Provider messages can contain paths, account details or prompt content.
  if (/rate limit|quota|usage limit|too many requests/i.test(error)) return 'rate limit';
  if (/not supported|model.*unavailable|model.*access/i.test(error)) return 'model unavailable';
  if (/cancel|abort|interrupt/i.test(error)) return 'cancelled';
  if (/timed out|timeout/i.test(error)) return 'timeout';
  if (/auth|sign.in|login/i.test(error)) return 'authentication';
  return 'turn failed';
}

async function main() {
  const [output, repetitionsArg = '3', orderArg = '0', only] = process.argv.slice(2);
  const repetitions = Number(repetitionsArg), order = Number(orderArg);
  assert.ok(output && Number.isInteger(repetitions) && repetitions >= 1 && repetitions <= 30 && Number.isInteger(order) && order >= 0);
  const selected = only ? scenarios.filter(scenario => scenario.id === only) : scenarios;
  assert.ok(selected.length, 'Expected a known scenario ID');
  const bin = toolchain.texBin();
  await fs.access(path.join(bin, 'pdflatex.exe'));
  const parent = await fs.realpath(os.tmpdir());
  const root = await fs.mkdtemp(path.join(parent, 'refract-change-bench-'));
  const cli = path.join(process.env.APPDATA || '', 'npm/node_modules/@openai/codex/bin/codex.js');
  const report = {
    date: new Date().toISOString(), source: process.env.REFRACT_BENCH_SOURCE ?? childProcess.execFileSync('git', ['rev-parse', 'HEAD']).toString().trim(),
    platform: process.platform, cpu: os.cpus()[0].model, node: process.version,
    cli: childProcess.execFileSync(process.execPath, [cli, '--version'], {windowsHide: true}).toString().trim(),
    model, effort, serviceTier: 'default', samples: [] as Array<{rep: number; scenario: string; turns: Awaited<ReturnType<typeof runScenario>>}>,
  };
  try {
    for (let rep = order; rep < order + repetitions; rep++) {
      const start = rep % selected.length;
      for (const scenario of [...selected.slice(start), ...selected.slice(0, start)]) {
        const turns = await runScenario(root, scenario, bin);
        report.samples.push({rep, scenario: scenario.id, turns});
        await fs.writeFile(output, JSON.stringify(report, null, 2) + '\n');
        console.log(rep, scenario.id, turns.map(turn => ({pass: turn.pass, ms: Math.round(turn.totalMs), tokens: turn.tokens?.totalTokens, error: turn.error, diff: turn.different})));
      }
    }
  } finally {
    const target = await fs.realpath(root);
    assert.equal(path.dirname(target), parent);
    assert.ok(path.basename(target).startsWith('refract-change-bench-'));
    await fs.rm(target, {recursive: true, force: true, maxRetries: 5, retryDelay: 200});
  }
}

void main().catch(error => {console.error(error); process.exitCode = 1;});
