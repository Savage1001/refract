const { texBin } = require('./tex-toolchain.cjs');
const { spawn } = require('node:child_process');
const fs = require('node:fs/promises');
const path = require('node:path');
const { cancelProcess } = require('./chat.cjs');

async function runCompilerProcess(command:string, commandArgs:string[], cwd:string, env:NodeJS.ProcessEnv, signal?:AbortSignal): Promise<{code:number|null;output:string}> {
  return new Promise((resolve,reject)=>{
    if(signal?.aborted) {resolve({code:null,output:'Compilation cancelled.'});return;}
    const child=spawn(command,commandArgs,{cwd,env,windowsHide:true,shell:false});
    let output='', cancelled=false;
    const collect=(chunk:Buffer)=>{output=(output+chunk.toString()).slice(-100000);};
    const cancel=()=>{if(cancelled)return;cancelled=true;void cancelProcess(child);};
    signal?.addEventListener('abort',cancel,{once:true});if(signal?.aborted)cancel();
    child.stdout.on('data',collect);child.stderr.on('data',collect);
    child.on('error',error=>{signal?.removeEventListener('abort',cancel);reject(new Error(error.code==='ENOENT' ? 'A local LaTeX installation with pdflatex is required.' : error.message));});
    child.on('close',code=>{signal?.removeEventListener('abort',cancel);resolve({code:cancelled ? null : code,output:cancelled ? output+'\nCompilation cancelled.' : output});});
  });
}
async function compile(projectDir, options: { executable?: string; rootFile?: string; draftMode?: boolean; signal?:AbortSignal } = {}) {
  if (options.signal?.aborted) return {ok:false,log:'Compilation cancelled.',exitCode:null};
  const rootInfo=await fs.lstat(projectDir);
  if(rootInfo.isSymbolicLink() || !rootInfo.isDirectory())throw new Error('Invalid project directory');
  const rootFile = options.rootFile || 'main.tex';
  if (typeof rootFile!=='string' || path.isAbsolute(rootFile) || rootFile.includes(':') || rootFile.split(/[\\/]/).some(part => !part || part === '.' || part === '..' || part.startsWith('.'))) throw new Error('Invalid LaTeX root file');
  const parts=rootFile.split(/[\\/]/);let cursor=path.resolve(projectDir);
  for(let i=0;i<parts.length;i++) {
    cursor=path.join(cursor,parts[i]);const info=await fs.lstat(cursor);
    if(info.isSymbolicLink() || (i===parts.length-1 ? !info.isFile() : !info.isDirectory()))throw new Error('Invalid LaTeX root file');
  }
  const outputDir = path.resolve(projectDir, '.refract-build');
  try {await fs.mkdir(outputDir);} catch(error) {if(error.code!=='EEXIST')throw error;}
  const validateBuild=async (dir:string):Promise<void>=>{
    const info=await fs.lstat(dir);if(info.isSymbolicLink() || !info.isDirectory())throw new Error('Invalid compilation directory');
    for(const entry of await fs.readdir(dir,{withFileTypes:true})) {
      const child=path.join(dir,entry.name),stat=await fs.lstat(child);
      if(stat.isSymbolicLink())throw new Error('Links are not allowed in the compilation directory');
      if(stat.isDirectory())await validateBuild(child);
      else if(!stat.isFile() || stat.nlink>1)throw new Error('Only unlinked regular files are allowed in the compilation directory');
    }
  };
  await validateBuild(outputDir);
  if (options.signal?.aborted) return {ok:false,log:'Compilation cancelled.',exitCode:null};
  const executable = options.executable || path.join(texBin(), 'pdflatex.exe');
  const pdfPath = path.join(outputDir, 'main.pdf');
  const syncFiles=['main.synctex.gz','main.synctex','main.synctex(busy)','main.synctex.gz(busy)'];
  const clearSync=async()=>{for(const name of syncFiles)await fs.rm(path.join(outputDir,name),{force:true});};
  for (const name of ['main.pdf', 'main.log', 'main.aux', 'main.bbl', 'main.bcf']) await fs.rm(path.join(outputDir, name), { force: true });
  await clearSync();
  // Read the filename as inert characters so TeX syntax in valid filenames
  // (including percent signs and braces) cannot change the input expression.
  const draftDirectory=path.join(outputDir,'refract-internal');
  if (options.draftMode) {
    await fs.mkdir(draftDirectory,{recursive:true});
    await fs.writeFile(path.join(draftDirectory,'draft-root.txt'),`${path.resolve(projectDir,...parts).replaceAll('\\','/')}\n`,'utf8');
    await fs.writeFile(path.join(draftDirectory,'draft.tex'),String.raw`\PassOptionsToPackage{draft}{graphics}
\PassOptionsToPackage{draft}{graphicx}
\newread\refractrootstream
\openin\refractrootstream=.refract-build/refract-internal/draft-root.txt
\begingroup\endlinechar=-1
\global\readline\refractrootstream to\refractroot
\endgroup
\closein\refractrootstream
\expandafter\input\expandafter{\refractroot}
`,'utf8');
  }
  const args = ['-interaction=nonstopmode', '-halt-on-error', '-no-shell-escape', '-recorder', '-synctex=1', '-jobname=main', `-output-directory=${outputDir}`, options.draftMode ? './.refract-build/refract-internal/draft.tex' : `./${rootFile}`];
  const env: NodeJS.ProcessEnv = { ...process.env, PATH: `${path.dirname(executable)}${path.delimiter}${process.env.PATH || ''}` };
  env.BIBINPUTS = `${projectDir}${path.sep}//${path.delimiter}${process.env.BIBINPUTS || ''}`;
  // TinyTeX releases can contain paths from their build machine. Generate a
  // relocatable fontconfig file for XeTeX using the installed TeX Live font tree.
  if (/^xelatex(?:\.exe)?$/i.test(path.basename(executable))) {
    const texRoot = path.resolve(path.dirname(executable),'..','..');
    const fonts = path.join(texRoot,'texmf-dist','fonts');
    const available = await fs.stat(fonts).then(info=>info.isDirectory(),()=>false);
    if (available) {
      const escape = value => String(value).replaceAll('\\','/').replaceAll('&','&amp;').replaceAll('<','&lt;').replaceAll('>','&gt;').replaceAll('"','&quot;');
      const cache=path.join(outputDir,'font-cache'); await fs.mkdir(cache,{recursive:true});
      const systemFonts=path.join(process.env.WINDIR || 'C:\\Windows','Fonts');
      const config=path.join(outputDir,'fontconfig.conf');
      await fs.writeFile(config,`<?xml version="1.0"?><!DOCTYPE fontconfig SYSTEM "fonts.dtd"><fontconfig><dir>${escape(path.join(fonts,'opentype'))}</dir><dir>${escape(path.join(fonts,'truetype'))}</dir><dir>${escape(systemFonts)}</dir><cachedir>${escape(cache)}</cachedir></fontconfig>`,'utf8');
      env.FONTCONFIG_FILE=config;
    }
  }
  const run = (command, commandArgs, cwd = projectDir) => runCompilerProcess(command,commandArgs,cwd,env,options.signal);
  let log = '';
  let exitCode: number | null = null;
  const step = async (command, commandArgs, cwd = projectDir) => { const result = await run(command, commandArgs, cwd); exitCode = result.code; log = (log + '\n' + result.output).slice(-250000); return result.code === 0; };
  let success=false;
  try {
  if (!await step(executable, args)) return {arguments:args,ok:false,log,exitCode};
  const aux = await fs.readFile(path.join(outputDir,'main.aux'),'utf8').catch(error => { if (error.code === 'ENOENT') return ''; throw error; });
  const bcf = await fs.stat(path.join(outputDir,'main.bcf')).then(() => true, error => { if (error.code === 'ENOENT') return false; throw error; });
  if (bcf) {
    const biber = path.join(path.dirname(executable), process.platform === 'win32' ? 'biber.exe' : 'biber');
    if (!await step(biber, ['--input-directory', outputDir, '--output-directory', outputDir, 'main'])) return {arguments:args,ok:false,log,exitCode};
  } else if (/\\bibdata\{/.test(aux)) {
    const bibtex = path.join(path.dirname(executable), process.platform === 'win32' ? 'bibtex.exe' : 'bibtex');
    if (!await step(bibtex, ['main'], outputDir)) return {arguments:args,ok:false,log,exitCode};
  }
  if (!await step(executable,args)) return {arguments:args,ok:false,log,exitCode};
  if ((bcf || /\\bibdata\{/.test(aux)) && !await step(executable,args)) return {arguments:args,ok:false,log,exitCode};
  try { const bytes=await fs.readFile(pdfPath); if (!bytes.subarray(0,5).equals(Buffer.from('%PDF-'))) throw new Error('Invalid PDF'); const pdf=bytes.toString('base64');success=true;return {arguments:args,ok:true,pdf,log,exitCode}; }
  catch { return {arguments:args,ok:false,exitCode,log:'Compilation finished without producing a PDF.\n'+log}; }
  } finally {if(!success){await clearSync();await fs.rm(pdfPath,{force:true});}}
}
module.exports = { compile, runCompilerProcess };
