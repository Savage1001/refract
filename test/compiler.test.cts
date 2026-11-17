const {test} = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const os = require('node:os');
const path = require('node:path');
const compiler = require('../src/compiler.cjs');
const trust = require('../src/project-trust.cjs');
// These tests exercise compiler mechanics using explicitly trusted test fixtures.
const compile = async (directory:string,options?:Parameters<typeof compiler.compile>[1]) => {await trust.grantTrust(directory);return compiler.compile(directory,options);};
const runCompilerProcess = compiler.runCompilerProcess;
test('cancelling a compiler process stops it and reports cancellation', {timeout:15000}, async t=>{
 const directory=await fs.mkdtemp(path.join(os.tmpdir(),'refract-cancel-'));
 t.after(async()=>{await trust.revokeTrust(directory);await fs.rm(directory,{recursive:true,force:true});});
 await trust.grantTrust(directory);
 const controller=new AbortController();
 const running=runCompilerProcess(process.execPath,['-e','setInterval(() => {}, 1000)'],directory,process.env,controller.signal);
 controller.abort();const result=await running;
 assert.equal(result.code,null);assert.match(result.output,/Compilation cancelled/);
 const already=new AbortController();already.abort();
 assert.deepEqual(await runCompilerProcess(process.execPath,['-e','throw new Error("should not run")'],directory,process.env,already.signal),{code:null,output:'Compilation cancelled.'});
});
test('missing compiler is reported without returning a stale PDF', async t => {
 const root = await fs.mkdtemp(path.join(os.tmpdir(), 'refract-compile-'));
 t.after(() => fs.rm(root, {recursive: true, force: true}));
 await fs.writeFile(path.join(root,'main.tex'),'test');
 await fs.mkdir(path.join(root, '.refract-build'));
 await fs.writeFile(path.join(root, '.refract-build', 'main.pdf'), 'stale');
 await fs.writeFile(path.join(root, '.refract-build', 'main.synctex.gz'), 'stale');
 await assert.rejects(compile(root, {executable: 'refract-missing-compiler'}), /local LaTeX installation/);
 await assert.rejects(fs.access(path.join(root, '.refract-build', 'main.pdf')));
 await assert.rejects(fs.access(path.join(root, '.refract-build', 'main.synctex.gz')));
});

test('compilation refuses linked build directories and preserves external files',async t=>{
 const root:string=await fs.mkdtemp(path.join(os.tmpdir(),'refract-build-links-'));
 t.after(async()=>{const resolved=await fs.realpath(root),parent=await fs.realpath(os.tmpdir());if(path.dirname(resolved).toLowerCase()!==parent.toLowerCase() || !path.basename(resolved).startsWith('refract-build-links-'))throw new Error('Unsafe cleanup path');await fs.rm(resolved,{recursive:true,force:true});});
 const project=path.join(root,'project'),outside=path.join(root,'outside'),build=path.join(project,'.refract-build');await fs.mkdir(project);await fs.mkdir(outside);await fs.writeFile(path.join(project,'main.tex'),'test');await fs.writeFile(path.join(outside,'main.pdf'),'preserve');
 try {await fs.symlink(outside,build,process.platform==='win32'?'junction':'dir');}catch(error){if(['EPERM','EACCES'].includes(error.code))return t.skip('Creating directory links is not permitted');throw error;}
 try {await assert.rejects(compile(project),/Invalid compilation directory/);assert.equal(await fs.readFile(path.join(outside,'main.pdf'),'utf8'),'preserve');}finally{await fs.unlink(build);}
 await fs.mkdir(build);await fs.mkdir(path.join(build,'nested'));await fs.link(path.join(outside,'main.pdf'),path.join(build,'nested','linked.pdf'));
 await assert.rejects(compile(project),/Only unlinked regular files/);assert.equal(await fs.readFile(path.join(outside,'main.pdf'),'utf8'),'preserve');
});

test('compilation rejects invalid and linked roots without changing previous outputs',async t=>{
 const root:string=await fs.mkdtemp(path.join(os.tmpdir(),'refract-root-links-'));
 t.after(async()=>{const resolved=await fs.realpath(root),parent=await fs.realpath(os.tmpdir());if(path.dirname(resolved).toLowerCase()!==parent.toLowerCase() || !path.basename(resolved).startsWith('refract-root-links-'))throw new Error('Unsafe cleanup path');await fs.rm(resolved,{recursive:true,force:true});});
 const project=path.join(root,'project'),outside=path.join(root,'outside'),build=path.join(project,'.refract-build');await fs.mkdir(project);await fs.mkdir(outside);await fs.mkdir(build);
 await fs.writeFile(path.join(build,'main.pdf'),'preserve');await fs.writeFile(path.join(outside,'main.tex'),'external');
 await assert.rejects(compile(project,{rootFile:'../outside/main.tex'}),/Invalid LaTeX root file/);
 await assert.rejects(compile(project,{rootFile:'.private.tex'}),/Invalid LaTeX root file/);
 try {await fs.symlink(outside,path.join(project,'linked'),process.platform==='win32'?'junction':'dir');}catch(error){if(['EPERM','EACCES'].includes(error.code))return t.skip('Creating directory links is not permitted');throw error;}
 try {await assert.rejects(compile(project,{rootFile:'linked/main.tex'}),/Invalid LaTeX root file/);assert.equal(await fs.readFile(path.join(build,'main.pdf'),'utf8'),'preserve');}finally{await fs.unlink(path.join(project,'linked'));}
});

test('configured TeX compiles the default project', async t => {
 const exe = path.join(process.env.REFRACT_TEX_BIN || 'missing-tex', 'pdflatex.exe');
 try { await fs.access(exe); } catch { return t.skip('External TeX is not configured'); }
 const { ProjectStore } = require('../src/project-store.cjs');
 const root = await fs.mkdtemp(path.join(os.tmpdir(), 'refract-compile-real-'));
 t.after(() => fs.rm(root, {recursive:true,force:true}));
 const store = new ProjectStore(root), project = await store.create('Test');
 const result = await compile(store.projectDir(project.id));
 assert.equal(result.ok, true, result.log);
 assert.equal(Buffer.from(result.pdf, 'base64').subarray(0, 5).toString(), '%PDF-');
 const sync=require('node:zlib').gunzipSync(await fs.readFile(path.join(store.projectDir(project.id),'.refract-build','main.synctex.gz'))).toString('utf8');
 assert.match(sync,/SyncTeX Version:/);assert.match(sync,/Input:\d+:.*main\.tex/);
});

test('compilation resolves BibTeX references with configured TeX', async t => {
 const exe = path.join(process.env.REFRACT_TEX_BIN || 'missing-tex', 'pdflatex.exe');
 try { await fs.access(exe); } catch { return t.skip('External TeX is not configured'); }
 const root = await fs.mkdtemp(path.join(os.tmpdir(),'refract-bib-')); t.after(()=>fs.rm(root,{recursive:true,force:true}));
 await fs.writeFile(path.join(root,'main.tex'),'\\documentclass{article}\n\\begin{document}\nSee \\cite{example}.\n\\bibliographystyle{plain}\n\\bibliography{references}\n\\end{document}\n');
 await fs.writeFile(path.join(root,'references.bib'),'@article{example,author={Ada Lovelace},title={A Paper},journal={Journal},year={1843}}');
 const result = await compile(root); assert.equal(result.ok,true,result.log);
 const bbl = await fs.readFile(path.join(root,'.refract-build','main.bbl'),'utf8'); assert.match(bbl,/Lovelace/);
 assert.doesNotMatch(result.log.split('This is pdfTeX').at(-1),/Citation .* undefined/);
});

test('compilation resolves biblatex references with configured biber', async t => {
 const exe = path.join(process.env.REFRACT_TEX_BIN || 'missing-tex', 'pdflatex.exe');
 try { await fs.access(exe); } catch { return t.skip('External TeX is not configured'); }
 const root = await fs.mkdtemp(path.join(os.tmpdir(),'refract-biber-')); t.after(()=>fs.rm(root,{recursive:true,force:true}));
 await fs.writeFile(path.join(root,'main.tex'),'\\documentclass{article}\n\\usepackage[backend=biber]{biblatex}\n\\addbibresource{references.bib}\n\\begin{document}\nSee \\cite{example}.\n\\printbibliography\n\\end{document}\n');
 await fs.writeFile(path.join(root,'references.bib'),'@article{example,author={Ada Lovelace},title={A Paper},journal={Journal},year={1843}}');
 const result = await compile(root); assert.equal(result.ok,true,result.log);
 const bbl = await fs.readFile(path.join(root,'.refract-build','main.bbl'),'utf8'); assert.match(bbl,/Lovelace/);
});

test('configured engines resolve nested bibliographies in a multi-file paper', {timeout:300000}, async t => {
 const bin=(process.env.REFRACT_TEX_BIN || 'missing-tex');
 try {await fs.access(path.join(bin,'pdflatex.exe'));} catch {return t.skip('External TeX is not configured');}
 const root=await fs.mkdtemp(path.join(os.tmpdir(),'refract-paper-bibliography-'));
 t.after(()=>fs.rm(root,{recursive:true,force:true}));
 await fs.mkdir(path.join(root,'sections'));await fs.mkdir(path.join(root,'references'));
 await fs.writeFile(path.join(root,'sections','body.tex'),'A nested section cites \\cite{example}.\n');
 await fs.writeFile(path.join(root,'references','sources.bib'),'@article{example,author={Ada Lovelace},title={A Paper},journal={Journal},year={1843}}\n');
 for(const engine of ['pdflatex.exe','xelatex.exe','lualatex.exe']) {
  for(const backend of ['bibtex','biber']) {
   const bibliography=backend==='bibtex' ? '\\bibliographystyle{plain}\n\\bibliography{references/sources}' : '\\printbibliography';
   const preamble=backend==='biber' ? '\\usepackage[backend=biber]{biblatex}\n\\addbibresource{references/sources.bib}\n' : '';
   await fs.writeFile(path.join(root,'main.tex'),'\\documentclass{article}\n\\usepackage{amsmath,graphicx}\n'+preamble+'\\begin{document}\n\\input{sections/body}\n'+bibliography+'\n\\end{document}\n');
   const result=await compile(root,{executable:path.join(bin,engine)});
   assert.equal(result.ok,true,`${engine}/${backend}: ${result.log}`);
   assert.match(await fs.readFile(path.join(root,'.refract-build','main.bbl'),'utf8'),/Lovelace/);
   assert.doesNotMatch(result.log.split(/This is (?:pdfTeX|XeTeX|LuaHBTeX)/).at(-1),/Citation .* undefined/);
  }
 }
});

test('configured engines compile a multi-file paper with an image and standard packages', {timeout:90000}, async t=>{
 const bin=(process.env.REFRACT_TEX_BIN || 'missing-tex');
 try {await fs.access(path.join(bin,'pdflatex.exe'));} catch {return t.skip('External TeX is not configured');}
 const base=await fs.mkdtemp(path.join(os.tmpdir(),'refract-multifile-'));t.after(()=>fs.rm(base,{recursive:true,force:true}));
 const root=path.join(base,'Paper & Notes'); await fs.mkdir(root);
 await fs.mkdir(path.join(root,'sections')); await fs.mkdir(path.join(root,'figures'));
 await fs.writeFile(path.join(root,'sections','body.tex'),'A nested section with $\\int_0^1 x^2\\,dx$.\n');
 await fs.writeFile(path.join(root,'figures','pixel.png'),Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR4nGP4z8AAAAMBAQDJ/pLvAAAAAElFTkSuQmCC','base64'));
 for(const engine of ['pdflatex.exe','xelatex.exe','lualatex.exe']) {
  const executable=path.join(bin,engine);await fs.access(executable);
  const fonts=engine==='pdflatex.exe' ? '\\usepackage[T1]{fontenc}\n\\usepackage{lmodern}\n' : '\\usepackage{fontspec}\n\\setmainfont{Latin Modern Roman}\n';
  await fs.writeFile(path.join(root,'main.tex'),'\\documentclass{article}\n\\usepackage{amsmath,graphicx}\n'+fonts+'\\begin{document}\n\\input{sections/body}\n\\includegraphics[width=1cm]{figures/pixel.png}\n\\end{document}\n');
  const result=await compile(root,{executable});assert.equal(result.ok,true,`${engine}: ${result.log}`);
  assert.equal(Buffer.from(result.pdf,'base64').subarray(0,5).toString(),'%PDF-');
  const sync=require('node:zlib').gunzipSync(await fs.readFile(path.join(root,'.refract-build','main.synctex.gz'))).toString('utf8');assert.match(sync,/sections[\\/]body\.tex/);
 }
 const synctex=path.join(bin,'synctex.exe');await fs.access(synctex);
 const query=require('node:child_process').spawnSync(synctex,['view','-i','1:1:sections/body.tex','-o',path.join(root,'.refract-build','main.pdf')],{cwd:root,windowsHide:true,encoding:'utf8'});
 assert.equal(query.status,0,query.stderr);assert.match(query.stdout,/^Page:1\s*$/m);
 const navigation=require('../src/synctex.cjs'),position=await navigation.forward(synctex,root,'sections/body.tex',1);
 assert.equal(position.page,1);const source=await navigation.reverse(synctex,root,position.page,position.x,position.y);
 assert.equal(source.file,'sections/body.tex');assert.equal(source.line,1);
 await assert.rejects(navigation.forward(synctex,root,'../outside.tex',1),/Invalid source position/);
 await assert.rejects(navigation.reverse(synctex,root,1,-1,0),/Invalid PDF position/);
});

test('missing packages fail without leaving an old PDF',async t=>{
 const bin=path.join(process.env.REFRACT_TEX_BIN || 'missing-tex', 'pdflatex.exe');try {await fs.access(bin);} catch {return t.skip('External TeX is not configured');}
 const root=await fs.mkdtemp(path.join(os.tmpdir(),'refract-missing-package-'));t.after(()=>fs.rm(root,{recursive:true,force:true}));
 await fs.mkdir(path.join(root,'.refract-build'));await fs.writeFile(path.join(root,'.refract-build','main.pdf'),'old');
 await fs.writeFile(path.join(root,'.refract-build','main.synctex.gz'),'stale');
 await fs.writeFile(path.join(root,'main.tex'),'\\documentclass{article}\n\\usepackage{refract-package-that-does-not-exist}\n\\begin{document}Text\\end{document}');
 const result=await compile(root);assert.equal(result.ok,false);assert.match(result.log,/refract-package-that-does-not-exist/);
 await assert.rejects(fs.access(path.join(root,'.refract-build','main.pdf')));
 await assert.rejects(fs.access(path.join(root,'.refract-build','main.synctex.gz')));
});

test('SyncTeX refuses to open source files outside the project', {timeout:30000}, async t=>{
 const bin=(process.env.REFRACT_TEX_BIN || 'missing-tex'),exe=path.join(bin,'pdflatex.exe'),sync=path.join(bin,'synctex.exe');
 try {await fs.access(exe);await fs.access(sync);}catch{return t.skip('Configured TeX and SyncTeX are required');}
 const root:string=await fs.mkdtemp(path.join(os.tmpdir(),'refract-synctex-outside-'));
 t.after(async()=>{const resolved=await fs.realpath(root),parent=await fs.realpath(os.tmpdir());if(path.dirname(resolved).toLowerCase()!==parent.toLowerCase() || !path.basename(resolved).startsWith('refract-synctex-outside-'))throw new Error('Unsafe cleanup path');await fs.rm(resolved,{recursive:true,force:true});});
 const project=path.join(root,'project');await fs.mkdir(project);await fs.writeFile(path.join(root,'outside.tex'),'External source text.\n');
 await fs.writeFile(path.join(project,'main.tex'),'\\documentclass{article}\n\\begin{document}\n\\input{../outside}\n\\end{document}\n');
 const compiled=await compile(project,{executable:exe});assert.equal(compiled.ok,true,compiled.log);
 await assert.rejects(require('../src/synctex.cjs').reverse(sync,project,1,160,130),/outside the project/);
});

test('SyncTeX navigates Unicode source filenames with all configured engines', {timeout:90000}, async t=>{
 const bin=(process.env.REFRACT_TEX_BIN || 'missing-tex'),sync=path.join(bin,'synctex.exe');
 try {await fs.access(sync);} catch {return t.skip('Configured SyncTeX is required');}
 const base:string=await fs.mkdtemp(path.join(os.tmpdir(),'refract-synctex-unicode-'));
 t.after(async()=>{const resolved=await fs.realpath(base),parent=await fs.realpath(os.tmpdir());if(path.dirname(resolved).toLowerCase()!==parent.toLowerCase() || !path.basename(resolved).startsWith('refract-synctex-unicode-'))throw new Error('Unsafe cleanup path');await fs.rm(resolved,{recursive:true,force:true});});
 const root=path.join(base,'Café & Notes');await fs.mkdir(root);await fs.mkdir(path.join(root,'sections'));
 await fs.writeFile(path.join(root,'main.tex'),'\\documentclass{article}\n\\begin{document}\n\\input{sections/résumé}\n\\end{document}\n');
 await fs.writeFile(path.join(root,'sections','résumé.tex'),'A synchronized line.\n');
 const navigation=require('../src/synctex.cjs');
 for(const engine of ['pdflatex.exe','xelatex.exe','lualatex.exe']) {
  t.diagnostic(`Testing SyncTeX with ${engine}`);
  const result=await compile(root,{executable:path.join(bin,engine)});assert.equal(result.ok,true,`${engine}: ${result.log}`);
  const position=await navigation.forward(sync,root,'sections/résumé.tex',1);assert.equal(position.page,1);
  const source=await navigation.reverse(sync,root,position.page,position.x,position.y);assert.equal(source.file,'sections/résumé.tex');assert.equal(source.line,1);
 }
});

test('configured XeLaTeX resolves fonts from the external distribution', {timeout:60000}, async t=>{
 const exe=path.join(process.env.REFRACT_TEX_BIN || 'missing-tex','xelatex.exe');
 try {await fs.access(exe);} catch {return t.skip('External TeX is not configured');}
 const root=await fs.mkdtemp(path.join(os.tmpdir(),'refract-external-font-'));t.after(()=>fs.rm(root,{recursive:true,force:true}));
 await fs.writeFile(path.join(root,'main.tex'),'\\documentclass{article}\n\\usepackage{fontspec}\n\\setmainfont{Latin Modern Roman}\n\\begin{document}External font.\\end{document}\n');
 const result=await compile(root,{executable:exe});assert.equal(result.ok,true,result.log);
});

 test('draft mode produces a PDF with graphicx placeholders across engines', {timeout:90000}, async t=>{
 const root=await fs.mkdtemp(path.join(os.tmpdir(),'refract-draft-'));
 t.after(()=>fs.rm(root,{recursive:true,force:true}));
 const bin=(process.env.REFRACT_TEX_BIN || 'missing-tex');
 try {await fs.access(path.join(bin,'pdflatex.exe'));} catch {return t.skip('External TeX is not configured');}
 await fs.writeFile(path.join(root,'report%2026.tex'),'\\documentclass{article}\n\\usepackage{graphics}\n\\usepackage{graphicx}\n\\begin{document}\\includegraphics[width=2cm,height=2cm]{missing-image.png}\\end{document}');
 await fs.copyFile(path.resolve(__dirname,'../../resources/logo.png'),path.join(root,'missing-image.png'));
 await fs.copyFile(path.join(root,'report%2026.tex'),path.join(root,'draft.tex'));
 for(const engine of ['pdflatex','xelatex','lualatex']) {
 for(const rootFile of ['report%2026.tex','draft.tex']) {
 const result=await compile(root,{executable:path.join(bin,engine+'.exe'),draftMode:true,rootFile});
 assert.equal(result.ok,true,result.log);
 assert.doesNotMatch(Buffer.from(result.pdf,'base64').toString('latin1'),/\/Subtype\s*\/Image\b/,'Draft PDF must not embed the image');
 const {getDocument}=await import('pdfjs-dist/legacy/build/pdf.mjs');
 const pdfTask=getDocument({data:new Uint8Array(Buffer.from(result.pdf,'base64'))});
 try {const pdf=await pdfTask.promise;const page=await pdf.getPage(1);const text=await page.getTextContent();
 assert.match(text.items.map(item=>'str' in item ? item.str : '').join(''),/missing-image\.png/,'Draft PDF must label the placeholder');
 } finally {await pdfTask.destroy();}
 assert.equal(result.arguments.at(-1),'./.refract-build/refract-internal/draft.tex');
 const replay=await require('../src/compiler.cjs').runCompilerProcess(path.join(bin,engine+'.exe'),result.arguments,root,process.env);
 assert.equal(replay.code,0,replay.output);
 const normal=await compile(root,{executable:path.join(bin,engine+'.exe'),rootFile:'draft.tex'});
 assert.equal(normal.ok,true,normal.log);
 assert.match(Buffer.from(normal.pdf,'base64').toString('latin1'),/\/Subtype\s*\/Image\b/,'Normal PDF must embed the image');
 }
 }
 });
