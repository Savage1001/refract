import fs = require('node:fs/promises');
import path = require('node:path');
import os = require('node:os');
import assert = require('node:assert/strict');
import cp = require('node:child_process');
import clientSdk = require('@modelcontextprotocol/sdk/client/index.js');
import transportSdk = require('@modelcontextprotocol/sdk/client/streamableHttp.js');
import tools = require('../src/assistant-tools.cjs');
import legacyBridge = require('../src/assistant-build-bridge.cjs');
const {ProjectStore} = require('../src/project-store.cjs');
const {Client} = clientSdk, {StreamableHTTPClientTransport} = transportSdk;

const samples = Number(process.env.REFRACT_BENCH_SAMPLES || 30);
const warmups = 3;
const shell = path.join(process.env.SystemRoot || 'C:/Windows','System32/WindowsPowerShell/v1.0/powershell.exe');
const summarize = (values: number[]) => {
  const sorted = [...values].sort((a,b) => a-b);
  return {medianMs: (sorted[Math.floor((sorted.length-1)/2)] + sorted[Math.floor(sorted.length/2)])/2,p95Ms:sorted[Math.ceil(sorted.length*.95)-1],minMs:sorted[0],maxMs:sorted.at(-1),samplesMs:values};
};
async function main() {
  assert.equal(process.platform,'win32','This benchmark measures the Windows shell path');
  assert.ok(Number.isInteger(samples) && samples >= 10 && samples <= 1000);
  const parent = await fs.realpath(os.tmpdir()), root = await fs.mkdtemp(path.join(parent,'refract-tool-bench-'));
  const store = new ProjectStore(root), project = await store.create('Benchmark'), cwd = store.projectDir(project.id);
  const text = '% benchmark\r\n' + 'A realistic paper line with α and β.\r\n'.repeat(1024);
  const fixtureKiB = (Buffer.byteLength(text)/1024).toFixed(1);
  await store.write(project.id,'main.tex',text);
  const started = performance.now();
  const bridge = await tools.startAssistantTools(store, project.id,async()=>({ok:true}),async()=>({available:true}));
  const client = new Client({name:'refract-benchmark',version:'1.0.0'});
  await client.connect(new StreamableHTTPClientTransport(new URL(bridge.connection.url),{requestInit:{headers:{Authorization:`Bearer ${bridge.connection.token}`}}}));
  const setupMs = performance.now()-started;
  const legacy = await legacyBridge.startAssistantBuildBridge(async()=>({ok:true}),async()=>({available:true}));
  const call = async (name: string, args: Record<string,unknown> = {}) => {
    const reply = await client.callTool({name,arguments:args});
    assert.ok(!reply.isError,JSON.stringify(reply));
    return JSON.parse((reply.content as Array<{text:string}>)[0].text);
  };
  const runShell = (command: string, noProfile = false) => new Promise<string>((resolve,reject) => {
    const child = cp.spawn(shell,[...(noProfile ? ['-NoProfile'] : []),'-NonInteractive','-Command',command],{cwd,windowsHide:true});
    let stdout = '', stderr = ''; child.stdout.on('data',chunk=>stdout+=chunk); child.stderr.on('data',chunk=>stderr+=chunk);
    const timer = setTimeout(()=>{child.kill();reject(new Error('Shell timed out'));},15000);
    child.once('error',error=>{clearTimeout(timer);reject(error);});
    child.once('close',code=>{clearTimeout(timer);code === 0 ? resolve(stdout) : reject(new Error(stderr));});
  });
  const rows: Array<{operation:string;before:ReturnType<typeof summarize>;after:ReturnType<typeof summarize>}> = [];
  try {
    let current = await call('project_read',{path:'main.tex',limit:65536});
    const operations = [
      {name:'List project files (1 file)',before:async()=>{assert.match(await runShell("Get-ChildItem -File -Recurse | Where-Object { $_.Name -notlike '.*' } | ForEach-Object { $_.Name }"),/main.tex/);},after:async()=>{assert.deepEqual((await call('project_list')).files,['main.tex']);}},
      {name:`Read text (${fixtureKiB} KiB UTF-8)`,before:async()=>{const output=await runShell("[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false); [Console]::Write([IO.File]::ReadAllText((Join-Path (Get-Location) 'main.tex')))");assert.equal(output,text);},after:async()=>{current=await call('project_read',{path:'main.tex',limit:65536});assert.equal(current.content,text);}},
      {name:`Verified exact edit (${fixtureKiB} KiB UTF-8)`,before:async()=>{await runShell("$p=Join-Path (Get-Location) 'main.tex'; $s=[IO.File]::ReadAllText($p); $s=$s.Replace('% benchmark','% changed'); [IO.File]::WriteAllText($p,$s,[Text.UTF8Encoding]::new($false)); if ([IO.File]::ReadAllText($p) -cne $s) { exit 1 }");assert.equal(await store.read(project.id,'main.tex'),text.replace('% benchmark','% changed'));},after:async()=>{const edited=await call('project_edit',{path:'main.tex',sha256:current.sha256,oldText:'% benchmark',newText:'% changed'});assert.equal(edited.applied,true);assert.equal(await store.read(project.id,'main.tex'),text.replace('% benchmark','% changed'));}},
      {name:'PowerShell no-op (login vs no profile)',before:async()=>{assert.match(await runShell("[Console]::Write('ready')"),/ready/);},after:async()=>{assert.match(await runShell("[Console]::Write('ready')",true),/ready/);}},
      {name:'Build dispatch (stub compiler)',before:async()=>{assert.equal(JSON.parse(await runShell(legacy.instructions.split('\n')[1])).ok,true);},after:async()=>{assert.equal((await call('build')).ok,true);}},
    ];
    for (const operation of operations) {
      const before: number[] = [], after: number[] = [];
      for (let i=-warmups;i<samples;i++) {
        // Alternate order to avoid giving either path a systematic cache advantage.
        for (const side of (i%2 ? ['after','before'] : ['before','after']) as Array<'before'|'after'>) {
          await store.write(project.id,'main.tex',text);
          current=await call('project_read',{path:'main.tex',limit:65536});
          const start=performance.now(); await operation[side](); const ms=performance.now()-start;
          if(i>=0)(side==='before' ? before : after).push(ms);
        }
      }
      rows.push({operation:operation.name,before:summarize(before),after:summarize(after)});
      console.log(JSON.stringify(rows.at(-1)));
    }
    const report={date:new Date().toISOString(),platform:process.platform,osRelease:os.release(),node:process.version,cpu:os.cpus()[0].model,samples,warmups,setupMs,scope:'Local tool execution only. Before: fresh Windows PowerShell process with default profile behavior. Build dispatch uses the original bridge and its documented command with a stub compiler. After: real MCP SDK client over authenticated loopback HTTP, or -NoProfile for shell row. Includes response validation; excludes model/network time, provider sandbox setup, CLI startup, and real compilation. Not a claim that OpenCode native tools or Codex apply_patch have these baseline timings.',rows};
    await fs.mkdir(path.resolve('docs/benchmarks'),{recursive:true});
    await fs.writeFile(path.resolve('docs/benchmarks/assistant-tools.json'),JSON.stringify(report,null,2)+'\n');
    const table = rows.map(row=>`| ${row.operation} | ${row.before.medianMs.toFixed(2)} | ${row.after.medianMs.toFixed(2)} | ${row.before.p95Ms.toFixed(2)} | ${row.after.p95Ms.toFixed(2)} | ${(row.before.medianMs/row.after.medianMs).toFixed(1)}× |`).join('\n');
    await fs.writeFile(path.resolve('docs/benchmarks/assistant-tools.md'),`# Assistant tool latency\n\nMeasured ${report.date} on Windows ${report.osRelease}, ${report.cpu}, Node ${report.node}. Base implementation: d5004ac (v0.2.1).\n\n| Operation | Before median ms | After median ms | Before p95 ms | After p95 ms | Median speedup |\n| --- | ---: | ---: | ---: | ---: | ---: |\n${table}\n\nOne-time MCP server start and SDK client initialization: ${setupMs.toFixed(2)} ms. This cost occurs once per assistant response, not once per tool. It excludes module loading at app startup and provider startup.\n\n## Method\n\nRun \`npm run benchmark:tools\` on Windows. Each path gets ${warmups} warmups and ${samples} measured samples per operation. Execution order alternates within each pair. The fixture is one ${fixtureKiB} KiB UTF-8 paper containing Unicode and CRLF line endings; it is reset outside the timer before every call. Before and after results are validated. Median is the mean of the two central sorted samples; p95 uses nearest rank. [Raw samples](assistant-tools.json) include all measurements, min/max, hardware, and setup time.\n\nBefore: fresh Windows PowerShell with its default profile behavior. The build row executes the original bridge's documented Invoke-RestMethod command against its real HTTP server. After: the real MCP SDK client makes authenticated HTTP calls to the same implementation used by the app. The shell row compares the same no-op command with and without -NoProfile. Both build implementations use an immediate stub compiler, so that row measures dispatch overhead only.\n\nThese are local execution and transport measurements, not total agent response times. They exclude model inference, remote model latency, CLI startup, provider sandbox setup, and real compilation. The file-operation baseline is a shell-based workflow; it does not measure Codex apply_patch or OpenCode's native file tools, nor establish that every prior agent call took this long. The measured shell command still takes roughly hundreds of milliseconds. Compilers and other long-running commands retain their actual execution cost.\n\n## Reliability and integration checks\n\nThe regression tests exercise exact-match and SHA-256 preconditions, concurrent edits, read-back verification, UTF-8 BOM/CRLF preservation, private/traversal/link rejection, binary/invalid UTF-8/oversized files, authentication, origin rejection, connection expiry, and waiting for active tools at shutdown. A hidden desktop test checks both provider paths, snapshots, and that the capability is not saved in chat.\n\nOpt-in real-provider checks (\`REFRACT_REAL_TOOLS_TEST=1\`, optionally \`REFRACT_REAL_TOOLS_PROVIDER=opencode\`) each perform two turns: read, edit, stub build, then resume the same session through a fresh capability. Both installed providers passed these checks during development. They verify wiring and session reuse; their model-dependent wall times are not benchmark samples.\n`);
    const max = Math.max(...rows.map(row=>row.before.medianMs));
    const bars = rows.map(row=>`<section><div class="label">${row.operation}</div><div class="bar before" style="width:${row.before.medianMs/max*100}%"><span>${row.before.medianMs.toFixed(1)} ms</span></div><div class="bar after" style="width:${Math.max(1,row.after.medianMs/max*100)}%"><span>${row.after.medianMs.toFixed(1)} ms</span></div></section>`).join('');
    await fs.writeFile(path.resolve('docs/benchmarks/assistant-tools.html'),`<!doctype html><html><head><meta charset="utf-8"><style>body{font-family:var(--font-sans,system-ui);color:var(--foreground);margin:0}section{margin:20px 80px 24px 0}.label{font-size:14px;margin-bottom:7px}.bar{height:12px;border-radius:3px;position:relative;margin:6px 0}.bar span{position:absolute;left:calc(100% + 8px);font-size:12px;white-space:nowrap;line-height:12px}.before{background:var(--chart-2,#9c8df1)}.after{background:var(--chart-1,#38bfa7)}p{font-size:12px;color:var(--muted-foreground);line-height:1.5}.legend{font-size:13px}.key{display:inline-block;width:10px;height:10px;margin-right:5px}.before-key{background:var(--chart-2,#9c8df1)}.after-key{background:var(--chart-1,#38bfa7)}</style></head><body><div class="legend"><span class="key before-key"></span>Before &nbsp; <span class="key after-key"></span>After · median local latency · lower is faster</div>${bars}<p>${samples} samples per path; ${warmups} warmups. File rows: fresh PowerShell versus direct MCP tools. Build row: dispatch to a stub compiler. Shell row: profile versus no profile. Excludes model time, provider startup, sandbox setup, and real compilation.</p></body></html>`);
  } finally {
    await client.close(); await bridge.close(); await legacy.close();
    assert.equal(path.dirname(await fs.realpath(root)),parent); assert.ok(path.basename(root).startsWith('refract-tool-bench-'));
    await fs.rm(root,{recursive:true,force:true});
  }
}
void main().catch(error=>{console.error(error);process.exitCode=1;});
