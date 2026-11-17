import test = require('node:test');
import assert = require('node:assert/strict');
import fs = require('node:fs/promises');
import path = require('node:path');
import os = require('node:os');
import childProcess = require('node:child_process');

test('NSIS dependency hook runs the packaged helper hidden and propagates setup failure', {timeout:600000}, async t => {
  if (process.platform !== 'win32') return t.skip('Windows installer hook');
  const app=path.resolve(__dirname,'../../dist/win-unpacked');
  try { await fs.access(path.join(app,'resources/app/build/scripts/install-dependencies.cjs')); }
  catch { return t.skip('Build the packaged application first'); }
  const cache=path.join(process.env.LOCALAPPDATA ?? '', 'electron-builder/Cache');
  let compiler=path.join(cache,'nsis/nsis-3.0.4.1/makensis.exe');
  try { await fs.access(compiler); } catch {
    const directory=path.join(cache,'nsis-3.0.4.1');
    const entries=await fs.readdir(directory).catch(()=>[] as string[]);
    const candidates=entries.map(entry=>path.join(directory,entry,'makensis.exe'));
    const available=await Promise.all(candidates.map(file=>fs.access(file).then(()=>file,()=>undefined)));
    compiler=available.find((file):file is string=>Boolean(file)) ?? '';
    if (!compiler) return t.skip('NSIS compiler is unavailable');
  }
  const root=await fs.mkdtemp(path.join(os.tmpdir(),'refract-dependency-hook-'));
  t.after(async()=>{await fs.rm(root,{recursive:true,force:true,maxRetries:5,retryDelay:200});});
  const installer=path.join(root,'dependency-hook.exe'),source=path.join(root,'dependency-hook.nsi');
  const quote=(value:string)=>value.replaceAll('$','$$');
  await fs.writeFile(source,`!include "LogicLib.nsh"
!define APP_EXECUTABLE_FILENAME "Refract.exe"
!include "${quote(path.resolve(__dirname,'../../scripts/installer.nsh'))}"
Name "Refract dependency hook test"
OutFile "${quote(installer)}"
RequestExecutionLevel user
SilentInstall silent
Section
StrCpy $INSTDIR "${quote(app)}"
!insertmacro customInstall
SetErrorLevel 0
SectionEnd
`);
  const compiled=childProcess.spawnSync(compiler,[source],{windowsHide:true,encoding:'utf8',timeout:20000});
  assert.equal(compiled.status,0,compiled.stderr || compiled.stdout);
  const data=path.join(root,'data'),bin=path.join(data,'Refract/dependencies/TinyTeX/bin/windows');
  const live=process.env.REFRACT_TEST_DEPENDENCY_DOWNLOAD==='1';
  if (!live) {
    await fs.mkdir(bin,{recursive:true});
    for (const tool of ['pdflatex','xelatex','lualatex','kpsewhich','synctex','bibtex','biber']) await fs.writeFile(path.join(bin,`${tool}.exe`),'reuse fixture');
  }
  const run=(localAppData:string)=>childProcess.spawnSync(installer,['/S'],{windowsHide:true,encoding:'utf8',timeout:live?300000:15000,env:{...process.env,LOCALAPPDATA:localAppData}});
  const reused=run(data);assert.equal(reused.status,0,String(reused.error ?? reused.stderr));
  if (live) {
    for (const tool of ['pdflatex','xelatex','lualatex','kpsewhich','synctex','bibtex','biber']) assert.ok((await fs.stat(path.join(bin,`${tool}.exe`))).size>1000,tool);
    assert.equal(run(data).status,0,'freshly downloaded toolchain must be reusable');
    t.diagnostic('Fresh upstream download, checksum verification, extraction and reuse passed through the NSIS hook');
  }
  assert.deepEqual(await fs.readdir(path.join(data,'Refract/dependencies')),['TinyTeX']);
  const blocked=path.join(root,'blocked');await fs.writeFile(blocked,'not a directory');
  const failed=run(blocked);assert.equal(failed.status,3,String(failed.error ?? failed.stderr));
});
