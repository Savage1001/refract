const { contextBridge, ipcRenderer } = require('electron');
type ModelDiscoveryResult = import('./src/codex-models.cjs').ModelDiscoveryResult;
ipcRenderer.on('app:prepare-shutdown', () => {
  if(document.documentElement) document.documentElement.inert = true;
  ipcRenderer.send('app:shutdown-ready');
});
ipcRenderer.on('app:shutdown-cancelled', () => {
  if(document.documentElement) document.documentElement.inert = false;
});
contextBridge.exposeInMainWorld('store', {
  updateRestart: (): Promise<void> => ipcRenderer.invoke('updates:restart'),
  updateCheck: (): Promise<import('./src/updates.cjs').UpdateStatus> => ipcRenderer.invoke('updates:check'),
  updateStatus: (): Promise<import('./src/updates.cjs').UpdateStatus> => ipcRenderer.invoke('updates:status'),
  updateTrack: (track: import('./src/updates.cjs').UpdateTrack): Promise<import('./src/updates.cjs').UpdateStatus> => ipcRenderer.invoke('updates:track', track),
  discoverProviders: (): Promise<{codex:boolean;opencode:boolean;codexSetupRequired?:boolean;codexSignInRequired?:boolean}> => ipcRenderer.invoke('providers:discover'),
  discoverOpenCodeModels: (projectId?: string): Promise<{ok:true;models:import('./src/opencode.cjs').OpenCodeModel[]}|{ok:false;error:string}> => ipcRenderer.invoke('opencode:discover-models', projectId),
  openCodePreferencesLoad: (): Promise<import('./src/codex-preferences.cjs').CodexPreferences> => ipcRenderer.invoke('opencode-preferences:load'),
  openCodePreferencesSave: (value: import('./src/codex-preferences.cjs').CodexPreferences): Promise<void> => ipcRenderer.invoke('opencode-preferences:save', value),
  codexAccountStatus: (): Promise<import('./src/codex-account.cjs').CodexAccountStatus> => ipcRenderer.invoke('codex-account:status'),
  codexAccountSelect: (mode: import('./src/codex-account.cjs').CodexMode): Promise<void> => ipcRenderer.invoke('codex-account:select', mode),
  codexAccountSignIn: (register: boolean): Promise<void> => ipcRenderer.invoke('codex-account:sign-in', register),
  codexAccountCancel: (): Promise<void> => ipcRenderer.invoke('codex-account:cancel'),
  codexAccountSignOut: (): Promise<void> => ipcRenderer.invoke('codex-account:sign-out'),
  discoverCodexModels: (mode?: import('./src/codex-account.cjs').CodexMode): Promise<ModelDiscoveryResult> => ipcRenderer.invoke('codex:discover-models', mode),
  openExternal: url => ipcRenderer.invoke('external:open', url),
  projectPreview: (id: string): Promise<string | null> => ipcRenderer.invoke('projects:preview', id),
  list: () => ipcRenderer.invoke('projects:list'),
  create: name => ipcRenderer.invoke('projects:create', name),
  trashProject: id => ipcRenderer.invoke('projects:trash', id),
  deletedProjects: () => ipcRenderer.invoke('projects:trash-list'),
  restoreProject: token => ipcRenderer.invoke('projects:trash-restore', token),
  duplicate: id => ipcRenderer.invoke('projects:duplicate', id),
  rename: (id, name) => ipcRenderer.invoke('projects:rename', id, name),
  setDraftMode: (id: string, enabled: boolean) => ipcRenderer.invoke('projects:draft', id, enabled),
  setEngine: (id, engine) => ipcRenderer.invoke('projects:engine', id, engine),
  onShutdownNotice: (callback: (title: string, message: string, action: string) => void) => {
    ipcRenderer.on('app:shutdown-notice', (_event, title, message, action) => callback(title, message, action));
  },
  onImportRoot: (select: (choices: string[]) => Promise<string | string[] | null>) => {
    ipcRenderer.on('projects:import-root', async (event: Electron.IpcRendererEvent, choices: string[]) => {
      const [port] = event.ports;
      try {
        port.postMessage(await select(choices));
      } catch {
        port.postMessage(null);
      } finally {
        port.close();
      }
    });
  },
  importProject: () => ipcRenderer.invoke('projects:import'),
  exportProject: id => ipcRenderer.invoke('projects:export', id),
  read: (id, name) => ipcRenderer.invoke('files:read', id, name),
  openFile: (id, name) => ipcRenderer.invoke('files:open', id, name),
  image: (id, name) => ipcRenderer.invoke('files:image', id, name),
  files: id => ipcRenderer.invoke('files:list', id),
  tree: id => ipcRenderer.invoke('files:tree', id),
  search: (id, query) => ipcRenderer.invoke('files:search', id, query),
  createFile: (id, name) => ipcRenderer.invoke('files:create', id, name),
  renameFile: (id, oldName, newName) => ipcRenderer.invoke('files:rename', id, oldName, newName),
  trashFile: (id, name) => ipcRenderer.invoke('files:trash', id, name),
  trashFolder: (id, name) => ipcRenderer.invoke('files:trash-folder', id, name),
  trashList: id => ipcRenderer.invoke('files:trash-list', id),
  restoreTrash: (id, token) => ipcRenderer.invoke('files:trash-restore', id, token),
  downloadFile: (id, name) => ipcRenderer.invoke('files:download', id, name),
  createFolder: (id, name) => ipcRenderer.invoke('files:folder', id, name),
  moveFolder: (id, oldName, newName) => ipcRenderer.invoke('files:move-folder', id, oldName, newName),
  importFiles: (id, destination) => ipcRenderer.invoke('files:import', id, destination),
  importFolder: (id, destination) => ipcRenderer.invoke('files:import-folder', id, destination),
  codexPreferencesLoad: (mode?: import('./src/codex-account.cjs').CodexMode): Promise<import('./src/codex-preferences.cjs').CodexPreferences> => ipcRenderer.invoke('codex-preferences:load', mode),
  codexPreferencesSave: (value: import('./src/codex-preferences.cjs').CodexPreferences, mode?: import('./src/codex-account.cjs').CodexMode): Promise<void> => ipcRenderer.invoke('codex-preferences:save', value, mode),
  chatImagePreview: (id, token) => ipcRenderer.invoke('chat:image-preview', id, token),
  chatLoad: (id, chatId) => ipcRenderer.invoke('chat:load', id, chatId),
  commentList: id => ipcRenderer.invoke('comments:list', id),
  commentAdd: (id, file, quote, text, selection) => ipcRenderer.invoke('comments:add', id, file, quote, text, selection),
  commentLocate: (id, commentId) => ipcRenderer.invoke('comments:locate', id, commentId),
  commentReply: (id, commentId, text) => ipcRenderer.invoke('comments:reply', id, commentId, text),
  commentResolve: (id, commentId, resolved) => ipcRenderer.invoke('comments:resolve', id, commentId, resolved),
  versionList: id => ipcRenderer.invoke('versions:list', id),
  versionRetainedStages: id => ipcRenderer.invoke('versions:retained-stages', id),
  versionSnapshot: id => ipcRenderer.invoke('versions:snapshot', id),
  versionCompare: (id, versionId) => ipcRenderer.invoke('versions:compare', id, versionId),
  versionRestoreProject: (id, versionId) => ipcRenderer.invoke('versions:restore-project', id, versionId),
  versionRestoreFile: (id, versionId, name) => ipcRenderer.invoke('versions:restore-file', id, versionId, name),
  chatRename: (id, chatId, title) => ipcRenderer.invoke('chat:rename', id, chatId, title),
  chatList: id => ipcRenderer.invoke('chat:list', id),
  chatCreate: id => ipcRenderer.invoke('chat:create', id),
  chatPasteImage: (id,bytes) => ipcRenderer.invoke('chat:paste-image', id, bytes),
  chatAttach: (id,remaining) => ipcRenderer.invoke('chat:attach', id,remaining),
  chatSend: (id, prompt, chatId, attachments, provider, selection?: Omit<import('./src/codex-preferences.cjs').CodexPreferences, 'favorites'>, mode?: string) => ipcRenderer.invoke('chat:send', id, prompt, chatId, attachments, provider, selection, mode),
  chatCancel: id => ipcRenderer.invoke('chat:cancel', id),
  onChatProgress: callback => {ipcRenderer.on('chat:progress', (_event,update) => callback(update));},
  onProjectEngineChanged: (callback: (update: {projectId: string; engine: string}) => void) => {
    ipcRenderer.on('projects:engine-changed', (_event, update) => callback(update));
  },
  compile: (id,force=false) => ipcRenderer.invoke('compile', id,force),
  assistantPerformance: id => ipcRenderer.invoke('assistant:performance',id),
  syncForward: (id,file,line) => ipcRenderer.invoke('synctex:forward',id,file,line),
  syncReverse: (id,page,x,y) => ipcRenderer.invoke('synctex:reverse',id,page,x,y),
  savePdf: pdf => ipcRenderer.invoke('pdf:save', pdf),
  write: (id, name, content) => ipcRenderer.invoke('files:write', id, name, content)
});

