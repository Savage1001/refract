import assert = require('node:assert/strict');
import fs = require('node:fs/promises');
import os = require('node:os');
import path = require('node:path');
import nodeTest = require('node:test');
import assistantTools = require('../src/assistant-tools.cjs');
import builds = require('../src/build-cache.cjs');
import buildBridge = require('../src/assistant-build-bridge.cjs');
import codex = require('../src/codex-runtime.cjs');
import toolchain = require('../src/tex-toolchain.cjs');
const {ChatService} = require('../src/chat.cjs');
const {compile} = require('../src/compiler.cjs');
const {ProjectStore} = require('../src/project-store.cjs');

const plain = String.raw`\documentclass{article}
\begin{document}
A sample with the integral $\int_0^1 x^2\,dx$.
\end{document}
`;
const cases = [
  {name: 'keeps pdfLaTeX for a standard document', initial: 'pdflatex', expected: 'pdflatex', source: plain,
    prompt: 'Build this paper using the fastest suitable compiler. Keep its content and layout.'},
  {name: 'switches a standard document from LuaLaTeX to pdfLaTeX', initial: 'lualatex', expected: 'pdflatex', source: plain,
    prompt: 'The project has slow builds. Please use the fastest compatible LaTeX compiler and compile the paper. Leave the source alone.'},
  {name: 'keeps XeLaTeX for a template that requires it', initial: 'xelatex', expected: 'xelatex',
    source: '% !TEX program = xelatex\n' + String.raw`\documentclass{article}
\usepackage{fontspec}
\setmainfont{Latin Modern Roman}
\begin{document}
A template with an OpenType font.
\end{document}
`,
    prompt: 'Build this paper as fast as possible and follow the template’s compiler requirement. Preserve its font and source.'},
  {name: 'switches from pdfLaTeX to LuaLaTeX when the source needs Lua', initial: 'pdflatex', expected: 'lualatex',
    source: String.raw`\documentclass{article}
\usepackage{fontspec}
\setmainfont{Latin Modern Roman}
\begin{document}
This line uses Lua: \directlua{tex.sprint("hello from Lua")}.
\end{document}
`,
    prompt: 'Compile this document using the fastest compiler that supports everything used in the source. Leave the source unchanged.'}
] as const;

nodeTest.describe('GPT-6 Luna at medium reasoning chooses a compatible engine', {
  concurrency: 2, skip: process.env.REFRACT_REAL_ENGINE_SELECTION_TEST !== '1'
}, () => {
  for (const fixture of cases) nodeTest.test(fixture.name, {timeout: 150000}, async t => {
    const parent = await fs.realpath(os.tmpdir());
    const root = await fs.mkdtemp(path.join(parent, 'refract-real-engine-'));
    t.after(async () => {
      assert.equal(path.dirname(await fs.realpath(root)), parent);
      await fs.rm(root, {recursive: true, force: true, maxRetries: 5, retryDelay: 200});
    });
    const store = new ProjectStore(root);
    const project = await store.create('Engine selection fixture');
    const cwd = store.projectDir(project.id);
    await store.write(project.id, 'main.tex', fixture.source);
    await store.setEngine(project.id, fixture.initial);
    const runtime = new codex.CodexRuntime(cwd);
    const models: string[] = [];
    const reroutes: string[] = [], efforts: unknown[] = [], toolCalls: string[] = [];
    const request = runtime.request.bind(runtime);
    runtime.request = async (...args) => {
      const result = await request(...args);
      if (args[0] === 'thread/start' && typeof result.model === 'string') models.push(result.model);
      if (args[0] === 'thread/start') assert.match((args[1] as {developerInstructions: string}).developerInstructions, /Choose the fastest LaTeX engine/);
      if (args[0] === 'turn/start') efforts.push((args[1] as {effort?: string}).effort);
      return result;
    };
    runtime.on('notification', event => {
      if (/rerout/i.test(event.method)) reroutes.push(event.method);
      if (event.method === 'item/completed' && event.params?.item?.type === 'commandExecution') toolCalls.push('shell');
      if (event.method === 'item/completed' && event.params?.item?.type === 'mcpToolCall') {
        const {server, tool} = event.params.item;
        if (server === 'refract' && typeof tool === 'string') toolCalls.push(tool);
      }
    });
    const bin = toolchain.texBin();
    const operations: Array<{engine: string; kind: 'build' | 'set'; ok?: boolean}> = [];
    const cache = new builds.BuildCache(store, compile);
    const tools = await assistantTools.startAssistantTools(store, project.id, async force => {
      const options = await buildBridge.configuredBuild(store, project.id, bin);
      const result = await cache.run(project.id, cwd, options, force);
      operations.push({engine: options.engine, kind: 'build', ok: result.ok});
      return {engine: options.engine, rootFile: options.rootFile, ok: result.ok, log: result.log};
    }, name => buildBridge.lookupPackage(name, cwd, bin), {
      initialEngine: fixture.initial,
      setEngine: async engine => {
        const previous = (await store.metadata(project.id)).engine;
        const changed = previous !== engine;
        if (changed) await store.setEngine(project.id, engine);
        operations.push({engine, kind: 'set'});
        return {engine, previousEngine: toolchain.isLatexEngine(previous) ? previous : null, changed};
      }
    });
    try {
      const chat = await new ChatService(store, (_cwd: string, ...args: Parameters<typeof runtime.run>) => runtime.run(...args)).send(
        project.id, fixture.prompt, 'default', [], AbortSignal.timeout(120000), undefined,
        {model: 'gpt-6-luna', reasoningEffort: 'medium', fastMode: false, tools: tools.connection,
          developerInstructions: tools.engineInstructions, hasSuccessfulBuild: tools.hasSuccessfulBuild}, 'codex', tools.instructions
      );
      const answer = chat.messages.at(-1);
      t.diagnostic(JSON.stringify({models, efforts, toolCalls, operations}));
      if (!operations.some(op => op.kind === 'build')) t.diagnostic(`Reply without a build: ${answer?.text?.replaceAll(cwd, '<fixture>')}`);
      assert.equal(answer?.role, 'assistant', answer?.text);
      assert.deepEqual(models, ['gpt-6-luna']);
      assert.ok(efforts.length >= 1 && efforts.length <= 2 && efforts.every(effort => effort === 'medium'));
      assert.deepEqual(reroutes, []);
      assert.equal((await store.metadata(project.id)).engine, fixture.expected);
      assert.deepEqual(operations.filter(op => op.kind === 'set').map(op => op.engine),
        fixture.initial === fixture.expected ? [] : [fixture.expected]);
      assert.deepEqual(operations.at(-1), {engine: fixture.expected, kind: 'build', ok: true});
      assert.match(answer.text, new RegExp(fixture.expected, 'i'));
      assert.deepEqual(await store.files(project.id), ['main.tex']);
      assert.equal(await store.read(project.id, 'main.tex'), fixture.source);
      assert.equal((await fs.readFile(path.join(cwd, '.refract-build', 'main.pdf'))).subarray(0, 5).toString(), '%PDF-');
    } finally {
      await runtime.close();
      await tools.close();
    }
  });
});
