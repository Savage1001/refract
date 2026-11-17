const { UpdateService } = require('./src/updates.cjs');
import projectTrust = require('./src/project-trust.cjs');
const { texBin } = require('./src/tex-toolchain.cjs');
const { app, BrowserWindow, ipcMain, dialog, shell, nativeTheme } = require('electron');
const fs = require('node:fs/promises');
const path = require('node:path');
const { ProjectStore } = require('./src/project-store.cjs');
const { compile } = require('./src/compiler.cjs');
const { configuredBuild, lookupPackage } = require('./src/assistant-build-bridge.cjs');
const { startAssistantTools } = require('./src/assistant-tools.cjs');
const crypto = require('node:crypto');
const synctex = require('./src/synctex.cjs');
const { ChatService } = require('./src/chat.cjs');
const {runWarmCodex,closeCodexRuntimes,closeCodexWorkspace} = require('./src/codex-runtime.cjs');
const {AssistantPerformance} = require('./src/assistant-performance.cjs');
const codexModels: typeof import('./src/codex-models.cjs') = require('./src/codex-models.cjs');
const { CodexPreferencesStore } = require('./src/codex-preferences.cjs');
const { importProject, exportProject } = require('./src/archive.cjs');
const { Versions } = require('./src/versions.cjs');
const { Comments } = require('./src/comments.cjs');
const { inspectFile } = require('./src/file-preview.cjs');
const { AssistantCoordinator } = require('./src/assistant-coordinator.cjs');
let writes = Promise.resolve();
let flushSnapshots = () => {};
let hasPendingSnapshots = () => false;
let shuttingDown = false;
let allowQuit = false;
let mainWindow: any = null;
let waitForAssistants = async () => {};
let cancelCompilations = () => {};
let waitForCompilations = async () => {};
let acceptingAssistantOperations = true;
async function shutdownBarrier(window) {
  if(!window || window.isDestroyed() || window.webContents.isDestroyed()) return;
  await new Promise<void>(resolve=>{
    const contents=window.webContents;
    const finish=()=>{clearTimeout(timer);ipcMain.removeListener('app:shutdown-ready',onReady);contents.removeListener('destroyed',finish);resolve();};
    const onReady=event=>{if(event.sender===contents) finish();};
    const timer=setTimeout(finish,5000);
    ipcMain.on('app:shutdown-ready',onReady);contents.once('destroyed',finish);
    try {contents.send('app:prepare-shutdown');} catch {finish();}
  });
}
function beginShutdown() {
  if(shuttingDown) return;shuttingDown=true;
  cancelCompilations();
  void (async()=>{
    if(mainWindow && !mainWindow.isDestroyed()) mainWindow.hide();
    await shutdownBarrier(mainWindow);
    await waitForCompilations();
    acceptingAssistantOperations=false;
    await waitForAssistants();
    await closeCodexRuntimes();
    for(;;) {
      flushSnapshots();const pending=writes;
      try {await pending;} catch(error) {console.error(error);}
      if(pending===writes && !hasPendingSnapshots()) break;
    }
    allowQuit=true;app.quit();
  })().catch(error=>{console.error(error);allowQuit=true;app.quit();});
}
if (process.env.REFRACT_DATA_DIR) app.setPath('userData', process.env.REFRACT_DATA_DIR);
app.whenReady().then(async () => {
  const updates = new UpdateService(path.join(app.getPath('userData'), 'update-settings.json'), app.getVersion(), app.isPackaged && process.platform === 'win32');
  await updates.start();
  app.on('before-quit', () => updates.stop());
  ipcMain.handle('updates:status', () => updates.status());
  ipcMain.handle('updates:check', async () => { await updates.check(); return updates.status(); });
  ipcMain.handle('updates:track', (_, track: unknown) => updates.setTrack(track));
  ipcMain.handle('providers:discover', () => require('./src/providers.cjs').discoverProviders());
  const store = new ProjectStore(path.join(app.getPath('userData'), 'projects'));
  const cleanupDeletedProjects=()=>{
    const operation=writes.catch(()=>{}).then(()=>store.listDeletedProjects());
    writes=operation;return operation;
  };
  cleanupDeletedProjects().catch(console.error);
  setInterval(()=>{cleanupDeletedProjects().catch(console.error);},60*60*1000).unref();
  const chats = new ChatService(store,runWarmCodex);
  const {BuildCache} = require('./src/build-cache.cjs');
  const buildCache = new BuildCache(store,compile);
  const assistantPerformance = new Map<string,import('./src/assistant-performance.cjs').PerformanceReport>();
  ipcMain.handle('assistant:performance',(_,id)=>assistantPerformance.get(id) ?? null);
  const codexPreferences = new CodexPreferencesStore(path.join(app.getPath('userData'), 'codex-preferences.json'));
  const openCodePreferences = new CodexPreferencesStore(path.join(app.getPath('userData'), 'opencode-preferences.json'), true);
  ipcMain.handle('opencode-preferences:load', () => openCodePreferences.load());
  ipcMain.handle('opencode-preferences:save', (_, value: unknown) => openCodePreferences.save(value));
  ipcMain.handle('opencode:discover-models', async (_, projectId?: string) => { if (projectId) await projectTrust.assertTrusted(store.projectDir(projectId)); return require('./src/opencode.cjs').discoverOpenCodeModels(undefined, projectId ? store.projectDir(projectId) : undefined); });
  ipcMain.handle('codex-preferences:load', () => codexPreferences.load());
  ipcMain.handle('codex-preferences:save', (_, value: unknown) => codexPreferences.save(value));
  const versions = new Versions(store);
  const comments = new Comments(store);
  const compilations = new Map<string, Promise<any>>();
  const compilationControllers = new Set<AbortController>();
  cancelCompilations=()=>{for(const controller of compilationControllers)controller.abort();};
  waitForCompilations=async()=>{await Promise.allSettled([...compilations.values()]);};
  const waitForCompilation = async id => { const job = compilations.get(id); if (job) await job.catch(() => {}); };
  const readText = async (id, name) => {
    try {
      const bytes = await fs.readFile(store.filePath(id,name));
      if (bytes.includes(0)) return null;
      try { return new TextDecoder('utf-8',{fatal:true,ignoreBOM:true}).decode(bytes); } catch { return null; }
    } catch (error) { if (['ENOENT','ENOTDIR','EISDIR'].includes(error.code)) return null; throw error; }
  };
  const writeFile = async (id, name, content) => {
    await waitForCompilation(id);
    const before = await readText(id,name);
    let after = typeof content === 'string' ? content : null;
    if (Buffer.isBuffer(content) && !content.includes(0)) { try { after = new TextDecoder('utf-8',{fatal:true,ignoreBOM:true}).decode(content); } catch {} }
    const updated = await comments.planEdit(id,name,before,after);
    await store.write(id,name,content);
    if (updated) {
      try { await comments.save(id,updated); }
      catch (error) { throw new Error(`File was saved, but comment anchors could not be updated: ${error.message}`); }
    }
  };
  const snapshotTimers = new Map();
  hasPendingSnapshots=()=>snapshotTimers.size>0;
  const deletingProjects = new Set<string>();
  const failedSaves = new Map<string, Set<string>>();
  const enqueue = fn => {const operation=writes.catch(()=>{}).then(fn);writes=operation;return operation;};
  const assistant=new AssistantCoordinator(
    enqueue,
    async id=>{if(deletingProjects.has(id)) throw new Error('Project deletion is in progress');if(failedSaves.get(id)?.size) throw new Error('Save pending changes before sending a message.');await store.metadata(id);await waitForCompilation(id);},
    id=>versions.snapshot(id)
  );
  const assertAssistantIdle = id=>{if(assistant.isBusy(id)) throw new Error('Wait for the assistant to finish changing this project');};
  waitForAssistants=()=>assistant.waitForAll();
  const snapshot = id => {
    writes = writes.catch(() => {}).then(() => versions.snapshot(id));
    return writes;
  };
  flushSnapshots = () => {
    for (const [id, timer] of snapshotTimers) { clearTimeout(timer); snapshot(id); }
    snapshotTimers.clear();
  };
  await store.init();
  // A restore journal survives termination until the pre-restore snapshot is recovered.
  for (const project of await store.list()) await versions.recoverInterrupted(project.id);
  // Preserve projects made by the earlier prototype, without overwriting the source.
  const legacy = path.join(app.getPath('userData'), 'projects.json');
  const marker = path.join(app.getPath('userData'), 'legacy-migrated.json');
  try {
    await fs.access(marker);
  } catch (error) {
    if (error.code !== 'ENOENT') throw error;
    let old = [];
    try { old = JSON.parse(await fs.readFile(legacy, 'utf8')); } catch (e) { if (e.code !== 'ENOENT') throw e; }
    if (!Array.isArray(old)) throw new Error('Invalid legacy project data');
    // Record each migration before continuing, so interrupted migrations can resume.
    const donePath = path.join(app.getPath('userData'), 'legacy-migration-progress.json');
    let done = [];
    try { done = JSON.parse(await fs.readFile(donePath, 'utf8')); } catch (e) { if (e.code !== 'ENOENT') throw e; }
    for (let i = 0; i < old.length; i++) {
      if (done.includes(i)) continue;
      const project = await store.create(old[i].name);
      if (typeof old[i].source === 'string') await store.write(project.id, 'main.tex', old[i].source);
      done.push(i); await fs.writeFile(donePath, JSON.stringify(done));
    }
    await fs.writeFile(marker, JSON.stringify({ count: done.length }));
  }
  ipcMain.handle('external:open', async (_, value) => {
    if(typeof value!=='string' || value.length>8192 || /[\u0000-\u001f\u007f]/.test(value)) throw new Error('Invalid link');
    let url: URL; try {url=new URL(value);} catch {throw new Error('Invalid link');}
    if(!['https:','http:'].includes(url.protocol) || url.username || url.password) throw new Error('Unsupported link');
    await shell.openExternal(url.href);
  });
  ipcMain.handle('codex:discover-models', async (): Promise<import('./src/codex-models.cjs').ModelDiscoveryResult> => {
    try { return await codexModels.discoverCodexModels(); }
    catch { return { ok: false, error: 'Codex model discovery failed' }; }
  });
  ipcMain.handle('projects:list', () => store.list());
  ipcMain.handle('projects:trust-status', (_,id:string) => projectTrust.isTrusted(store.projectDir(id)));
  ipcMain.handle('projects:trust-request', async (event,id:string) => {
    assertAssistantIdle(id);await writes;await waitForCompilation(id);assertAssistantIdle(id);
    return projectTrust.requestTrust(store.projectDir(id),async message => {
      const result=await dialog.showMessageBox(BrowserWindow.fromWebContents(event.sender), {type:'warning',title:'Trust this project?',message:'Allow compilation and assistant execution?',detail:message,buttons:['Cancel','Trust project'],defaultId:0,cancelId:0,noLink:true});
      return result.response===1;
    });
  });
  ipcMain.handle('projects:trust-revoke', async (_,id:string) => {
    assertAssistantIdle(id);await writes;await waitForCompilation(id);assertAssistantIdle(id);
    await projectTrust.revokeTrust(store.projectDir(id));
  });
  ipcMain.handle('projects:create', (_, name) => store.create(name));
  ipcMain.handle('projects:trash', (_, id) => {
    assertAssistantIdle(id);
    if(deletingProjects.has(id)) throw new Error('Project deletion is already in progress');
    deletingProjects.add(id);
    const hadTimer=snapshotTimers.has(id);clearTimeout(snapshotTimers.get(id));snapshotTimers.delete(id);
    const operation=writes.catch(()=>{}).then(async()=>{
      if([...chats.busy].some(key=>key.startsWith(id+':'))) throw new Error('Wait for the assistant to finish before deleting this project');
      await waitForCompilation(id);
      await closeCodexWorkspace(store.projectDir(id));await versions.snapshot(id);await store.trashProject(id);
    });
    writes=operation;
    return operation.then(()=>{deletingProjects.delete(id);},error=>{
      deletingProjects.delete(id);
      if(hadTimer) snapshotTimers.set(id,setTimeout(()=>{snapshotTimers.delete(id);snapshot(id).catch(console.error);},3000));
      throw error;
    });
  });
  ipcMain.handle('projects:trash-list', ()=>cleanupDeletedProjects());
  ipcMain.handle('projects:trash-restore', (_,token)=>{
    writes=writes.catch(()=>{}).then(()=>store.restoreProject(token));return writes;
  });
  ipcMain.handle('projects:duplicate', (_, id) => {
    assertAssistantIdle(id);
    writes=writes.catch(()=>{}).then(async()=>{
      if ([...chats.busy].some(key=>key.startsWith(id+':'))) throw new Error('Wait for the assistant to finish before duplicating this project');
      return store.duplicate(id);
    }); return writes;
  });
  ipcMain.handle('projects:rename', (_, id, name) => {
    assertAssistantIdle(id);
    writes = writes.catch(() => {}).then(() => store.renameProject(id, name)); return writes;
  });
  ipcMain.handle('projects:draft', (_, id: string, enabled: boolean) => {
    assertAssistantIdle(id);
    writes=writes.catch(()=>{}).then(()=>store.setDraftMode(id,enabled)); return writes;
  });
  ipcMain.handle('projects:engine', (_, id, engine) => {
    assertAssistantIdle(id);
    writes=writes.catch(()=>{}).then(()=>store.setEngine(id,engine)); return writes;
  });
  ipcMain.handle('projects:import', async event => {
    const selected = await dialog.showOpenDialog(BrowserWindow.fromWebContents(event.sender), { properties: ['openFile'], filters: [{ name: 'ZIP archive', extensions: ['zip'] }] });
    if (selected.canceled || !selected.filePaths[0]) return null;
    const archive = selected.filePaths[0];
    return importProject(store, archive, path.basename(archive, path.extname(archive)), async candidates => {
      const result = await dialog.showMessageBox(BrowserWindow.fromWebContents(event.sender), {
        type: 'question', message: 'Select the main LaTeX document',
        detail: 'This document will be used when compiling the imported project.',
        buttons: [...candidates, 'Import all', 'Cancel'], cancelId: candidates.length + 1, noLink: true
      });
      return result.response === candidates.length + 1 ? null : result.response === candidates.length ? candidates : candidates[result.response];
    });
  });
  ipcMain.handle('projects:export', async (event, id) => {
    assertAssistantIdle(id);
    await writes;
    const selected = await dialog.showSaveDialog(BrowserWindow.fromWebContents(event.sender), { defaultPath: 'project.zip', filters: [{ name: 'ZIP archive', extensions: ['zip'] }] });
    if (!selected.canceled && selected.filePath) {assertAssistantIdle(id);await enqueue(()=>exportProject(store,id,selected.filePath));}
  });
  ipcMain.handle('files:read', async (_, id, name) => { await writes; return store.read(id, name); });
  ipcMain.handle('files:open', async (_, id, name) => { await writes; return inspectFile(store,id,name); });
  ipcMain.handle('files:image', async (_, id, name) => {
    const extensions = { '.png': 'image/png', '.jpg': 'image/jpeg', '.jpeg': 'image/jpeg', '.gif': 'image/gif', '.webp': 'image/webp' };
    const type = extensions[path.extname(name).toLowerCase()];
    if (!type) throw new Error('Unsupported image format');
    const file = await store.existingFile(id, name), info = await fs.stat(file);
    if (info.size > 32 * 1024 * 1024) throw new Error('Image is too large to preview');
    return `data:${type};base64,${(await fs.readFile(file)).toString('base64')}`;
  });
  ipcMain.handle('files:list', (_, id) => store.files(id));
  ipcMain.handle('files:tree', (_, id) => store.tree(id));
  ipcMain.handle('files:search', async (_, id, query) => { await writes; return store.search(id, query); });
  ipcMain.handle('files:create', (_, id, name) => {
    assertAssistantIdle(id);
    writes = writes.catch(() => {}).then(() => store.createFile(id, name)); return writes;
  });
  ipcMain.handle('files:rename', (_, id, oldName, newName) => {
    assertAssistantIdle(id);
    writes = writes.catch(() => {}).then(async () => {
      await waitForCompilation(id);
      await versions.snapshot(id);
      const meta = await store.renameFile(id, oldName, newName);
      try { await comments.renameFile(id,oldName,newName); }
      catch (error) {
        try { await store.renameFile(id,newName,oldName); }
        catch (rollback) { throw new Error(`Could not update comments: ${error.message}. File rename could not be reversed: ${rollback.message}`); }
        throw error;
      }
      return meta;
    }); return writes;
  });
  ipcMain.handle('files:trash', (_, id, name) => {
    assertAssistantIdle(id);
    writes = writes.catch(() => {}).then(async () => { await waitForCompilation(id); await versions.snapshot(id); return store.trashFile(id, name); }); return writes;
  });
  ipcMain.handle('files:trash-folder', (_,id,name) => {
    assertAssistantIdle(id);
    writes=writes.catch(()=>{}).then(async()=>{
      if([...chats.busy].some(key=>key.startsWith(id+':'))) throw new Error('Wait for the assistant to finish before deleting folders');
      await waitForCompilation(id);await versions.snapshot(id);return store.trashFolder(id,name);
    });return writes;
  });
  ipcMain.handle('files:trash-list', async (_, id) => { await writes; return store.listTrash(id); });
  ipcMain.handle('files:trash-restore', (_, id, token) => {
    assertAssistantIdle(id);
    writes=writes.catch(()=>{}).then(async()=>{await waitForCompilation(id); return store.restoreTrash(id,token);}); return writes;
  });
  ipcMain.handle('files:download', async (event, id, name) => {
    assertAssistantIdle(id);
    await writes; await store.existingFile(id, name);
    const result = await dialog.showSaveDialog(BrowserWindow.fromWebContents(event.sender), { defaultPath: path.basename(name) });
    if (!result.canceled && result.filePath) {assertAssistantIdle(id);await enqueue(async()=>fs.copyFile(await store.existingFile(id,name),result.filePath));}
  });
  ipcMain.handle('files:folder', (_, id, name) => {
    assertAssistantIdle(id);
    writes = writes.catch(() => {}).then(() => store.createFolder(id, name)); return writes;
  });
  ipcMain.handle('files:move-folder', (_,id,oldName,newName) => {
    assertAssistantIdle(id);
    writes=writes.catch(()=>{}).then(async()=>{
      if(typeof oldName!=='string' || typeof newName!=='string') throw new Error('Invalid folder path');
      oldName=oldName.replaceAll('\\','/');newName=newName.replaceAll('\\','/');
      if([...chats.busy].some(key=>key.startsWith(id+':'))) throw new Error('Wait for the assistant to finish before moving folders');
      await waitForCompilation(id); await versions.snapshot(id);
      const meta=await store.moveFolder(id,oldName,newName);
      try {await comments.moveFolder(id,oldName,newName);}
      catch(error) {try {await store.moveFolder(id,newName,oldName);} catch(rollback) {throw new Error(`Could not update comments: ${error.message}. Could not reverse the move: ${rollback.message}`);} throw error;}
      return meta;
    });return writes;
  });
  ipcMain.handle('files:import', async (event, id, destination) => {
    assertAssistantIdle(id);
    const selected = await dialog.showOpenDialog(BrowserWindow.fromWebContents(event.sender), { properties: ['openFile', 'multiSelections'] });
    if (selected.canceled) return [];
    assertAssistantIdle(id);
    writes = writes.catch(() => {}).then(() => store.importFiles(id, selected.filePaths,destination)); return writes;
  });
  ipcMain.handle('files:import-folder', async (event, id, destination) => {
    assertAssistantIdle(id);
    const selected = await dialog.showOpenDialog(BrowserWindow.fromWebContents(event.sender), { properties: ['openDirectory'] });
    if (selected.canceled || !selected.filePaths[0]) return null;
    assertAssistantIdle(id);
    writes = writes.catch(() => {}).then(() => store.importFolder(id, selected.filePaths[0],destination)); return writes;
  });
  ipcMain.handle('chat:image-preview', async (_, id, token) => { await writes; return chats.imagePreview(id, token); });
  ipcMain.handle('chat:load', (_, id, chatId) => chats.load(id, chatId));
  ipcMain.handle('comments:list', async (_, id) => { await writes; return comments.list(id); });
  ipcMain.handle('comments:add', (_, id, file, quote, text, selection) => {
    assertAssistantIdle(id);
    writes = writes.catch(() => {}).then(() => comments.add(id, file, quote, text, selection)); return writes;
  });
  ipcMain.handle('comments:locate', async (_, id, commentId) => { await writes; return comments.locate(id,commentId); });
  ipcMain.handle('comments:reply', (_, id, commentId, text) => {
    assertAssistantIdle(id);
    writes = writes.catch(() => {}).then(() => comments.reply(id,commentId,text)); return writes;
  });
  ipcMain.handle('comments:resolve', (_, id, commentId, resolved) => {
    assertAssistantIdle(id);
    writes = writes.catch(() => {}).then(() => comments.resolve(id, commentId, resolved)); return writes;
  });
  ipcMain.handle('versions:list', async (_, id) => { await writes; return versions.list(id); });
  ipcMain.handle('versions:retained-stages', async (_, id) => { await writes; return versions.retainedStages(id); });
  ipcMain.handle('versions:snapshot', (_, id) => assistant.isBusy(id) ? null : snapshot(id));
  ipcMain.handle('versions:compare', async (_, id, versionId) => { await writes; return versions.compare(id, versionId); });
  ipcMain.handle('versions:restore-project', (_, id, versionId) => {
    assertAssistantIdle(id);
    const operation = writes.catch(() => {}).then(async () => {
      await waitForCompilation(id);
      const files = [...new Set<string>((await comments.list(id)).filter(c => c.anchor).map(c => c.file))];
      const before = new Map<string,string | null>();
      for (const file of files) before.set(file,await readText(id,file));
      const result = await versions.restore(id, versionId);
      try {
        for (const file of files) {
          const updated = await comments.planEdit(id,file,before.get(file),await readText(id,file));
          if (updated) await comments.save(id,updated);
        }
      } catch (error) { throw new Error(`Project was restored, but comment anchors could not be updated: ${error.message}${result.cleanupPending ? '. Temporary restore data remains in the project folder.' : ''}`); }
      return {project:await store.metadata(id),cleanupPending:result.cleanupPending};
    });
    writes=operation.then(()=>{},()=>{});return operation;
  });
  ipcMain.handle('versions:restore-file', (_, id, versionId, name) => {
    assertAssistantIdle(id);
    writes = writes.catch(() => {}).then(async () => {
      await waitForCompilation(id);
      await versions.snapshot(id);
      const content = await versions.readFile(id, versionId, name);
      await writeFile(id, name, content);
      await versions.snapshot(id);
    });
    return writes;
  });
  ipcMain.handle('chat:rename', (_, id, chatId, title) => {
    assertAssistantIdle(id);
    writes=writes.catch(()=>{}).then(async()=>{assertAssistantIdle(id);await store.metadata(id);return chats.rename(id,chatId,title);});return writes;
  });
  ipcMain.handle('chat:list', (_, id) => chats.list(id));
  ipcMain.handle('chat:create', (_, id) => {
    writes=writes.catch(()=>{}).then(async()=>{await store.metadata(id);return chats.create(id);});return writes;
  });
  ipcMain.handle('chat:attach', async (event, id, remaining=10) => {
    if(!Number.isInteger(remaining) || remaining<1 || remaining>10)throw new Error('Invalid attachment limit');
    assertAssistantIdle(id);
    const selected = await dialog.showOpenDialog(BrowserWindow.fromWebContents(event.sender), { properties: ['openFile', 'multiSelections'], filters: [{ name: 'Documents and images', extensions: ['png', 'jpg', 'jpeg', 'webp', 'pdf', 'txt', 'md', 'tex', 'bib', 'csv', 'docx'] }] });
    if(selected.canceled) return [];
    assertAssistantIdle(id);
    writes=writes.catch(()=>{}).then(async()=>{await store.metadata(id);return chats.attach(id,selected.filePaths,remaining);});return writes;
  });
  ipcMain.handle('chat:paste-image', async (_, id, bytes) => {
    writes=writes.catch(()=>{}).then(async()=>{await store.metadata(id);return chats.attachPastedImage(id,bytes);});
    return writes;
  });

  ipcMain.handle('chat:send', async (event, id, prompt, chatId, attachments, provider = 'codex') => {
    await projectTrust.assertTrusted(store.projectDir(id));
    if(!acceptingAssistantOperations) throw new Error('Application is shutting down');
    if(deletingProjects.has(id)) throw new Error('Project deletion is in progress');
    clearTimeout(snapshotTimers.get(id));snapshotTimers.delete(id);
    const metrics = new AssistantPerformance();
    try {return await assistant.run(id,async signal=>{
      const preferences = provider === 'codex' ? await codexPreferences.load() : await openCodePreferences.load();
      const project = await store.destinationDir(id);
      const bridge = await startAssistantTools(store, id, async (force=false) => {
        const options = await configuredBuild(store,id,texBin(),signal);
        const result = await metrics.measure('build',()=>buildCache.run(id,project,options,force));
        metrics.count(result.cached ? 'buildCacheHits' : 'builds');
        return {id:crypto.randomUUID(), cached:result.cached, engine:options.engine, rootFile:options.rootFile, cwd:project, outputDirectory:path.join(project,'.refract-build'), arguments:result.arguments, environment:'Inherited environment, configured external TeX bin prepended to PATH; BIBINPUTS includes the project recursively. XeLaTeX may use a generated FONTCONFIG_FILE.', pdfPath:result.ok ? path.join(project,'.refract-build','main.pdf') : null, ok:result.ok, exitCode:result.exitCode, log:result.log};
      }, name => lookupPackage(name,project,texBin(),signal),{performance:metrics,history:()=>chats.load(id,chatId)});
      try {return await chats.send(id,prompt,chatId,attachments,signal,status=>{
        if(!event.sender.isDestroyed())event.sender.send('chat:progress',{projectId:id,chatId,status});
      }, {model: preferences.model ?? undefined, reasoningEffort: preferences.reasoningEffort ?? undefined, ...(provider === 'codex' ? {fastMode: preferences.fastMode} : {}), tools:bridge.connection, performance:metrics}, provider,bridge.instructions);}
      finally {await bridge.close();}
    },metrics);} finally {
      if(assistantPerformance.size>=50)assistantPerformance.delete(assistantPerformance.keys().next().value);
      const report=metrics.report();assistantPerformance.set(id,report);
    }
  });
  ipcMain.handle('chat:cancel', (_,id)=>assistant.cancel(id));
  ipcMain.handle('projects:preview', async (_, id: string) => {
    if(!await projectTrust.isTrusted(store.projectDir(id)))return null;
    try { return (await fs.readFile(path.join(store.projectDir(id), '.refract-build', 'main.pdf'))).toString('base64'); } catch { return null; }
  });
  ipcMain.handle('compile', async (_, id, force=false) => {
    if(typeof force!=='boolean')throw new Error('Invalid compilation request');
    assertAssistantIdle(id);
    await writes;
    // An assistant may have started while a pending write was completing.
    assertAssistantIdle(id);
    if(shuttingDown) return {ok:false,log:'Compilation cancelled.'};
    const controller=new AbortController();compilationControllers.add(controller);
    const previous = compilations.get(id);
    const job = (previous ? previous.catch(() => {}) : Promise.resolve()).then(async () => {
      if(controller.signal.aborted) return {ok:false,log:'Compilation cancelled.'};
      return buildCache.run(id,store.projectDir(id), await configuredBuild(store,id,texBin(),controller.signal),force);
    });
    compilations.set(id,job);
    try { return await job; } finally { compilationControllers.delete(controller);if (compilations.get(id) === job) compilations.delete(id); }
  });
  const syncExecutable=()=>path.join(texBin(),'synctex.exe');
  ipcMain.handle('synctex:forward',async (_,id,file,line)=>{
    await writes;if(compilations.has(id))throw new Error('Compilation is in progress');
    await projectTrust.assertTrusted(store.projectDir(id));
    const project=await store.destinationDir(id);await store.existingFile(id,file);
    return synctex.forward(syncExecutable(),project,file,line);
  });
  ipcMain.handle('synctex:reverse',async (_,id,page,x,y)=>{
    await writes;if(compilations.has(id))throw new Error('Compilation is in progress');
    await projectTrust.assertTrusted(store.projectDir(id));
    const project=await store.destinationDir(id),result=await synctex.reverse(syncExecutable(),project,page,x,y);
    await store.existingFile(id,result.file);return result;
  });
  ipcMain.handle('pdf:save', async (event, encoded) => {
    if (typeof encoded !== 'string' || encoded.length > 140 * 1024 * 1024 || !/^[A-Za-z0-9+/]*={0,2}$/.test(encoded)) throw new Error('Invalid PDF data');
    const pdf = Buffer.from(encoded, 'base64');
    if (pdf.subarray(0, 5).toString() !== '%PDF-') throw new Error('Invalid PDF data');
    const result = await dialog.showSaveDialog(BrowserWindow.fromWebContents(event.sender), { defaultPath: 'main.pdf', filters: [{ name: 'PDF', extensions: ['pdf'] }] });
    if (!result.canceled && result.filePath) await fs.writeFile(result.filePath, pdf);
  });
  ipcMain.handle('files:write', (_, id, name, content) => {
    if(deletingProjects.has(id)) throw new Error('Project deletion is in progress');
    assertAssistantIdle(id);
    writes = writes.catch(() => {}).then(() => writeFile(id, name, content));
    const operation=writes;
    clearTimeout(snapshotTimers.get(id));
    snapshotTimers.delete(id);
    operation.then(()=>{
      failedSaves.get(id)?.delete(name);
      if(deletingProjects.has(id) || assistant.isBusy(id)) return;
      clearTimeout(snapshotTimers.get(id));
      snapshotTimers.set(id,setTimeout(()=>{snapshotTimers.delete(id);snapshot(id).catch(console.error);},3000));
    },()=>{let files=failedSaves.get(id);if(!files){files=new Set();failedSaves.set(id,files);}files.add(name);});
    return operation;
  });
  const window = new BrowserWindow({ titleBarStyle: 'hidden', titleBarOverlay: { color: nativeTheme.shouldUseDarkColors ? '#000000' : '#fafafa', symbolColor: nativeTheme.shouldUseDarkColors ? '#f1f3f7' : '#27272a', height: 28 }, icon: path.join(__dirname, 'logo.png'), show: !process.env.REFRACT_TEST_HIDDEN, width: 1440, height: 900, webPreferences: { preload: path.join(__dirname, 'preload.cjs'), contextIsolation: true, nodeIntegration: false, plugins: true } });
  nativeTheme.on('updated', () => { if (!window.isDestroyed()) window.setTitleBarOverlay({ color: nativeTheme.shouldUseDarkColors ? '#000000' : '#fafafa', symbolColor: nativeTheme.shouldUseDarkColors ? '#f1f3f7' : '#27272a' }); });
  window.removeMenu();
  window.webContents.on('before-input-event', (event: Electron.Event, input: Electron.Input) => {
    if (input.type !== 'keyDown' || !input.control || input.alt || input.meta) return;
    const direction = ['+', '=', 'Add'].includes(input.key) ? 1 : ['-', '_', 'Subtract'].includes(input.key) ? -1 : 0;
    if (!direction) return;
    event.preventDefault();
    const level = window.webContents.getZoomLevel();
    window.webContents.setZoomLevel(Math.max(-3, Math.min(3, level + direction)));
  });
  window.webContents.setWindowOpenHandler(()=>({action:'deny'}));
  window.webContents.on('will-navigate',event=>event.preventDefault());
  mainWindow=window;
  window.on('close',event=>{if(!allowQuit){event.preventDefault();beginShutdown();}});
  await window.loadFile(path.join(__dirname, 'index.html'));
}).catch(error => { console.error(error); app.exit(1); });
app.on('before-quit',event=>{if(!allowQuit){event.preventDefault();beginShutdown();}});
app.on('window-all-closed',()=>{if(!allowQuit)beginShutdown();});


