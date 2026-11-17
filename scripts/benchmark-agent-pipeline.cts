import fs = require('node:fs/promises');
import path = require('node:path');
import os = require('node:os');
import crypto = require('node:crypto');
import assert = require('node:assert/strict');
import runtime = require('../src/codex-runtime.cjs');
import builds = require('../src/build-cache.cjs');
import assistantTools = require('../src/assistant-tools.cjs');
import clientSdk = require('@modelcontextprotocol/sdk/client/index.js');
import transportSdk = require('@modelcontextprotocol/sdk/client/streamableHttp.js');
const {ProjectStore}=require('../src/project-store.cjs'),{Versions}=require('../src/versions.cjs'),{compile}=require('../src/compiler.cjs'),Zip=require('adm-zip');
const n=Number(process.env.REFRACT_BENCH_SAMPLES || 30),warmups=3;
const summarize=(values:number[])=>{const a=[...values].sort((x,y)=>x-y);return {medianMs:(a[Math.floor((a.length-1)/2)]+a[Math.floor(a.length/2)])/2,p95Ms:a[Math.ceil(a.length*.95)-1],samplesMs:values};};
async function main() {
  assert.ok(Number.isInteger(n)&&n>=10&&n<=1000);
  const parent=await fs.realpath(os.tmpdir()),root=await fs.mkdtemp(path.join(parent,'refract-pipeline-bench-'));
  const store=new ProjectStore(root),project=await store.create('Benchmark'),cwd=store.projectDir(project.id),versions=new Versions(store);
  const rows:Array<{operation:string;before:ReturnType<typeof summarize>;after:ReturnType<typeof summarize>}>=[];
  let warm:InstanceType<typeof runtime.CodexRuntime>|undefined,bridge:Awaited<ReturnType<typeof assistantTools.startAssistantTools>>|undefined;
  const client=new clientSdk.Client({name:'refract-pipeline-benchmark',version:'1.0.0'});
  const measure=async(name:string,before:()=>Promise<unknown>,after:()=>Promise<unknown>,prepare?:()=>Promise<unknown>,verify?:(value:unknown)=>Promise<void>)=>{
    const samples={before:[] as number[],after:[] as number[]};
    for(let i=-warmups;i<n;i++)for(const side of (i%2 ? ['after','before'] : ['before','after']) as Array<'before'|'after'>){
      await prepare?.();const started=performance.now();const value=await(side==='before'?before():after());const elapsed=performance.now()-started;await verify?.(value);if(i>=0)samples[side].push(elapsed);
    }
    const row={operation:name,before:summarize(samples.before),after:summarize(samples.after)};rows.push(row);console.log(`${name}: ${row.before.medianMs.toFixed(2)} -> ${row.after.medianMs.toFixed(2)} ms`);
  };
  try {
    // Frozen duplicate-snapshot scan from commit 5158953: sequential reads,
    // full content hashing, and ZIP entry construction before deduplication.
    for(let i=0;i<160;i++)await store.write(project.id,`assets/file-${i}.bin`,crypto.randomBytes(65536));
    const initial=await versions.snapshot(project.id);
    const legacyScan=async()=>{
      const names=(await store.files(project.id)).sort(),zip=new Zip(),hash=crypto.createHash('sha256'),directories:string[]=[];
      const visit=(nodes:Array<{path:string;children?:any[]}>)=>{for(const node of nodes)if(node.children){directories.push(node.path);visit(node.children);}};
      visit(await store.tree(project.id));directories.sort();const metadata=await store.metadata(project.id);
      hash.update(JSON.stringify({directories,rootFile:metadata.rootFile || 'main.tex'}));
      for(const name of names){const data=await fs.readFile(await store.existingFile(project.id,name));hash.update(Buffer.from(name.length+':'+name+':'+data.length+':'));hash.update(data);zip.addFile(name,data);}
      await versions.list(project.id);return hash.digest('hex');
    };
    const legacyDigest=await legacyScan(),statsBefore=versions.sourceStatistics();
    await measure('Unchanged snapshot scan (161 files, 10 MiB)',async()=>assert.equal(await legacyScan(),legacyDigest),async()=>assert.equal((await versions.snapshot(project.id)).id,initial.id));
    const statsAfter=versions.sourceStatistics();
    const snapshotSourceBytesRead=statsAfter.bytesRead-statsBefore.bytesRead;
    const changedProject=await store.create('Changed snapshots');
    const assets=new Map<string,Buffer>();
    for(let i=0;i<8;i++){const name=`assets/${i}.bin`,data=crypto.randomBytes(512*1024);assets.set(name,data);await store.write(changedProject.id,name,data);}
    await versions.snapshot(changedProject.id);let edit=0;
    const legacySnapshot=async()=>{
      const id=changedProject.id,names=(await store.files(id)).sort(),zip=new Zip(),hash=crypto.createHash('sha256'),directories:string[]=[];
      const visit=(nodes:Array<{path:string;children?:any[]}>)=>{for(const node of nodes)if(node.children){directories.push(node.path);visit(node.children);}};
      visit(await store.tree(id));directories.sort();const metadata=await store.metadata(id),rootFile=metadata.rootFile || 'main.tex';hash.update(JSON.stringify({directories,rootFile}));
      for(const name of names){const data=await fs.readFile(await store.existingFile(id,name));hash.update(Buffer.from(name.length+':'+name+':'+data.length+':'));hash.update(data);zip.addFile(name,data);}
      const digest=hash.digest('hex'),latest=(await versions.list(id))[0],version={id:crypto.randomUUID(),created:new Date().toISOString(),sequence:latest.sequence+1,digest,files:names,directories,rootFile};
      const folder=versions.directory(id);await fs.writeFile(path.join(folder,version.id+'.zip'),zip.toBuffer(),{flag:'wx'});
      const temp=path.join(folder,version.id+'.json.tmp');await fs.writeFile(temp,JSON.stringify(version),{flag:'wx'});await fs.rename(temp,path.join(folder,version.id+'.json'));
      return version;
    };
    await measure('One-file snapshot edit (4 MiB unchanged assets)',legacySnapshot,()=>versions.snapshot(changedProject.id),()=>store.write(changedProject.id,'main.tex',`changed ${++edit}`),async value=>{
      const archive=new Zip(path.join(versions.directory(changedProject.id),(value as {id:string}).id+'.zip'));
      assert.equal(archive.getEntry('main.tex').getData().toString(),`changed ${edit}`);
      for(const [name,data] of assets)assert.deepEqual(archive.getEntry(name).getData(),data);
    });
    // Provider process work only: no turn/model request is started.
    warm=new runtime.CodexRuntime(cwd);await warm.ready();
    await measure('Codex initialize + catalog (cold vs warm)',async()=>{
      const cold=new runtime.CodexRuntime(cwd);
      try{await cold.ready();const reply=await cold.request('model/list',{limit:1});assert.ok(Array.isArray(reply.data));}finally{await cold.close();}
    },async()=>{const reply=await warm!.request('model/list',{limit:1});assert.ok(Array.isArray(reply.data));});
    await warm.close();warm=undefined;
    bridge=await assistantTools.startAssistantTools(store,project.id,async()=>({ok:true}),async()=>({available:true}));
    await client.connect(new transportSdk.StreamableHTTPClientTransport(new URL(bridge.connection.url),{requestInit:{headers:{Authorization:`Bearer ${bridge.connection.token}`}}}));
    const call=async(name:string,args:Record<string,unknown>)=>{const reply=await client.callTool({name,arguments:args});assert.ok(!reply.isError,JSON.stringify(reply));return JSON.parse((reply.content as Array<{text:string}>)[0].text);};
    const paths:string[]=[];
    for(let i=0;i<8;i++){const file=`sections/${i}.tex`;paths.push(file);await store.write(project.id,file,'short context\n'.repeat(50));}
    await measure('Read 8 files (8 RPCs vs 1 batch)',async()=>{for(const file of paths)assert.ok((await call('project_read',{path:file})).sha256);},async()=>{const batch=await call('project_read_many',{paths});assert.equal(batch.length,8);assert.ok(batch.every(file=>file.sha256));});
    await store.write(project.id,'context.tex','context '.repeat(8192));
    let fullCharacters=0,boundedCharacters=0;
    await measure('Read context (64 KiB vs 16k excerpt)',async()=>{fullCharacters=(await call('project_read',{path:'context.tex',limit:65536})).content.length;},async()=>{boundedCharacters=(await call('project_read',{path:'context.tex'})).content.length;});
    await client.close();await bridge.close();bridge=undefined;
    // A small separate paper prevents the 10 MiB snapshot fixture from biasing builds.
    const paper=await store.create('Build benchmark'),paperDir=store.projectDir(paper.id),executable=path.resolve('vendor/TinyTeX/bin/windows/pdflatex.exe');
    const cache=new builds.BuildCache(store,compile),options={executable,rootFile:'main.tex'};
    await measure('Repeated pdflatex build (force vs validated reuse)',async()=>{const result=await cache.run(paper.id,paperDir,options,true);assert.equal(result.ok,true,result.log);},async()=>{const result=await cache.run(paper.id,paperDir,options);assert.equal(result.cached,true,result.log);},()=>cache.run(paper.id,paperDir,options));
    const report={date:new Date().toISOString(),platform:process.platform,node:process.version,cpu:os.cpus()[0].model,samples:n,warmups,snapshotSourceBytesReadAfter:snapshotSourceBytesRead,fullContextCharacters:fullCharacters,boundedContextCharacters:boundedCharacters,rows};
    await fs.mkdir('docs/benchmarks',{recursive:true});await fs.writeFile('docs/benchmarks/agent-pipeline.json',JSON.stringify(report,null,2)+'\n');
    const table=rows.map(row=>`| ${row.operation} | ${row.before.medianMs.toFixed(2)} | ${row.after.medianMs.toFixed(2)} | ${row.before.p95Ms.toFixed(2)} | ${row.after.p95Ms.toFixed(2)} |`).join('\n');
    await fs.writeFile('docs/benchmarks/agent-pipeline.md',`# Agent pipeline benchmark\n\nMeasured ${report.date}, ${report.platform}, Node ${report.node}, ${report.cpu}. ${n} samples per path and operation after ${warmups} warmups; alternating order. All outputs validated.\n\n| Operation | Before median ms | After median ms | Before p95 ms | After p95 ms |\n| --- | ---: | ---: | ---: | ---: |\n${table}\n\nUnchanged snapshot reads after warmup: **${snapshotSourceBytesRead} source bytes** across the measured and warmup calls. The original scan reads the entire 10 MiB fixture each time. Snapshot history reads and filesystem validation still occur.\n\nBatching reduces requests for 8 files from 8 to 1. Bounded reads reduce the fixture's returned context from ${fullCharacters} to ${boundedCharacters} characters, preserving its full-file edit hash and an explicit continuation offset.\n\nRun \`npm run benchmark:pipeline\` on Windows with bundled TinyTeX and authenticated Codex installed. Raw samples are in [agent-pipeline.json](agent-pipeline.json).\n\nThe snapshot baseline reproduces the unchanged-snapshot scan in commit 5158953; after uses the real Versions.snapshot implementation with its cache. The one-file edit row compares the original full snapshot writer with incremental source/compression reuse; every resulting archive is independently checked against the changed text and all unchanged binary assets outside timing. Both paths include archive and metadata writes. The Codex row compares fresh app-server initialization plus one catalog request against a catalog request on an initialized process; it includes closing the cold process and measures process lifecycle overhead, not an old codex exec turn or model inference. File rows use a real MCP client against the app's server. The build row compares forced real two-pass pdfLaTeX compilation against validation and reuse of the same successful build; cache seeding occurs outside timing. These measurements do not predict model thinking time. OpenCode already reuses its shared background service, so no second OpenCode daemon was added.\n`);
    const max=Math.max(...rows.map(row=>row.before.medianMs));
    const bars=rows.map(row=>`<section><p>${row.operation}</p><div class="before" style="width:${row.before.medianMs/max*100}%"><span>${row.before.medianMs.toFixed(1)} ms</span></div><div class="after" style="width:${Math.max(1,row.after.medianMs/max*100)}%"><span>${row.after.medianMs.toFixed(1)} ms</span></div></section>`).join('');
    await fs.writeFile('docs/benchmarks/agent-pipeline.html',`<!doctype html><html><head><meta charset="utf-8"><style>body{font-family:var(--font-sans,system-ui);color:var(--foreground);margin:0}section{margin:20px 90px 24px 0}p{font-size:13px;margin:0 0 7px}section div{height:12px;border-radius:3px;position:relative;margin:6px 0}span{position:absolute;left:calc(100% + 8px);font-size:12px;white-space:nowrap;line-height:12px}.before{background:var(--chart-2,#9c8df1)}.after{background:var(--chart-1,#38bfa7)}small{color:var(--muted-foreground);line-height:1.5}</style></head><body><p>Before (purple) / After (green) — median local latency; lower is faster</p>${bars}<small>${n} paired samples per operation. Codex row includes cold process lifecycle versus warm requests; no model inference. Build row uses real pdfLaTeX. Details and raw samples are included in the PR.</small></body></html>`);
  } finally {
    await client.close().catch(()=>{});await bridge?.close();await warm?.close();
    assert.equal(path.dirname(await fs.realpath(root)),parent);assert.ok(path.basename(root).startsWith('refract-pipeline-bench-'));
    await fs.rm(root,{recursive:true,force:true,maxRetries:5,retryDelay:200});
  }
}
void main().catch(error=>{console.error(error);process.exitCode=1;});
