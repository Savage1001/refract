import assert = require('node:assert/strict');
import fs = require('node:fs/promises');
import os = require('node:os');
import path = require('node:path');
import compiler = require('../src/compiler.cjs');
import builds = require('../src/build-cache.cjs');
import images = require('../src/document-images.cjs');
import toolchain = require('../src/tex-toolchain.cjs');
const {ProjectStore} = require('../src/project-store.cjs');

const plain = String.raw`\documentclass{article}
\usepackage{amsmath,graphicx}
\begin{document}
An integral: $\int_0^1 x^2\,dx$.
\end{document}
`;
const references = String.raw`\documentclass{article}
\begin{document}
\tableofcontents
\section{Example}\label{sec:example}
See Section~\ref{sec:example}.
\end{document}
`;
const bibliography = String.raw`\documentclass{article}
\begin{document}
See \cite{example}.
\bibliographystyle{plain}
\bibliography{references}
\end{document}
`;
const biber = String.raw`\documentclass{article}
\usepackage[backend=biber]{biblatex}
\addbibresource{references.bib}
\begin{document}
See \cite{example}.
\printbibliography
\end{document}
`;

async function benchmark(): Promise<void> {
  const [baselineBuild, count = '12', filter] = process.argv.slice(2);
  const samples = Number(count);
  assert.ok(baselineBuild && Number.isSafeInteger(samples) && samples >= 2,
    'Usage: node build/scripts/benchmark-compilation.cjs <baseline build directory> [samples] [case filter]');
  const baseline = path.resolve(baselineBuild, 'src');
  const oldCompiler: typeof compiler = require(path.join(baseline, 'compiler.cjs'));
  const oldBuilds: typeof builds = require(path.join(baseline, 'build-cache.cjs'));
  const oldImages: typeof images = require(path.join(baseline, 'document-images.cjs'));
  const variants = [
    {name: 'baseline', compiler: oldCompiler, builds: oldBuilds, images: oldImages},
    {name: 'compilerOnly', compiler, builds: oldBuilds, images: oldImages},
    {name: 'optimized', compiler, builds, images}
  ];
  const cases = [
    {name: 'pdfLaTeX one page (forced)', engine: 'pdflatex', source: plain},
    {name: 'XeLaTeX one page (forced)', engine: 'xelatex', source: plain},
    {name: 'LuaLaTeX one page (forced)', engine: 'lualatex', source: plain},
    {name: 'Table of contents and reference (forced)', engine: 'pdflatex', source: references},
    {name: 'BibTeX (forced)', engine: 'pdflatex', source: bibliography},
    {name: 'Biber (forced)', engine: 'pdflatex', source: biber},
    {name: 'Project with 121 TeX files (forced)', engine: 'pdflatex', source: plain.replace('An integral:', '\\input{sections/1/part0}\nAn integral:'), large: true},
    {name: 'Unchanged project with 121 TeX files (cached)', engine: 'pdflatex', source: plain, large: true, cached: true}
  ].filter(fixture => !filter || fixture.name.toLowerCase().includes(filter.toLowerCase()));
  assert.ok(cases.length, 'No benchmark cases match the filter');
  const root = await fs.mkdtemp(path.join(await fs.realpath(os.tmpdir()), 'refract-bench-compile-'));
  // Freeze the cache's clock for reproducible hits across minute boundaries.
  // TeX runs in separate processes and retains its real system clock.
  const RealDate = Date;
  const fixedTime = Date.now();
  globalThis.Date = class extends RealDate {
    constructor() { super(fixedTime); }
    static now() { return fixedTime; }
  } as DateConstructor;
  try {
    for (const fixture of cases) {
      const executable = path.join(toolchain.texBin(), fixture.engine + '.exe');
      const runners = [];
      const caseDir = await fs.mkdtemp(path.join(root, 'case-'));
      for (const variant of variants) {
        const store = new ProjectStore(path.join(caseDir, variant.name));
        const {id} = await store.create('Benchmark paper');
        const cwd = store.projectDir(id);
        await store.write(id, 'main.tex', fixture.source);
        if (fixture.source === bibliography || fixture.source === biber) {
          await store.write(id, 'references.bib', '@article{example,author={Ada Lovelace},title={A Paper},journal={Journal},year={1843}}');
        }
        if (fixture.large) {
          for (let index = 0; index < 120; index++) {
            await store.write(id, `sections/${1 + index % 12}/part${index}.tex`, 'Example text.\n' + '% Project notes.\n'.repeat(750));
          }
        }
        const cache = new variant.builds.BuildCache(store, variant.compiler.compile);
        const run = async (force: boolean) => {
          const start = performance.now();
          await variant.images.materializeDocumentImages(store, id);
          const result = await cache.run(id, cwd, {executable, rootFile: 'main.tex'}, force);
          const ms = performance.now() - start;
          assert.ok(result.ok && result.pdf, result.log);
          const passes = result.cached ? 0 : (result.log.match(/^This is (?:pdfTeX|XeTeX|LuaHBTeX)/gm) || []).length;
          return {ms, passes, cached: result.cached};
        };
        await run(true);
        await run(true);
        runners.push({name: variant.name, run, measurements: [] as Array<Awaited<ReturnType<typeof run>>>});
      }
      // Rotate order to limit bias from OS caching, power and thermal changes.
      for (let sample = 0; sample < samples; sample++) {
        for (let index = 0; index < runners.length; index++) {
          const runner = runners[(sample + index) % runners.length];
          const result = await runner.run(!fixture.cached);
          assert.equal(result.cached, !!fixture.cached, fixture.name);
          runner.measurements.push(result);
        }
      }
      console.log(JSON.stringify({case: fixture.name, variants: runners.map(({name, measurements}) => {
        const sorted = measurements.map(value => value.ms).sort((a, b) => a - b);
        const median = (sorted[Math.floor((samples - 1) / 2)] + sorted[Math.ceil((samples - 1) / 2)]) / 2;
        return {name, medianMs: +median.toFixed(2), passes: [...new Set(measurements.map(value => value.passes))],
          samplesMs: measurements.map(value => +value.ms.toFixed(2))};
      })}));
    }
  } finally {
    globalThis.Date = RealDate;
    assert.equal(path.dirname(await fs.realpath(root)), await fs.realpath(os.tmpdir()));
    await fs.rm(root, {recursive: true, force: true, maxRetries: 5, retryDelay: 200});
  }
}

benchmark().catch(error => {
  console.error(error);
  process.exitCode = 1;
});
