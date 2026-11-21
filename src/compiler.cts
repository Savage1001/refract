const { texBin } = require('./tex-toolchain.cjs');
const { spawn } = require('node:child_process');
import fs = require('node:fs/promises');
import path = require('node:path');
import sources = require('./source-cache.cjs');
const {mapLimited} = sources;
const { cancelProcess } = require('./chat.cjs');

async function validateBuildDirectory(root: string): Promise<void> {
  const info = await fs.lstat(root);
  if (info.isSymbolicLink() || !info.isDirectory()) throw new Error('Invalid compilation directory');
  let directories = [root];
  while (directories.length) {
    const entries = await mapLimited(directories, async directory =>
      (await fs.readdir(directory)).map(name => path.join(directory, name)));
    const children = await mapLimited(entries.flat(), async file => {
      const stat = await fs.lstat(file);
      if (stat.isSymbolicLink()) throw new Error('Links are not allowed in the compilation directory');
      if (stat.isDirectory()) return file;
      if (!stat.isFile() || stat.nlink > 1) throw new Error('Only unlinked regular files are allowed in the compilation directory');
    });
    directories = children.filter((child): child is string => child !== undefined);
  }
}

async function removeBuildFiles(directory: string, names: string[]): Promise<void> {
  const results = await Promise.allSettled(names.map(name => fs.rm(path.join(directory, name), {force: true})));
  for (const result of results) if (result.status === 'rejected') throw result.reason;
}

async function firstPassComplete(projectDir: string, outputDir: string, executable: string, aux: string, log: string): Promise<boolean> {
  const engine = path.basename(executable).toLowerCase().replace(/\.exe$/, '');
  if (!['pdflatex', 'xelatex', 'lualatex'].includes(engine)) return false;
  // Plain LaTeX bookkeeping is safe to ignore. Any other auxiliary commands
  // may feed the next pass, even when the package doesn't ask for a rerun.
  if (!/^\\relax[ \t]*\r?\n\\gdef \\@abspage@last\{[1-9]\d*\}\r?\n?$/.test(aux)) return false;
  if (/(?:\brerun\b|\bre-run\b|run (?:La)?TeX again|undefined (?:references|citations)|^No file (?!main\.aux\.$))/im.test(log)) return false;
  try {
    const recorder = path.join(outputDir, 'main.fls');
    const info = await fs.lstat(recorder);
    if (!info.isFile() || info.nlink > 1 || info.size > 8 * 1024 * 1024) return false;
    const files = await fs.readFile(recorder, 'utf8');
    const standardOutputs = new Set(['main.aux', 'main.log', 'main.pdf', 'main.xdv'].map(name => path.join(outputDir, name)));
    const outputs = new Set(files.split(/\r?\n/).filter(line => line.startsWith('OUTPUT '))
      .map(line => path.resolve(projectDir, line.slice(7))));
    // A package may write a state file without using the main aux or logging a
    // warning. Keep the second pass whenever the recorder lists extra outputs,
    // except LuaLaTeX's transient font-cache write probe outside the project.
    if (!outputs.has(path.join(outputDir, 'main.aux'))) return false;
    for (const file of outputs) {
      if (standardOutputs.has(file)) continue;
      const relative = path.relative(projectDir, file);
      if (engine !== 'lualatex' || path.basename(file) !== 'm_t_x_t_e_s_t.tmp' ||
          (!relative.startsWith('..' + path.sep) && !path.isAbsolute(relative))) return false;
      if (await fs.lstat(file).then(() => true, error => { if (error.code === 'ENOENT') return false; throw error; })) return false;
    }
    return true;
  } catch {
    return false;
  }
}

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
async function compile(projectDir: string, options: { executable?: string; rootFile?: string; draftMode?: boolean; signal?:AbortSignal } = {}) {
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
  await validateBuildDirectory(outputDir);
  if (options.signal?.aborted) return {ok:false,log:'Compilation cancelled.',exitCode:null};
  const executable = options.executable || path.join(texBin(), 'pdflatex.exe');
  const pdfPath = path.join(outputDir, 'main.pdf');
  const syncFiles=['main.synctex.gz','main.synctex','main.synctex(busy)','main.synctex.gz(busy)'];
  await removeBuildFiles(outputDir, ['main.pdf', 'main.log', 'main.aux', 'main.bbl', 'main.bcf', 'main.fls', ...syncFiles]);
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
  const args = ['-interaction=nonstopmode', '-halt-on-error', '-file-line-error', '-no-shell-escape', '-recorder', '-synctex=1', '-jobname=main', `-output-directory=${outputDir}`, options.draftMode ? './.refract-build/refract-internal/draft.tex' : `./${rootFile}`];
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
  let log = '';
  let exitCode: number | null = null;
  const step = async (command: string, commandArgs: string[], cwd = projectDir) => {
    const result = await runCompilerProcess(command, commandArgs, cwd, env, options.signal);
    exitCode = result.code;
    log = (log + '\n' + result.output).slice(-250000);
    return result.code === 0;
  };
  const failed = () => ({arguments: args, ok: false, log, exitCode});
  let success = false;
  try {
    if (!await step(executable, args)) return failed();
    const [aux, bcf] = await Promise.all([
      fs.readFile(path.join(outputDir, 'main.aux'), 'utf8').catch(error => { if (error.code === 'ENOENT') return ''; throw error; }),
      fs.stat(path.join(outputDir, 'main.bcf')).then(() => true, error => { if (error.code === 'ENOENT') return false; throw error; })
    ]);
    const bibtexRequired = /\\bibdata\{/.test(aux);
    if (bcf) {
      const biber = path.join(path.dirname(executable), process.platform === 'win32' ? 'biber.exe' : 'biber');
      if (!await step(biber, ['--input-directory', outputDir, '--output-directory', outputDir, 'main'])) return failed();
    } else if (bibtexRequired) {
      const bibtex = path.join(path.dirname(executable), process.platform === 'win32' ? 'bibtex.exe' : 'bibtex');
      if (!await step(bibtex, ['main'], outputDir)) return failed();
    }
    const bibliography = bcf || bibtexRequired;
    const needsSecondPass = bibliography || !await firstPassComplete(projectDir, outputDir, executable, aux, log);
    if (needsSecondPass && !await step(executable, args)) return failed();
    if (bibliography && !await step(executable, args)) return failed();
    if (options.signal?.aborted) return {...failed(), log: log + '\nCompilation cancelled.', exitCode: null};
    try {
      const bytes = await fs.readFile(pdfPath);
      if (!bytes.subarray(0, 5).equals(Buffer.from('%PDF-'))) throw new Error('Invalid PDF');
      const pdf = bytes.toString('base64');
      success = true;
      return {arguments: args, ok: true, pdf, log, exitCode};
    } catch {
      return {...failed(), log: 'Compilation finished without producing a PDF.\n' + log};
    }
  } finally {
    if (!success) await removeBuildFiles(outputDir, [...syncFiles, 'main.pdf']);
  }
}
export = { compile, runCompilerProcess };
