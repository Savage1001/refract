const {test}=require('node:test'),assert=require('node:assert/strict');
const fs=require('node:fs/promises'),path=require('node:path'),os=require('node:os');
const {spawn,spawnSync}=require('node:child_process');
const {_electron:electron}=require('playwright');

test('dependency setup removes Node mode from the installer environment', {timeout:30000}, async t=>{
 if(process.platform!=='win32')return t.skip('Windows installer behavior');
 const nsis=path.join(process.env.LOCALAPPDATA || '','electron-builder','Cache','nsis','nsis-3.0.4.1','makensis.exe');
 try {await fs.access(nsis);}catch{return t.skip('NSIS compiler is not installed');}
 const root:string=await fs.mkdtemp(path.join(os.tmpdir(),'refract-nsis-env-'));
 try {
  // Exercise the actual setup macro with an inert dependency command.
  const include=(await fs.readFile(path.resolve(__dirname,'../../scripts/installer.nsh'),'utf8')).replace(/  nsExec::ExecToLog .*\r?\n  Pop \$R0/, '  StrCpy $R0 0');
  const executable=path.join(root,'check.exe'),source=path.join(root,'check.nsi');
  await fs.writeFile(source,`!include "LogicLib.nsh"\n!define APP_EXECUTABLE_FILENAME "Refract.exe"\n${include}\nName "Environment check"\nOutFile "${executable}"\nRequestExecutionLevel user\nSilentInstall silent\nSection\n!insertmacro customInstall\nSystem::Call 'Kernel32::SetLastError(i 0)'\nSystem::Call 'Kernel32::GetEnvironmentVariable(t "ELECTRON_RUN_AS_NODE", t .r2, i 1024)i.r3 ?e'\nPop $4\n\u0024{If} $4 != 203\nSetErrorLevel 9\nQuit\n\u0024{EndIf}\nSetErrorLevel 0\nSectionEnd\n`);
  const build=spawnSync(nsis,[source],{windowsHide:true,encoding:'utf8',timeout:20000});assert.equal(build.status,0,build.stdout+build.stderr);
  const result=spawnSync(executable,[],{windowsHide:true,timeout:10000});assert.equal(result.status,0,'Node mode must be absent, not empty');
 } finally {await fs.rm(root,{recursive:true,force:true});}
});

test('silent installer reports old-uninstall errors without a dialog', {timeout:120000}, async t=>{
 if(process.platform!=='win32')return t.skip('Windows installer behavior');
 const nsis=path.join(process.env.LOCALAPPDATA || '','electron-builder','Cache','nsis','nsis-3.0.4.1','makensis.exe');
 try {await fs.access(nsis);}catch{return t.skip('NSIS compiler is not installed');}
 const parent=await fs.realpath(os.tmpdir()),root:string=await fs.mkdtemp(path.join(parent,'refract-nsis-check-'));
 t.after(async()=>{const resolved=await fs.realpath(root);if(path.dirname(resolved).toLowerCase()!==parent.toLowerCase() || !path.basename(resolved).startsWith('refract-nsis-check-'))throw new Error('Unsafe cleanup path');await fs.rm(resolved,{recursive:true,force:true,maxRetries:5,retryDelay:200});});
 const script=path.resolve(__dirname,'../../scripts/installer.nsh').replaceAll('$','$$');
 for(const handler of ['customUnInstallCheck','customUnInstallCheckCurrentUser']) for(const mode of ['success','failure','launch-error']) {
  const executable=path.join(root,handler+'-'+mode+'.exe'),source=path.join(root,handler+'-'+mode+'.nsi');
  const body=mode==='launch-error'?'SetErrors\nStrCpy $R0 0':`ClearErrors\nStrCpy $R0 ${mode==='failure'?2:0}`;
  await fs.writeFile(source,`!include "LogicLib.nsh"\n!include "${script}"\nName "Refract installer test"\nOutFile "${executable.replaceAll('$','$$')}"\nRequestExecutionLevel user\nSilentInstall silent\nSection\n${body}\n!insertmacro ${handler}\nSetErrorLevel 0\nSectionEnd\n`);
  const build=spawnSync(nsis,[source],{windowsHide:true,encoding:'utf8',timeout:20000});assert.equal(build.status,0,build.stderr || build.stdout);
  const result = await new Promise<{code:number|null; signal:NodeJS.Signals|null; stderr:string}>((resolve,reject)=>{
   const child=spawn(executable,['/S'],{windowsHide:true,stdio:['ignore','ignore','pipe']});let stderr='';
   const timer=setTimeout(()=>{child.kill();reject(new Error(`${handler} ${mode}: installer did not exit within 15 seconds`));},15000);
   child.stderr.on('data',chunk=>{stderr+=chunk.toString();});
   child.once('error',error=>{clearTimeout(timer);reject(error);});
   child.once('close',(code,signal)=>{clearTimeout(timer);resolve({code,signal,stderr});});
  });
  assert.equal(result.code,mode==='success'?0:2,`${handler} ${mode}: ${result.signal || result.stderr}`);
 }
});

// This opt-in test temporarily registers an application with Windows. Never
// replace an existing installation or shortcut belonging to the user.
test('installer upgrades and uninstalls without deleting projects', {timeout:1800000,skip:process.platform!=='win32'||process.env.REFRACT_TEST_INSTALLER!=='1'}, async t=>{
 const manifest=require('../../package.json'),name=manifest.build.productName;
 const installer=process.env.REFRACT_TEST_INSTALLER_PATH ? path.resolve(process.env.REFRACT_TEST_INSTALLER_PATH) : path.resolve(__dirname,`../../dist/${name} Setup ${manifest.version}.exe`);
 await fs.access(installer);
 const running=()=>{
  const result=spawnSync('tasklist.exe',['/FI',`IMAGENAME eq ${name}.exe`,'/FO','CSV','/NH'],{windowsHide:true,encoding:'utf8'});
  if(result.error || result.status!==0) throw new Error('Could not check for a running Refract application');
  return result.stdout.split(/\r?\n/).some(line=>line.toLowerCase().startsWith(`"${name.toLowerCase()}.exe",`));
 };
 if(running()) return t.skip('A Refract application is running');
 const {UUID}=require('builder-util-runtime');
 const guid=manifest.build.nsis.guid || UUID.v5(manifest.build.appId,UUID.parse('50e065bc-3134-11e6-9bab-38c9862bdaf3'));
 for(const hive of ['HKCU','HKLM']) for(const view of ['32','64']) for(const suffix of [`Software\\${guid}`,`Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\${guid}`]) {
  const query=spawnSync('reg.exe',['query',`${hive}\\${suffix}`,`/reg:${view}`],{windowsHide:true,stdio:'ignore'});
  if(query.status===0) return t.skip('An existing Refract installation is registered');
  if(query.error) throw query.error;
 }
 const desktop=spawnSync('powershell.exe',['-NoProfile','-NonInteractive','-Command','[Environment]::GetFolderPath("Desktop")'],{windowsHide:true,encoding:'utf8'});
 if(desktop.status!==0) throw new Error('Could not check desktop shortcut');
 const shortcut=(manifest.build.nsis.shortcutName || name)+'.lnk';
 const shortcuts=[path.join(desktop.stdout.trim(),shortcut),path.join(process.env.APPDATA,'Microsoft','Windows','Start Menu','Programs',shortcut)];
 for(const shortcut of shortcuts) {try{await fs.access(shortcut);return t.skip('An existing Refract shortcut is present');}catch(error){if(error.code!=='ENOENT')throw error;}}
 const tempParent=await fs.realpath(os.tmpdir());
 const root=await fs.mkdtemp(path.join(tempParent,'refract-installer-test-')),installDir=path.join(root,'app'),data=path.join(root,'data');
 let completed=false,installAttempted=false,activeChild=null,activeClose:Promise<void>|null=null,timeoutKill:Promise<boolean>|null=null,phase='preparing installation',phaseStarted=Date.now();
 const startPhase=(label:string)=>{phase=label;phaseStarted=Date.now();t.diagnostic(`Starting ${phase} at ${new Date(phaseStarted).toISOString()}`);};
 const phaseElapsed=()=>`${((Date.now()-phaseStarted)/1000).toFixed(1)} seconds`;
 t.signal.addEventListener('abort',()=>{
  if(completed)return;
  t.diagnostic(`Installer test aborted during: ${phase} after ${phaseElapsed()}. Test files: ${root}`);
  const child=activeChild;if(!child || !child.pid || child.exitCode!==null || child.signalCode!==null)return;
  // Capture only the test process tree before terminating it; an upgrade may
  // be waiting for an old uninstaller rather than extracting files itself.
  const command=`$all=Get-CimInstance Win32_Process; $ids=@(${child.pid}); do { $before=$ids.Count; $ids+=@($all | Where-Object { $ids -contains $_.ParentProcessId -and $ids -notcontains $_.ProcessId } | ForEach-Object { $_.ProcessId }) } while($ids.Count -gt $before); $all | Where-Object { $ids -contains $_.ProcessId } | Select-Object Name,ProcessId,ParentProcessId | Format-Table -AutoSize | Out-String`;
  const snapshot=spawnSync('powershell.exe',['-NoProfile','-NonInteractive','-Command',command],{windowsHide:true,encoding:'utf8',timeout:10000});
  if(snapshot.status===0)t.diagnostic(`Installer process tree before timeout:\n${snapshot.stdout.trim()}`);
  else t.diagnostic('Could not capture the installer process tree before timeout');
  timeoutKill=new Promise(resolve=>{
   const killer=spawn('taskkill.exe',['/PID',String(child.pid),'/T','/F'],{windowsHide:true,stdio:'ignore'});
   killer.once('error',()=>resolve(false));killer.once('close',code=>resolve(code===0));
  });
 },{once:true});
 const run=(file,args,label='installer cleanup')=>new Promise<void>((resolve,reject)=>{
  startPhase(label);
  if(running()) {reject(new Error('A Refract application is running; installer action stopped'));return;}
  const child=spawn(file,args,{windowsHide:true,stdio:'ignore'});
  activeChild=child;activeClose=new Promise(done=>child.once('close',()=>{if(activeChild===child)activeChild=null;done();}));
  child.once('error',reject);child.once('close',code=>{t.diagnostic(`${label} exited with ${code} after ${phaseElapsed()}`);code===0 ? resolve() : reject(new Error(`${label}: ${path.basename(file)} exited with ${code}`));});
 });
 const uninstaller=path.join(installDir,`Uninstall ${name}.exe`);
 const exe=path.join(installDir,`${name}.exe`);
 const waitUntilRemoved=async()=>{
  const deadline=Date.now()+60000;
  for(;;){
   let filesRemain=false;
   for(const file of [exe,...shortcuts]) {
    try {await fs.access(file);filesRemain=true;}
    catch(error) {
     // Windows can report a delete-pending executable as access denied before
     // reporting ENOENT. Keep waiting; a persistent denial still times out.
     if(process.platform==='win32' && ['EPERM','EACCES','EBUSY'].includes(error.code))filesRemain=true;
     else if(error.code!=='ENOENT')throw error;
    }
   }
   const registration=spawnSync('reg.exe',['query',`HKCU\\Software\\${guid}`],{windowsHide:true,stdio:'ignore'});
   if(registration.error)throw registration.error;
   if(!filesRemain && registration.status!==0)return;
   if(Date.now()>=deadline) throw new Error('Uninstaller did not remove the application');
   await new Promise(resolve=>setTimeout(resolve,250));
  }
 };
 t.after(async()=>{
  if(timeoutKill && !await timeoutKill) throw new Error(`Could not stop the test installer; its files remain at ${root}`);
  if(activeClose) await activeClose;
  if(installAttempted) {
   try {await fs.access(uninstaller);await run(uninstaller,['/S']);await waitUntilRemoved();}
   catch(error) {
    if(error.code!=='ENOENT') throw error;
    const key=spawnSync('reg.exe',['query',`HKCU\\Software\\${guid}`],{windowsHide:true,stdio:'ignore'});
    if(key.error || key.status===0) throw new Error(`Installation cleanup needs attention at ${root}`);
   }
  }
  const resolved=await fs.realpath(root);
  if(path.dirname(resolved).toLowerCase()!==tempParent.toLowerCase() || !path.basename(resolved).startsWith('refract-installer-test-')) throw new Error(`Unsafe cleanup path: ${resolved}`);
  await fs.rm(resolved,{recursive:true,force:true,maxRetries:5,retryDelay:200});
 });
 installAttempted=true;await run(installer,['/S',`/D=${installDir}`],'initial installation');
 await fs.access(exe);await fs.access(uninstaller);
 const env: NodeJS.ProcessEnv={...process.env,REFRACT_DATA_DIR:data,REFRACT_TEST_HIDDEN:'1'};delete env.ELECTRON_RUN_AS_NODE;
 const app=await electron.launch({executablePath:exe,args:[],env});
 const content='\\documentclass{article}\n\\begin{document}\nSaved through upgrade.\n\\end{document}';
 try {
  const page=await app.firstWindow();await page.locator('#new').click();await page.locator('#source').waitFor({state:'visible'});await page.locator('#source').fill(content);
  // A silent upgrade must refuse to proceed while Refract is running and must
  // leave that process alive, rather than force-closing unsaved work.
  startPhase('upgrade while app is running');
  const blocked: number | null = await new Promise((resolve,reject)=>{
   const child=spawn(installer,['/S',`/D=${installDir}`],{windowsHide:true,stdio:'ignore'});
   let timedOut = false;
   const timer=setTimeout(()=>{ timedOut = true; if(child.pid) spawnSync('taskkill.exe',['/PID',String(child.pid),'/T','/F'],{windowsHide:true,stdio:'ignore'}); },60000);
   child.once('error',error=>{clearTimeout(timer);reject(error);});child.once('close',code=>{clearTimeout(timer);if(timedOut) reject(new Error('Blocked upgrade did not exit within 60 seconds')); else resolve(code);});
  });
  assert.notEqual(blocked,null,'The installer was terminated rather than refusing the upgrade');
  t.diagnostic(`Running-app upgrade check exited with ${blocked} after ${phaseElapsed()}`);
  assert.notEqual(blocked,0,'An upgrade should refuse while Refract is running');
  assert.equal(app.process().exitCode,null,'Upgrade unexpectedly closed Refract');
  assert.equal(await page.locator('#source').inputValue(),content);
 }
 finally {await app.close();}
 const projects=(await fs.readdir(path.join(data,'projects'))).filter((name:string)=>/^[0-9a-f-]{36}$/.test(name));assert.equal(projects.length,1);
 await run(installer,['/S',`/D=${installDir}`],'upgrade after closing app');assert.deepEqual((await fs.readdir(path.join(data,'projects'))).filter((name:string)=>/^[0-9a-f-]{36}$/.test(name)),projects);
 const reopened=await electron.launch({executablePath:exe,args:[],env});
 try {const page=await reopened.firstWindow();await page.locator('#list article').first().click();await page.locator('#source').waitFor({state:'visible'});assert.equal(await page.locator('#source').inputValue(),content);}
 finally {await reopened.close();}
 await run(uninstaller,['/S'],'uninstall');await waitUntilRemoved();installAttempted=false;
 assert.deepEqual((await fs.readdir(path.join(data,'projects'))).filter((name:string)=>/^[0-9a-f-]{36}$/.test(name)),projects);
 assert.equal(await fs.readFile(path.join(data,'projects',projects[0],'main.tex'),'utf8'),content);
 await assert.rejects(fs.access(exe));
 const registration=spawnSync('reg.exe',['query',`HKCU\\Software\\${guid}`],{windowsHide:true,stdio:'ignore'});
 if(registration.error) throw registration.error;
 assert.notEqual(registration.status,0,'Uninstall left the test installation registered');
 for(const shortcut of shortcuts) await assert.rejects(fs.access(shortcut));
 completed=true;
});
