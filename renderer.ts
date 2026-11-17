// All action glyphs use the same view box, stroke and fixed geometry.
type ActionIcon = 'search' | 'plus' | 'close' | 'menu' | 'chevron' | 'previous' | 'next' | 'minus' | 'trash' | 'copy' | 'agent' | 'editor' | 'preview' | 'download' | 'export' | 'wrench';
function actionIcon(name:ActionIcon):SVGSVGElement {
  const paths:Record<ActionIcon,string> = {wrench:'M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.8-3.8a6 6 0 0 1-7.9 7.9l-7 7a2.1 2.1 0 0 1-3-3l7-7a6 6 0 0 1 7.9-7.9z',download:'M12 3v12m-5-5 5 5 5-5M5 16v4h14v-4',export:'M14 3h7v7m0-7L10 14M10 5H5a2 2 0 0 0-2 2v12a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2v-5',agent:'M15 14H7l-4 3V6a2 2 0 0 1 2-2h10a2 2 0 0 1 2 2v6a2 2 0 0 1-2 2Z M10 14v2a2 2 0 0 0 2 2h5l4 3v-8a2 2 0 0 0-2-2h-2',editor:'m7 8-4 4 4 4m10-8 4 4-4 4M14 5l-4 14',preview:'M14 2H5v20h14V7Z M14 2v5h5M8 11h8M8 15h8M8 18h5',search:'M21 21l-5-5M17 10a7 7 0 1 1-14 0 7 7 0 0 1 14 0',plus:'M12 5v14M5 12h14',close:'M6 6l12 12M18 6 6 18',menu:'M4 6h16M4 12h16M4 18h16',chevron:'m6 9 6 6 6-6',previous:'m15 6-6 6 6 6',next:'m9 6 6 6-6 6',minus:'M5 12h14',trash:'M3 6h18M9 6V3h6v3M5 6l1 15h12l1-15M10 10v7M14 10v7',copy:'M9 9h12v12H9zM5 15H3V3h12v2'};
  const svg=document.createElementNS('http://www.w3.org/2000/svg','svg');
  for(const [key,value] of Object.entries({viewBox:'0 0 24 24',fill:'none',stroke:'currentColor','stroke-width':'1.8','stroke-linecap':'round','stroke-linejoin':'round','aria-hidden':'true',focusable:'false',class:'ui-icon'})) svg.setAttribute(key,value);
  const path=document.createElementNS(svg.namespaceURI,'path');path.setAttribute('d',paths[name]);svg.append(path);return svg;
}
declare const store: any;
let projects = [], active = null, currentFile = 'main.tex', opening = 0, projectOpening = 0, currentChat = 'default';
let revision = 0, compileTimer = null, compileRunning = false, compileQueued = false;
let restoringProject: string | null = null;
const chatBusy = new Set();
const chatProgress = new Map<string,{chatId:string;status:string}>();
const pendingAttachments = new Set<string>();
let attachments = [], welcomeDismissed = false, commentSelection = null, openFiles = [];
type EditorView={start:number;end:number;direction:'forward'|'backward'|'none';top:number;left:number};
const editorViews=new Map<string,EditorView>();
function editorViewKey(id:string,file:string) {return JSON.stringify([id,file]);}
function saveEditorView() {
  if(!active || !currentFile || $('.source-container').hidden)return;
  const editor=$('#source') as HTMLTextAreaElement;
  editorViews.set(editorViewKey(active.id,currentFile),{start:editor.selectionStart,end:editor.selectionEnd,direction:editor.selectionDirection,top:editor.scrollTop,left:editor.scrollLeft});
}
let chatOpening=0, chatLoading=false;
let providerDiscovery=0, providerAvailable=false;
type ChatDraft={text:string;attachments:{token:string;name:string}[]};
const chatDrafts=new Map<string,ChatDraft>();
function draftKey(id:string,chatId:string) {return `${id}:${chatId}`;}
function writeChatDraft(key:string,draft:ChatDraft) {
  chatDrafts.set(key,draft);
  try {if(draft.text || draft.attachments.length)localStorage.setItem('refract-chat-draft:'+key,JSON.stringify(draft));else localStorage.removeItem('refract-chat-draft:'+key);} catch(error) {console.error('Could not persist chat draft',error);}
}
function readChatDraft(key:string):ChatDraft {
  const cached=chatDrafts.get(key);if(cached)return cached;
  try {const data=JSON.parse(localStorage.getItem('refract-chat-draft:'+key) || 'null');if(data && typeof data.text==='string' && Array.isArray(data.attachments))return {text:data.text,attachments:data.attachments.filter(item=>item && typeof item.token==='string' && typeof item.name==='string')};} catch {}
  return {text:'',attachments:[]};
}
function saveChatDraft() {if(active)writeChatDraft(draftKey(active.id,currentChat),{text:$('#chat-input').value,attachments:[...attachments]});}
function restoreChatDraft(id:string,chatId:string) {const draft=readChatDraft(draftKey(id,chatId));$('#chat-input').value=draft.text;attachments=[...draft.attachments];renderAttachments();}
const $ = (s: string): any => document.querySelector(s);
function updateSendState() {$('#send').disabled=!active || !providerAvailable || chatLoading || chatBusy.has(active.id) || restoringProject===active.id || pendingAttachments.has(draftKey(active.id,currentChat));}
function setEditorReadOnly(value:boolean) {$('#source').readOnly=value;completionButton.disabled=value || $('.source-container').hidden;}
function report(action, error) { alert(`Could not ${action}: ${error.message}`); }
function showRestoreNotice(visible:boolean) {
  let notice=$('#restore-notice');
  if(!notice) {notice=document.createElement('div');notice.id='restore-notice';notice.setAttribute('role','status');notice.textContent='Temporary restore data remains in this project. It has been preserved because it could not be safely cleaned.';$('#workspace').prepend(notice);}
  notice.hidden=!visible;
}
type ProjectThumbnail = { fingerprint: string; image: string };
const thumbnailStorageKey = 'refract-project-thumbnails-v1';
const projectThumbnails = new Map<string, ProjectThumbnail>();
const pendingThumbnails = new Map<string, Promise<void>>();
try {
  const saved: unknown = JSON.parse(localStorage.getItem(thumbnailStorageKey) || '[]');
  if (Array.isArray(saved)) for (const entry of saved) {
    if (Array.isArray(entry) && typeof entry[0] === 'string' && typeof entry[1]?.fingerprint === 'string' && typeof entry[1]?.image === 'string' && entry[1].image.startsWith('data:image/png;base64,')) projectThumbnails.set(entry[0], entry[1]);
  }
} catch { /* A missing or damaged cache can be rebuilt. */ }
function persistProjectThumbnails(): void {
  // Bound storage so thumbnails do not crowd out drafts and other preferences.
  while (projectThumbnails.size > 60 || JSON.stringify([...projectThumbnails]).length > 2_000_000) {
    const oldest = projectThumbnails.keys().next().value;
    if (oldest === undefined) break;
    projectThumbnails.delete(oldest);
  }
  try { localStorage.setItem(thumbnailStorageKey, JSON.stringify([...projectThumbnails])); } catch { /* Keep the in-memory cache if storage is full. */ }
}
function showProjectThumbnail(id: string, button: HTMLButtonElement): void {
  const cached = projectThumbnails.get(id);
  const existing = button.querySelector<HTMLImageElement>('.project-page-thumbnail');
  if (cached && existing?.getAttribute('src') === cached.image) return;
  existing?.remove();
  button.classList.toggle('has-page-thumbnail', !!cached);
  if (!cached) return;
  const image = document.createElement('img'); image.className = 'project-page-thumbnail'; image.alt = ''; image.src = cached.image;
  button.prepend(image);
}
async function refreshProjectThumbnail(id: string): Promise<void> {
  const encoded: string | null = await store.projectPreview(id);
  if (!encoded) { projectThumbnails.delete(id); persistProjectThumbnails(); return; }
  const bytes = Uint8Array.from(atob(encoded), c => c.charCodeAt(0));
  const digest = await crypto.subtle.digest('SHA-256', bytes);
  const fingerprint = Array.from(new Uint8Array(digest), byte => byte.toString(16).padStart(2, '0')).join('');
  if (projectThumbnails.get(id)?.fingerprint === fingerprint) return;
  const modulePath: string = '../node_modules/pdfjs-dist/build/pdf.mjs';
  const pdfjs = await import(modulePath);
  pdfjs.GlobalWorkerOptions.workerSrc = '../node_modules/pdfjs-dist/build/pdf.worker.mjs';
  const doc = await pdfjs.getDocument({data: bytes, isEvalSupported: false}).promise;
  try {
    const page = await doc.getPage(1), original = page.getViewport({scale: 1});
    const viewport = page.getViewport({scale: 180 / original.width});
    const canvas = document.createElement('canvas'); canvas.width = Math.ceil(viewport.width); canvas.height = Math.ceil(viewport.height);
    await page.render({canvasContext: canvas.getContext('2d')!, viewport}).promise;
    projectThumbnails.delete(id);
    projectThumbnails.set(id, {fingerprint, image: canvas.toDataURL()});
    persistProjectThumbnails();
  } finally { await doc.destroy(); }
}
async function loadProjectThumbnail(id: string, button: HTMLButtonElement): Promise<void> {
  let pending = pendingThumbnails.get(id);
  if (!pending) {
    pending = refreshProjectThumbnail(id).catch(() => { /* Keep the last good preview on read/render failure. */ }).finally(() => { pendingThumbnails.delete(id); });
    pendingThumbnails.set(id, pending);
  }
  await pending;
  if (button.isConnected) showProjectThumbnail(id, button);
}
function renderProjects() {
  const matches = projects.filter(p => p.name.toLowerCase().includes($('#search').value.toLowerCase()));
  const sort=$('#project-sort').value;
  matches.sort((a,b)=>sort==='name' ? a.name.localeCompare(b.name) || String(b.created).localeCompare(String(a.created)) : (sort==='oldest' ? 1 : -1)*String(sort==='edited' ? a.edited || a.created : a.created).localeCompare(String(sort==='edited' ? b.edited || b.created : b.created)) || a.name.localeCompare(b.name));
  $('#empty').hidden = projects.length > 0;
  $('#no-results').hidden = !projects.length || matches.length > 0;
  $('#project-count').hidden = !matches.length; $('#project-count').textContent = `${matches.length} ${matches.length === 1 ? 'project' : 'projects'}`;
  $('.table-head').hidden = !matches.length || $('#list').classList.contains('grid');
  $('#list').replaceChildren(...matches.map(p => {
    const row = document.createElement('article'), name = document.createElement('button'), date = document.createElement('span'), remove=document.createElement('button');
    const projectLabel = document.createElement('span'); projectLabel.className='project-label'; projectLabel.textContent=p.name;
    const openHint = document.createElement('span'); openHint.className='project-open-hint'; openHint.textContent='Open project →'; openHint.setAttribute('aria-hidden','true');
    name.className='project-open'; name.title=p.name; name.append(projectLabel,openHint); name.setAttribute('aria-label',`Open ${p.name}`); name.onclick=event=>{event.stopPropagation(); if(!remove.disabled) openProject(p);}; date.className='project-date';date.textContent = new Date(p.created).toLocaleDateString();
    remove.textContent='Delete';remove.className='project-delete';remove.setAttribute('aria-label',`Delete ${p.name}`);
    remove.onclick=async event=>{event.stopPropagation();if(!confirm(`Move ${p.name} to Deleted projects?`))return;remove.disabled=true;try{await store.trashProject(p.id);projects=await store.list();renderProjects();}catch(error){report('delete project',error);remove.disabled=false;}};
    const actions=document.createElement('details'); actions.className='project-actions';
    const actionLabel=document.createElement('summary'); actionLabel.textContent='Actions'; actionLabel.setAttribute('aria-label',`Actions for ${p.name}`);
    actions.addEventListener('click',event=>event.stopPropagation());
    actions.addEventListener('keydown',event=>{if(event.key==='Escape' && actions.open){event.preventDefault();event.stopPropagation();actions.open=false;actionLabel.focus();}});
    actions.addEventListener('toggle',()=>{if(actions.open) document.querySelectorAll<HTMLDetailsElement>('.project-actions[open]').forEach(other=>{if(other!==actions) other.open=false;});});
    showProjectThumbnail(p.id, name);
    actions.append(actionLabel,remove); row.append(name, date, actions); row.onclick = () => {if(!remove.disabled) openProject(p);}; queueMicrotask(() => { void loadProjectThumbnail(p.id, name); }); return row;
  }));
}
document.addEventListener('click',event=>{if (!(event.target as Element).closest('.project-actions')) document.querySelectorAll<HTMLDetailsElement>('.project-actions[open]').forEach(menu=>{menu.open=false;});});
try {const saved=localStorage.getItem('refract-project-sort');if(['newest','oldest','name','edited'].includes(saved)) $('#project-sort').value=saved;} catch {}
$('#project-sort').onchange=()=>{try{localStorage.setItem('refract-project-sort',$('#project-sort').value);}catch{} renderProjects();};
async function renderDeletedProjects() {
  const items=await store.deletedProjects(), list=$('#project-trash-list');list.replaceChildren();
  if(!items.length){const title=document.createElement('p'), hint=document.createElement('p'); title.className='recovery-empty-title'; title.textContent='No deleted projects'; hint.className='recovery-empty-hint'; hint.textContent='Projects you delete appear here for up to 30 days.'; list.append(title,hint); return;}
  for(const item of items) {
    const row=document.createElement('div'), label=document.createElement('span'), date=document.createElement('small'), button=document.createElement('button');
    row.className='trash-row';label.textContent=item.name;const deleted=Date.parse(item.deleted);
    date.textContent=Number.isFinite(deleted) ? `Deleted ${new Date(deleted).toLocaleString()}` : 'Deletion date unavailable'; const retention=document.createElement('small'); retention.className='recovery-retention'; if(Number.isFinite(deleted)) retention.textContent=`${Math.max(0,Math.ceil((deleted+30*24*60*60*1000-Date.now())/(24*60*60*1000)))} days remaining`; button.textContent='Restore';button.setAttribute('aria-label',`Restore ${item.name}`);
    button.onclick=async()=>{button.disabled=true;try{await store.restoreProject(item.id);projects=await store.list();renderProjects();await renderDeletedProjects();}catch(error){report('restore project',error);button.disabled=false;}};
    const description=document.createElement('div'); description.className='trash-description'; description.append(label,date,retention); row.append(description,button);list.append(row);
  }
}
$('#deleted-projects').onclick=async()=>{try{await renderDeletedProjects();$('#project-trash-dialog').showModal();}catch(error){report('load deleted projects',error);}};
$('#project-trash-close').onclick=()=>$('#project-trash-dialog').close();
let contextMenuOrigin: HTMLElement | null = null;
function dismissFileMenus(restoreFocus = false): void {
  const menus = Array.from(document.querySelectorAll<HTMLElement>('#file-menu, #folder-menu'));
  const hadFocus = menus.some(menu => menu.contains(document.activeElement));
  for (const menu of menus) menu.hidden = true;
  if (restoreFocus && hadFocus) (contextMenuOrigin?.isConnected ? contextMenuOrigin : document.getElementById('source'))?.focus();
}
function showFileContextMenu(menu: HTMLElement, event: MouseEvent | KeyboardEvent, origin = event.currentTarget as HTMLElement): void {
  event.preventDefault(); dismissFileMenus(); contextMenuOrigin = origin;
  menu.hidden = false;
  const anchor = origin.getBoundingClientRect();
  const pointer = event instanceof MouseEvent && (event.clientX || event.clientY);
  const x = pointer ? event.clientX : anchor.left, y = pointer ? event.clientY : anchor.bottom;
  const bounds = menu.getBoundingClientRect();
  menu.style.left = `${Math.max(8, Math.min(x, window.innerWidth - bounds.width - 8))}px`;
  menu.style.top = `${Math.max(8, Math.min(y, window.innerHeight - bounds.height - 8))}px`;
  menu.querySelector<HTMLButtonElement>('button:not(:disabled)')?.focus();
}
document.getElementById('file-list')!.addEventListener('keydown', (event: KeyboardEvent) => {
  if (event.key !== 'ContextMenu' && !(event.shiftKey && event.key === 'F10')) return;
  const origin = (event.target as Element).closest<HTMLElement>('button[data-path], summary');
  if (!origin) return;
  const folder = origin.tagName === 'SUMMARY';
  if (folder) menuFolder = origin.title; else menuFile = origin.dataset.path;
  showFileContextMenu(document.getElementById(folder ? 'folder-menu' : 'file-menu')!, event, origin);
});
for (const menu of document.querySelectorAll<HTMLElement>('#file-menu, #folder-menu')) {
  menu.addEventListener('keydown', event => {
    if (event.key === 'Escape') { event.preventDefault(); dismissFileMenus(true); }
    else if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      event.preventDefault(); const buttons = Array.from(menu.querySelectorAll<HTMLButtonElement>('button:not(:disabled)'));
      const index = buttons.indexOf(document.activeElement as HTMLButtonElement);
      buttons[(index + (event.key === 'ArrowDown' ? 1 : -1) + buttons.length) % buttons.length]?.focus();
    }
  });
  menu.addEventListener('click', () => dismissFileMenus(true));
}
document.addEventListener('focusin', event => {
  if (!(event.target as Element).closest('#file-menu, #folder-menu')) dismissFileMenus();
});
let fileTreeRequest = 0, renderedTree = '', renderedTreeProject = '';
async function refreshFiles() {
  if (!active) return;
  const id = active.id, request = ++fileTreeRequest, nodes = await store.tree(id);
  if (active?.id !== id || request !== fileTreeRequest) return;
  const signature = JSON.stringify(nodes);
  if (renderedTreeProject === id && renderedTree === signature && document.querySelector('#file-list button.active-file')?.getAttribute('data-path') === currentFile) return;
  renderedTreeProject = id; renderedTree = signature;
  const folderStates = new Map(Array.from(document.querySelectorAll<HTMLDetailsElement>('#file-list details[data-path]'), folder => [folder.dataset.path, folder.open]));
  function renderNodes(items) {
    const list = document.createElement('div'); list.className = 'file-tree';
    for (const item of items) {
      if (item.children) {
        const folder = document.createElement('details'); folder.dataset.path = item.path; folder.open = folderStates.get(item.path) ?? true;
        const label = document.createElement('summary'), folderIcon = document.createElementNS('http://www.w3.org/2000/svg','svg'), folderPath = document.createElementNS('http://www.w3.org/2000/svg','path'), folderName = document.createElement('span');
        folderIcon.setAttribute('viewBox','0 0 24 24'); folderIcon.setAttribute('aria-hidden','true'); folderIcon.setAttribute('class','folder-icon'); folderPath.setAttribute('d','M3 6.5h6l2 2h10v9a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z'); folderIcon.append(folderPath); folderName.textContent=item.name; const disclosure=actionIcon('next');disclosure.classList.add('folder-chevron');label.append(disclosure,folderIcon,folderName); label.title = item.path;
        label.oncontextmenu = event => { menuFolder=item.path; $('#file-menu').hidden=true; showFileContextMenu($('#folder-menu'), event); };
        folder.append(label, renderNodes(item.children)); list.append(folder);
      } else {
        const button = document.createElement('button'), icon = document.createElement('span'), name = document.createElement('span');
        icon.className = 'tree-file-icon'; icon.setAttribute('aria-hidden', 'true'); icon.textContent = '\u25a7  '; icon.dataset.type = (item.name.split('.').pop() || 'file').slice(0,4).toUpperCase(); name.className = 'tree-file-name'; name.textContent = item.name; button.append(icon, name);
        if (/\.(png|jpe?g|gif|webp|svg)$/i.test(item.name)) {
          const thumbnail = document.createElement('img');
          thumbnail.className = 'tree-image-thumbnail'; thumbnail.alt = ''; thumbnail.hidden = true;
          thumbnail.onload = () => { thumbnail.hidden = false; icon.hidden = true; };
          thumbnail.onerror = () => { thumbnail.hidden = true; icon.hidden = false; };
          button.insertBefore(thumbnail, name);
          void store.openFile(id, item.path).then(file => {
            if (file.kind === 'image' && active?.id === id) thumbnail.src = file.data;
          }).catch(() => {});
        }
        button.title = item.path; button.setAttribute('aria-label', `Open ${item.path}`); button.dataset.path = item.path; button.classList.toggle('active-file', item.path === currentFile); if (item.path === currentFile) button.setAttribute('aria-current', 'page'); button.onclick = () => openFile(item.path);
        button.oncontextmenu = event => { menuFile = item.path; $('#folder-menu').hidden=true; showFileContextMenu($('#file-menu'), event); }; list.append(button);
      }
    }
    return list;
  }
  $('#file-list').replaceChildren(renderNodes(nodes));
}
setInterval(() => { if (active) void refreshFiles().catch(error => console.error('Could not refresh project files', error)); }, 3000);
let previewDocument: any = null, previewLoading: any = null, previewGeneration = 0;
function clearFilePreview() {
  previewGeneration++;
  const resource = previewDocument || previewLoading;
  previewDocument = null; previewLoading = null;
  if (resource) resource.destroy().catch(() => {});
  $('#file-preview').replaceChildren(); $('#file-preview').removeAttribute('role');$('#file-preview').hidden = true;
  $('#image-preview').hidden = true; $('#project-image').onerror=null;$('#project-image').removeAttribute('src');
}
async function showProjectPdf(encoded: string, request: number, id: string, name: string) {
  const panel = $('#file-preview'), loading = document.createElement('p');
  loading.textContent = 'Loading PDF…'; panel.replaceChildren(loading); panel.hidden = false;
  try {
    const bytes = Uint8Array.from(atob(encoded), c => c.charCodeAt(0));
    const pdfModulePath: string = '../node_modules/pdfjs-dist/build/pdf.mjs';
    const pdfjs = await import(pdfModulePath);
    if (request !== opening || active?.id !== id) return;
    pdfjs.GlobalWorkerOptions.workerSrc = '../node_modules/pdfjs-dist/build/pdf.worker.mjs';
    const task = pdfjs.getDocument({data:bytes,isEvalSupported:false}); previewLoading = task;
    const doc = await task.promise;
    if (request !== opening || active?.id !== id) { await doc.destroy(); return; }
    previewLoading = null; previewDocument = doc;
    if (!doc.numPages) throw new Error('The PDF has no pages.');
    const controls = document.createElement('div'); controls.className = 'file-pdf-controls';
    const previous = document.createElement('button'), next = document.createElement('button'), count = document.createElement('span');
    previous.textContent = 'Previous'; previous.setAttribute('aria-label','Previous PDF page');
    next.textContent = 'Next'; next.setAttribute('aria-label','Next PDF page');
    const pageArea = document.createElement('div'); pageArea.className = 'file-pdf-page';
    controls.append(previous,count,next); panel.replaceChildren(controls,pageArea);
    let pageNumber = 1;
    const renderPage = async number => {
      const generation = ++previewGeneration;
      previous.disabled = true; next.disabled = true; count.textContent = `Page ${number} of ${doc.numPages}`;
      pageArea.textContent = `Loading page ${number}…`;
      try {
        const page = await doc.getPage(number), natural = page.getViewport({scale:1});
        if (!Number.isFinite(natural.width) || !Number.isFinite(natural.height) || natural.width <= 0 || natural.height <= 0) throw new Error('Invalid PDF page size');
        const scale = Math.min(1.5,4096 / natural.width,4096 / natural.height,Math.sqrt(12000000 / (natural.width * natural.height)));
        const viewport = page.getViewport({scale}), canvas = document.createElement('canvas');
        canvas.width = Math.ceil(viewport.width); canvas.height = Math.ceil(viewport.height);
        canvas.setAttribute('aria-label',`Page ${number} of ${name}`);
        await page.render({canvasContext:canvas.getContext('2d'),viewport}).promise;
        if (generation !== previewGeneration || request !== opening || active?.id !== id) return;
        pageNumber = number; pageArea.replaceChildren(canvas);
        previous.disabled = number <= 1; next.disabled = number >= doc.numPages;
      } catch (error) {
        if (generation !== previewGeneration || request !== opening || active?.id !== id) return;
        pageArea.textContent = `Could not render this page: ${error.message}`;
        count.textContent = `Page ${pageNumber} of ${doc.numPages}`;
        previous.disabled = pageNumber <= 1; next.disabled = pageNumber >= doc.numPages;
      }
    };
    previous.onclick = () => renderPage(pageNumber-1); next.onclick = () => renderPage(pageNumber+1);
    await renderPage(1);
  } catch (error) {
    if (request === opening && active?.id === id) panel.textContent = `Could not preview PDF: ${error.message}`;
  }
}
async function openFile(name) {
  const request = ++opening, id = active.id, version = revision;
  try {
    const file = await store.openFile(id,name);
    if (request !== opening || active?.id !== id || revision !== version) return;
    saveEditorView();
    closeCompletions();
    clearFilePreview();
    commentSelection = null; currentFile = name; document.querySelectorAll<HTMLElement>('#file-list button[data-path]').forEach(button => { const selected = button.dataset.path === name; button.classList.toggle('active-file', selected); if (selected) button.setAttribute('aria-current','page'); else button.removeAttribute('aria-current'); }); if (!openFiles.includes(name)) openFiles.push(name); renderFileTabs();
    $('#numbers').hidden = file.kind !== 'text'; $('.source-container').hidden = file.kind !== 'text';
    completionButton.disabled=file.kind!=='text' || ($('#source') as HTMLTextAreaElement).readOnly;
    updatePdfControls();
    if(file.kind!=='text') findPanel.hidden=true;
    if (file.kind === 'text') {
      const editor=$('#source') as HTMLTextAreaElement,view=editorViews.get(editorViewKey(id,name));
      editor.value=file.text;updateEditor();editor.setSelectionRange(view?.start || 0,view?.end || 0,view?.direction || 'none');
      editor.scrollTop=view?.top || 0;editor.scrollLeft=view?.left || 0;
      if(!findPanel.hidden) {findAnchor=editor.selectionStart;refreshFind();}
      syncEditorScroll();
    }
    else if (file.kind === 'image') {
      const image=$('#project-image') as HTMLImageElement;
      image.onerror=()=>{if(request!==opening || active?.id!==id || currentFile!==name)return;$('#image-preview').hidden=true;const panel=$('#file-preview');panel.textContent='Could not preview this image.';panel.setAttribute('role','status');panel.hidden=false;};
      image.alt = name; $('#image-preview').hidden = false;image.src = file.data;
    } else if (file.kind === 'pdf') await showProjectPdf(file.data,request,id,name);
    else { $('#file-preview').textContent = file.reason; $('#file-preview').hidden = false; }
  } catch (error) { if (request === opening && active?.id === id) report('open file', error); }
}
async function openProject(p) {
  const request = ++projectOpening;
  saveEditorView();saveChatDraft();const chatRequest=++chatOpening;chatLoading=true;
  ++opening;
  ++searchRequest; clearTimeout(searchTimer); $('#file-search').hidden = !$('#files-tab').classList.contains('active'); $('#file-search-input').value = ''; clearFileSearch.hidden = true; $('#file-search-results').replaceChildren();
  attachments = []; openFiles = []; clearEditor(); currentChat = 'default'; active = p; restoreChatDraft(p.id,currentChat); $('#chat-messages').replaceChildren(); revision++; setEditorReadOnly(chatBusy.has(p.id) || restoringProject === p.id); $('#send').disabled=true; updateCancelButton();updateChatProgress(); $('#projects').hidden = true; $('#workspace').hidden = false; workspaceBar.hidden = false;
  compiledProject = null; compiledPdf = null; $('#download').disabled = true; compileLog = ''; showingTools = false; $('#tools-panel').hidden = true; $('#pdf-content').hidden = false; toolsButton.setAttribute('aria-expanded', 'false'); $('#compile').textContent = 'Compile';
  pdfDocument=null;lastPdfSync=null;pdfRenderRevision++;pdfPageCount=0;pdfPage=1;requestedPdfPage=1;updatePdfControls();$('#pdf-area').replaceChildren();
  $('#project-navigation').hidden = false; $('#project-name').textContent = p.name;
  showRestoreNotice(false);
  try {
    const retained=await store.versionRetainedStages(p.id);if(request!==projectOpening)return;showRestoreNotice(retained>0);
    await store.versionSnapshot(p.id); if (request !== projectOpening) return;
    await refreshFiles(); if (request !== projectOpening) return;
    await openFile(p.rootFile || 'main.tex'); if (request !== projectOpening) return;
    const chat = await store.chatLoad(p.id, currentChat); if (request !== projectOpening) return;
    if(chatRequest===chatOpening) renderChat(chat); await refreshChats(); if (request === projectOpening) scheduleCompile();
  } catch (error) { if (request === projectOpening) report('open project', error); }
  finally {if(request===projectOpening && chatRequest===chatOpening){chatLoading=false;updateSendState();}}
}
async function createProject() {
  try { const p = await store.create('New Project'); projects = await store.list(); renderProjects(); await openProject(p); }
  catch (error) { report('create project', error); }
}
function* outlineSections(source:string): Generator<{title:string; index:number}> {
  const heading = /\\(?:sub)*section\*?\{/g;
  for (let match = heading.exec(source); match; match = heading.exec(source)) {
    const start = heading.lastIndex;
    const braces = /\\[\s\S]|[{}]/g;
    braces.lastIndex = start;
    let depth = 1;
    for (let token = braces.exec(source); token; token = braces.exec(source)) {
      if (token[0] === '{') depth++;
      if (token[0] === '}') depth--;
      if (depth === 0) {
        heading.lastIndex = braces.lastIndex;
        if (token.index > start) yield {title:source.slice(start, token.index), index:match.index};
        break;
      }
    }
  }
}
function renderOutlineTitle(button:HTMLButtonElement, title:string): void {
  const equations = [...title.matchAll(/\\\(((?:\\(?!\))[^\n]|[^\\\n])+?)\\\)|\\.|\$\$|\$((?:\\.|[^\\$\n])+)\$/g)]
    .filter(match => match[1] !== undefined || match[2] !== undefined);
  if (!equations.length) return;
  void mathFormatting.then(({renderMath, sanitize}) => {
    if (!button.isConnected) return;
    const label = document.createDocumentFragment();
    let offset = 0;
    for (const match of equations) {
      label.append(title.slice(offset, match.index));
      const math = document.createElement('span');
      math.innerHTML = sanitize(renderMath(match[1] ?? match[2], false), ['annotation']);
      label.append(math);
      offset = match.index + match[0].length;
    }
    label.append(title.slice(offset));
    button.replaceChildren(label);
  }).catch(error => console.error('Outline math formatting unavailable', error));
}
function updateEditor() {
  const source = $('#source').value;
  highlightLatex(source);
  updateActiveLine();
  $('#numbers').textContent = Array.from({length: source.split('\n').length}, (_, i) => i + 1).join('\n');
  const sections = [...outlineSections(source)];
  $('#outline-list').replaceChildren(...sections.map(match => {
    const button = document.createElement('button');
    button.textContent = match.title;
    renderOutlineTitle(button, match.title);
    button.onclick = () => {
      const editor = $('#source') as HTMLTextAreaElement;
      const line = source.slice(0, match.index).split('\n').length;
      editor.focus(); editor.setSelectionRange(match.index, match.index);
      const lineHeight = parseFloat(getComputedStyle(editor).lineHeight);
      editor.scrollTop = Math.max(0, (line - 1) * lineHeight - editor.clientHeight / 2);
      const prefix = source.slice(source.lastIndexOf('\n', match.index - 1) + 1, match.index);
      const measure = document.createElement('span');
      const style = getComputedStyle(editor);
      measure.style.cssText = `position:absolute;visibility:hidden;white-space:pre;font:${style.font};letter-spacing:${style.letterSpacing};tab-size:${style.tabSize}`;
      measure.textContent = prefix; document.body.append(measure);
      editor.scrollLeft = Math.max(0, measure.getBoundingClientRect().width - editor.clientWidth / 2);
      measure.remove();
      editor.dispatchEvent(new Event('scroll'));
      if (pdfDocument && compiledProject === active?.id) showInPdf.click();
    };
    return button;
  }));
}
$('#new').onclick = createProject; $('#start').onclick = createProject;
$('#home').onclick = async () => { saveEditorView();saveChatDraft();++chatOpening;chatLoading=false;++opening; ++projectOpening; active = null; clearFilePreview(); $('#projects').hidden = false; $('#workspace').hidden = true; workspaceBar.hidden = true; $('#project-navigation').hidden = true; try { projects = await store.list(); renderProjects(); } catch (error) { report('load projects', error); } };
const pendingSaves = new Map<string, Map<string, Promise<Error | null>>>();
$('#source').oninput = () => {
  const id = active.id, name = currentFile, content = $('#source').value;
  commentSelection = null; updateEditor(); revision++;
  let files = pendingSaves.get(id); if (!files) { files = new Map(); pendingSaves.set(id, files); }
  const pending = store.write(id, name, content).then(() => null, error => error instanceof Error ? error : new Error(String(error)));
  files.set(name, pending);
  pending.then(error => { if (files.get(name) !== pending) return; if (error) report('save file', error); else files.delete(name); });
  scheduleCompile();
};
function updateActiveLine() {
  const editor = $('#source') as HTMLTextAreaElement;
  const caret = editor.selectionDirection === 'backward' ? editor.selectionStart : editor.selectionEnd;
  const line = editor.value.slice(0, caret).split('\n').length - 1;
  const style = getComputedStyle(editor), lineHeight = parseFloat(style.lineHeight), paddingTop = parseFloat(style.paddingTop);
  $('#active-line').style.top = `${paddingTop + line * lineHeight - editor.scrollTop}px`;
  $('#active-line').style.height = `${lineHeight}px`;
}
function syncEditorScroll() {
  const editor = $('#source') as HTMLTextAreaElement;
  // The textarea's horizontal scrollbar reduces its viewport; give the gutter
  // the same scroll range so its last lines cannot lag behind the source.
  $('#numbers').style.paddingBottom = `${parseFloat(getComputedStyle(editor).paddingBottom) + editor.offsetHeight - editor.clientHeight}px`;
  $('#numbers').scrollTop = editor.scrollTop;
  $('#highlight').scrollTop = editor.scrollTop;
  $('#highlight').scrollLeft = editor.scrollLeft;
  updateActiveLine();
}
$('#source').onscroll = syncEditorScroll;
$('#source').addEventListener('select', updateActiveLine);
$('#source').addEventListener('click', updateActiveLine);
$('#source').addEventListener('keyup', updateActiveLine);
$('#search').oninput = renderProjects;
$('#clear-search').onclick = () => { $('#search').value = ''; renderProjects(); $('#search').focus(); };
$('#files-tab').onclick = () => { $('#file-search').hidden = false; void runProjectSearch(); $('#files-tab').classList.add('active'); $('#files-tab').setAttribute('aria-pressed','true'); $('#chats-tab').classList.remove('active'); $('#chats-tab').setAttribute('aria-pressed','false'); $('#file-list').hidden = false; $('#outline').hidden = false; $('#chat-list').hidden = true; };
$('#chats-tab').onclick = () => { $('#file-search').hidden = true; ++searchRequest; clearTimeout(searchTimer); $('#chats-tab').classList.add('active'); $('#chats-tab').setAttribute('aria-pressed','true'); $('#files-tab').classList.remove('active'); $('#files-tab').setAttribute('aria-pressed','false'); $('#file-list').hidden = true; $('#outline').hidden = true; $('#chat-list').hidden = false; };
store.list().then(p => { projects = p; renderProjects(); }).catch(error => report('load projects', error));
let compiledProject = null, compiledPdf: string | null = null, compileLog = '';
type PdfViewport = { width:number; height:number };
type PdfPage = { getViewport(options: {scale:number}): PdfViewport; render(options:{canvasContext:CanvasRenderingContext2D;viewport:PdfViewport}): {promise:Promise<unknown>} };
type PdfDocument = { numPages:number; getPage(page:number):Promise<PdfPage> };
let pdfZoom=100, pdfPage=1, pdfPageCount=0, requestedPdfPage=1;
let pdfFitMode: 'width' | 'page' | 'custom' = 'width';
let pdfDocument:PdfDocument | null = null, pdfRenderRevision=0;
let lastPdfSync:{page:number;x:number;y:number}|null=null;
let pdfSharpTimer: ReturnType<typeof setTimeout> | null = null;
const pdfControls=document.createElement('div');pdfControls.className='pdf-controls';
pdfControls.innerHTML='<span class="pdf-control-label" aria-hidden="true">Page</span><button type="button" aria-label="Previous compiled PDF page">‹</button><span aria-live="polite" class="pdf-page-count"><button type="button" class="pdf-current-page" aria-label="Change compiled PDF page" title="Go to page">0</button><input hidden type="text" inputmode="numeric" aria-label="Go to compiled PDF page"><span class="pdf-page-total"> / 0</span></span><button type="button" aria-label="Next compiled PDF page">›</button><span class="pdf-control-label pdf-zoom-label" aria-hidden="true">Zoom</span><button type="button" aria-label="Zoom out compiled PDF">−</button><span class="pdf-zoom-value">100%</span><button type="button" aria-label="Zoom in compiled PDF">+</button>';
$('#zoom-fit').textContent='Fit width'; $('#zoom-fit').setAttribute('aria-label','Fit PDF to width');
$('#download').replaceChildren(actionIcon('download')); $('#download').setAttribute('aria-label','Download PDF');
$('#export-project').replaceChildren(actionIcon('export')); $('#export-project').setAttribute('aria-label','Export project');
$('#zoom-fit').before(pdfControls);
const fitPage = document.createElement('button'); fitPage.type='button'; fitPage.className='icon fit-page'; fitPage.textContent='Fit page'; fitPage.setAttribute('aria-label','Fit PDF page to view');
function fitPdfPage():void { const frame=pdfArea.querySelector<HTMLElement>('.pdf-pages .pdf-page-frame'); if(!frame) return; const height=frame.getBoundingClientRect().height; if(height>0) { changePdfZoom(Math.min(100,Math.floor(pdfZoom * Math.max(1,pdfArea.clientHeight - 36) / height)),'page'); goToPdfPage(pdfPage); } }
fitPage.onclick=fitPdfPage;
$('#zoom-fit').after(fitPage);
// Keep fit modes beside zoom in a single responsive control group.
const pdfFitControls = document.createElement('div'); pdfFitControls.className = 'pdf-fit-controls';
pdfFitControls.append(document.getElementById('zoom-fit')!, fitPage); pdfControls.append(pdfFitControls);

const pdfArea=$('#pdf-area') as HTMLElement;
// Zoom restores its own page anchor after layout changes.
pdfArea.style.overflowAnchor='none';
const pdfPageInput=pdfControls.querySelector<HTMLInputElement>('[aria-label="Go to compiled PDF page"]')!;
const pdfCurrentPage=pdfControls.querySelector<HTMLButtonElement>('.pdf-current-page')!;
const pdfPageTotal=pdfControls.querySelector<HTMLElement>('.pdf-page-total')!;
const showInPdf=document.createElement('button');showInPdf.type='button';showInPdf.className='plain';showInPdf.textContent='Show in PDF';showInPdf.title='Show the current source line in the compiled PDF';
$('.editor-header').insertBefore(showInPdf,$('#tools'));
function updatePdfControls() {
  pdfControls.hidden = pdfPageCount === 0;
  $('#zoom-fit').hidden = pdfPageCount === 0; fitPage.hidden = pdfPageCount === 0;
  pdfCurrentPage.textContent=String(pdfPageCount ? pdfPage : 0);
  pdfPageTotal.textContent=` / ${pdfPageCount}`;
  pdfControls.querySelector<HTMLElement>('.pdf-zoom-value')!.textContent=pdfFitMode==='width' ? 'Fit' : `${pdfZoom}%`;
  $('#zoom-fit').setAttribute('aria-pressed',String(pdfFitMode==='width')); fitPage.setAttribute('aria-pressed',String(pdfFitMode==='page'));
  pdfControls.querySelector<HTMLButtonElement>('[aria-label="Previous compiled PDF page"]')!.disabled=pdfPageCount===0 || pdfPage<=1;
  pdfControls.querySelector<HTMLButtonElement>('[aria-label="Next compiled PDF page"]')!.disabled=pdfPageCount===0 || pdfPage>=pdfPageCount;
  pdfCurrentPage.disabled=pdfPageCount===0; pdfPageInput.disabled=pdfPageCount===0; if(!pdfPageCount) finishPdfPageEdit(false);
  showInPdf.disabled=pdfPageCount===0 || $('.source-container').hidden;
  pdfControls.querySelector<HTMLButtonElement>('[aria-label="Zoom out compiled PDF"]')!.disabled=pdfPageCount===0 || pdfZoom<=25;
  pdfControls.querySelector<HTMLButtonElement>('[aria-label="Zoom in compiled PDF"]')!.disabled=pdfPageCount===0 || pdfZoom>=250;
  $('#zoom-fit').disabled=pdfPageCount===0;
}
async function sharpenPdfPages(doc:PdfDocument, revision:number, zoom:number) {
  const canvases = [...pdfArea.querySelectorAll<HTMLCanvasElement>('.pdf-pages canvas')];
  try {
    for (let i=0; i<canvases.length; i++) {
      if (revision !== pdfRenderRevision || doc !== pdfDocument) return;
      const bounds = canvases[i].getBoundingClientRect(), area = pdfArea.getBoundingClientRect();
      if (bounds.bottom < area.top - area.height || bounds.top > area.bottom + area.height) continue;
      const page = await doc.getPage(i+1), base=page.getViewport({scale:1});
      if (revision !== pdfRenderRevision || doc !== pdfDocument) return;
      // Match physical display pixels while bounding the canvas for unusually large pages.
      const requested=Math.max(1.4*Math.max(1,zoom/100),bounds.width*Math.max(1,window.devicePixelRatio || 1)/base.width);
      const scale=Math.min(requested,8192/base.width,8192/base.height,Math.sqrt(16000000/(base.width*base.height)));
      if (Number(canvases[i].dataset.renderScale || 0) >= scale) continue;
      const viewport = page.getViewport({scale});
      const replacement = document.createElement('canvas');
      replacement.width = Math.ceil(viewport.width); replacement.height = Math.ceil(viewport.height);
      replacement.dataset.renderScale = String(scale);
      replacement.style.width = '100%'; replacement.setAttribute('aria-label', `Page ${i+1}`);replacement.tabIndex=0;replacement.title='Double-click or press Enter to find this position in the source';
      const context = replacement.getContext('2d'); if (!context) throw new Error('Could not render PDF page');
      await page.render({canvasContext:context,viewport}).promise;
      if (revision !== pdfRenderRevision || doc !== pdfDocument || !canvases[i].isConnected) return;
      const focused=document.activeElement===canvases[i];canvases[i].replaceWith(replacement);if(focused)replacement.focus({preventScroll:true});
    }
  } catch (error) { console.error('PDF zoom rendering failed', error); }
}
function schedulePdfSharpen() {
  const revision=++pdfRenderRevision;
  if (pdfSharpTimer) clearTimeout(pdfSharpTimer);
  pdfSharpTimer=setTimeout(() => { pdfSharpTimer=null; if(pdfDocument) void sharpenPdfPages(pdfDocument,revision,pdfZoom); }, 80);
}
function changePdfZoom(zoom:number, mode:'width'|'page'|'custom'='custom') {
  if(!pdfPageCount) return;
  const pages=[...pdfArea.querySelectorAll<HTMLElement>('.pdf-pages .pdf-page-frame')];
  const area=pdfArea.getBoundingClientRect(),center=area.top+pdfArea.clientHeight/2;
  const anchor=pages.reduce<HTMLElement | undefined>((closest,canvas)=>{
    const distance=(page:HTMLElement)=>{const bounds=page.getBoundingClientRect();return Math.max(bounds.top-center,center-bounds.bottom,0);};
    return !closest || distance(canvas)<distance(closest) ? canvas : closest;
  },undefined);
  const bounds=anchor?.getBoundingClientRect();
  const position=bounds && bounds.height ? {x:(area.left+pdfArea.clientWidth/2-bounds.left)/bounds.width,y:(center-bounds.top)/bounds.height} : null;
  pdfFitMode=mode;pdfZoom=Math.min(250,Math.max(25,zoom));
  pages.forEach(canvas=>canvas.style.width=`${pdfZoom}%`);
  // Keep the same point on the visible page under the viewport center.
  if(anchor && position && pdfArea.clientHeight) {
    const resized=anchor.getBoundingClientRect();
    pdfArea.scrollTop+=resized.top+position.y*resized.height-area.top-pdfArea.clientHeight/2;
    pdfArea.scrollLeft+=resized.left+position.x*resized.width-area.left-pdfArea.clientWidth/2;
  }
  updatePdfControls();
  schedulePdfSharpen();
}
function goToPdfPage(page:number) {
  const pages=pdfArea.querySelectorAll<HTMLElement>('.pdf-pages canvas'), target=pages[page-1];if(!target) return;
  pdfPage=page;pdfArea.scrollTop+=target.getBoundingClientRect().top-pdfArea.getBoundingClientRect().top-18;updatePdfControls();
}
pdfControls.querySelector<HTMLButtonElement>('[aria-label="Previous compiled PDF page"]')!.onclick=()=>goToPdfPage(pdfPage-1);
pdfControls.querySelector<HTMLButtonElement>('[aria-label="Next compiled PDF page"]')!.onclick=()=>goToPdfPage(pdfPage+1);
function finishPdfPageEdit(restoreFocus:boolean):void {
  pdfPageInput.hidden=true; pdfCurrentPage.hidden=false;
  pdfPageInput.value=''; pdfPageInput.removeAttribute('aria-invalid');
  if(restoreFocus) pdfCurrentPage.focus();
}
pdfCurrentPage.onclick=()=>{
  pdfCurrentPage.hidden=true; pdfPageInput.hidden=false;
  pdfPageInput.value=String(pdfPage); pdfPageInput.focus(); pdfPageInput.select();
};
function submitPdfPage():void {
  const value=pdfPageInput.value.trim(), page=Number(value);
  if (!/^[0-9]+$/.test(value) || !Number.isSafeInteger(page) || page<1 || page>pdfPageCount) {
    pdfPageInput.setAttribute('aria-invalid','true'); pdfPageInput.select(); return;
  }
  goToPdfPage(page); finishPdfPageEdit(true);
}
pdfPageInput.addEventListener('keydown',event=>{
  if(event.key==='Enter'){event.preventDefault();submitPdfPage();}
  else if(event.key==='Escape'){event.preventDefault();event.stopPropagation();finishPdfPageEdit(true);}
});
pdfPageInput.addEventListener('input',()=>pdfPageInput.removeAttribute('aria-invalid'));
pdfPageInput.addEventListener('blur',()=>finishPdfPageEdit(false));
showInPdf.onclick=async()=>{
  if(!active || !compiledProject || compiledProject!==active.id || !currentFile || $('.source-container').hidden)return;
  closeLayoutTools();paneLayout={...visiblePaneLayout(),preview:true};if(window.matchMedia('(max-width:1200px)').matches) paneLayout={agent:false,editor:false,preview:true};applyPaneLayout();
  const id=active.id,file=currentFile,version=revision,editor=$('#source') as HTMLTextAreaElement,line=editor.value.slice(0,editor.selectionStart).split('\n').length;
  try {
    const position=await store.syncForward(id,file,line),doc=pdfDocument;
    if(active?.id!==id || currentFile!==file || revision!==version || compiledProject!==id || !doc)return;
    const viewport=(await doc.getPage(position.page)).getViewport({scale:1});
    if(active?.id!==id || currentFile!==file || revision!==version || compiledProject!==id || pdfDocument!==doc)return;
    schedulePdfSharpen();
    const canvas=pdfArea.querySelectorAll<HTMLCanvasElement>('.pdf-pages canvas')[position.page-1];if(!canvas)return;
    const bounds=canvas.getBoundingClientRect(),area=pdfArea.getBoundingClientRect();
    lastPdfSync=position;pdfPage=position.page;pdfArea.scrollTop+=bounds.top-area.top+(position.y/viewport.height)*bounds.height-area.height/2;updatePdfControls();
  }
  catch(error){if(active?.id===id && revision===version)report('find source in PDF',error);}
};
async function navigatePdfSource(canvas:HTMLCanvasElement,clientX:number,clientY:number) {
  const doc=pdfDocument;
  if(!canvas || !doc || !active || compiledProject!==active.id)return;
  const id=active.id,version=revision,index=[...pdfArea.querySelectorAll<HTMLCanvasElement>('.pdf-pages canvas')].indexOf(canvas),rect=canvas.getBoundingClientRect();
  if(index<0 || !rect.width || !rect.height)return;
  try {
    const viewport=(await doc.getPage(index+1)).getViewport({scale:1});
    const result=await store.syncReverse(id,index+1,(clientX-rect.left)*viewport.width/rect.width,(clientY-rect.top)*viewport.height/rect.height);
    if(active?.id!==id || revision!==version || compiledProject!==id || pdfDocument!==doc)return;
    await openFile(result.file);if(active?.id!==id || revision!==version || currentFile!==result.file)return;
    const editor=$('#source') as HTMLTextAreaElement,lines=editor.value.split('\n');if(result.line<1 || result.line>lines.length)return;
    const offset=lines.slice(0,result.line-1).reduce((n,line)=>n+line.length+1,0); closeLayoutTools();paneLayout={...visiblePaneLayout(),editor:true};if(window.matchMedia('(max-width:1200px)').matches)paneLayout.preview=false;applyPaneLayout(); editor.focus();editor.setSelectionRange(offset,offset+lines[result.line-1].length);
    const style=getComputedStyle(editor),lineHeight=parseFloat(style.lineHeight);
    editor.scrollTop=Math.max(0,parseFloat(style.paddingTop)+(result.line-1)*lineHeight-(editor.clientHeight-lineHeight)/2);
    syncEditorScroll();
  } catch(error){if(active?.id===id && revision===version)report('find PDF source',error);}
}
pdfArea.addEventListener('dblclick',event=>{if((event.target as HTMLElement).closest('a,button,input,select,textarea'))return;const canvas=(event.target as HTMLElement).closest<HTMLCanvasElement>('.pdf-pages canvas');if(canvas)void navigatePdfSource(canvas,event.clientX,event.clientY);});
pdfArea.addEventListener('keydown',event=>{
  if(event.key!=='Enter' && event.key!==' ')return;
  const canvas=(event.target as HTMLElement).closest<HTMLCanvasElement>('.pdf-pages canvas');if(!canvas)return;
  event.preventDefault();const rect=canvas.getBoundingClientRect(),doc=pdfDocument,index=[...pdfArea.querySelectorAll<HTMLCanvasElement>('.pdf-pages canvas')].indexOf(canvas),saved=lastPdfSync;
  if(doc && saved && saved.page===index+1) {
    void doc.getPage(saved.page).then(page=>{if(pdfDocument!==doc || !canvas.isConnected)return;const viewport=page.getViewport({scale:1}),bounds=canvas.getBoundingClientRect();return navigatePdfSource(canvas,bounds.left+saved.x*bounds.width/viewport.width,bounds.top+saved.y*bounds.height/viewport.height);}).catch(error=>{if(pdfDocument===doc)report('find PDF source',error);});
  } else void navigatePdfSource(canvas,rect.left+rect.width/2,rect.top+rect.height/2);
});
pdfControls.querySelector<HTMLButtonElement>('[aria-label="Zoom out compiled PDF"]')!.onclick=()=>changePdfZoom(pdfZoom-25);
pdfControls.querySelector<HTMLButtonElement>('[aria-label="Zoom in compiled PDF"]')!.onclick=()=>changePdfZoom(pdfZoom+25);
pdfArea.addEventListener('scroll',()=>{
  const pages=[...pdfArea.querySelectorAll<HTMLElement>('.pdf-pages canvas')];if(!pages.length)return;
  const rect=pdfArea.getBoundingClientRect(),center=rect.top+rect.height/2;let page=1,closest=Infinity;
  pages.forEach((canvas,index)=>{const bounds=canvas.getBoundingClientRect();const distance=center<bounds.top ? bounds.top-center : center>bounds.bottom ? center-bounds.bottom : 0;if(distance<closest){closest=distance;page=index+1;}});
  if(page!==pdfPage){pdfPage=page;updatePdfControls();}
  schedulePdfSharpen();
});
window.addEventListener('resize',()=>{if(pdfDocument){if(pdfFitMode==='page') fitPdfPage(); else schedulePdfSharpen();}});
updatePdfControls();
const pendingDraftSaves=new Set<string>();
let showingTools = false, selectedTool = 'logs';
function showCompileError(output: string) {
  const panel = document.createElement('section'); panel.className = 'compile-error'; panel.setAttribute('role', 'alert');
  const title = document.createElement('h2'); title.textContent = 'Could not compile';
  const message = document.createElement('p');
  const lines = output.split(/\r?\n/), cause = lines.find(line => /^!\s/.test(line)), location = lines.find(line => /^l\.\d+/.test(line));
  message.textContent = [cause, location].filter(Boolean).join(' - ') || lines.find(line => line.trim()) || 'Check the build output for details, then compile again.';
  const guidance = document.createElement('p'); guidance.className = 'compile-guidance'; guidance.textContent = cause ? 'Fix the issue in the source, then compile again.' : 'Resolve the issue, then compile again.';
  const details = document.createElement('details'); const summary = document.createElement('summary'); summary.textContent = 'View build output';
  const log = document.createElement('pre'); log.className = 'compile-log'; log.textContent = output;
  details.append(summary, log); panel.append(title, message, guidance);
  const lineNumber = location?.match(/^l\.(\d+)/)?.[1];
  if (lineNumber && currentFile && !$('.source-container').hidden) {
    const errorFile=currentFile, errorProject=active?.id; const jump = document.createElement('button'); jump.type='button'; jump.className='secondary compile-line-action';
    jump.textContent=`Go to line ${lineNumber} in ${currentFile}`;
    jump.onclick=async()=>{ if (!errorProject || active?.id !== errorProject) return; if(currentFile !== errorFile) await openFile(errorFile); if(active?.id !== errorProject || currentFile !== errorFile) return; const editor=$('#source') as HTMLTextAreaElement, rows=editor.value.split('\n'), index=Number(lineNumber)-1; if(index<0 || index>=rows.length) return; const offset=rows.slice(0,index).reduce((sum,line)=>sum+line.length+1,0); editor.focus(); editor.setSelectionRange(offset,offset+rows[index].length); editor.scrollTop=Math.max(0,index*parseFloat(getComputedStyle(editor).lineHeight)-editor.clientHeight/3); updateActiveLine(); };
    panel.append(jump);
  }
  panel.append(details); $('#pdf-area').replaceChildren(panel);
}
function scheduleCompile() {
  if (pdfPageCount) requestedPdfPage=pdfPage;
  compiledProject = null; compiledPdf = null; $('#download').disabled = true;
  lastPdfSync=null;
  pdfDocument = null; pdfRenderRevision++;
  pdfPageCount=0;pdfPage=1;updatePdfControls();
  compileLog = 'Changes pending compilation';
  if (showingTools && selectedTool === 'logs') renderToolPanel();
  const placeholder = document.createElement('div'); placeholder.className = 'pdf-placeholder'; placeholder.setAttribute('role','status');
  const placeholderTitle = document.createElement('strong'); placeholderTitle.textContent = 'Changes pending compilation';
  const placeholderHint = document.createElement('span'); placeholderHint.textContent = 'Your preview will update after the document is compiled.';
  placeholder.append(placeholderTitle,placeholderHint);
  $('#pdf-area').replaceChildren(placeholder);
  $('#compile').textContent = 'Compile';
  clearTimeout(compileTimer);
  // Queue the next event-loop turn so synchronous edits coalesce without a typing delay.
  compileTimer = setTimeout(() => compilePreview(), 0);
}
async function compilePreview(force=false) {
  if (!active) return;
  if (restoringProject === active.id) { compileQueued = true; return; }
  if (compileRunning) { compileQueued = true; return; }
  compileRunning = true;
  const id = active.id, version = revision;
  $('#compile').disabled = true; $('#compile').textContent = 'Compiling…';
  try {
    const saveResults = await Promise.all([...(pendingSaves.get(id)?.values() || [])]);
    if (!active || active.id !== id || revision !== version) { compileQueued = !!active; return; }
    const failedSave = saveResults.find(Boolean);
    if (failedSave) throw new Error(`Cannot compile until the file is saved: ${failedSave.message}`);
    const result = await store.compile(id,force);
    if (!active || active.id !== id || revision !== version) { compileQueued = !!active; return; }
    compileLog = result.log || '';
    if (showingTools && selectedTool === 'logs') renderToolPanel();
    if (!result.ok) {
      showCompileError(result.log || 'No build output was provided.'); compiledProject = null; $('#compile').textContent = 'Errors'; return;
    }
    const bytes = Uint8Array.from(atob(result.pdf), c => c.charCodeAt(0));
    const pdfModulePath: string = '../node_modules/pdfjs-dist/build/pdf.mjs';
    const pdfjs = await import(pdfModulePath);
    pdfjs.GlobalWorkerOptions.workerSrc = '../node_modules/pdfjs-dist/build/pdf.worker.mjs';
    const doc = await pdfjs.getDocument({ data: bytes, isEvalSupported:false }).promise;
    const container = document.createElement('div'); container.className = 'pdf-pages';
    for (let i = 1; i <= doc.numPages; i++) {
      if (!active || active.id !== id || revision !== version) { compileQueued = !!active; return; }
      const page = await doc.getPage(i), base = page.getViewport({scale:1});
      // Prepare every page before publishing the preview, so even a jump to the
      // end has content immediately. Bound this first pass to roughly 16M pixels
      // across the document; visible pages are sharpened separately below.
      const scale = Math.min(1.4,1024/base.width,1024/base.height,Math.sqrt(16000000/(doc.numPages*base.width*base.height)));
      const viewport = page.getViewport({scale});
      const canvas = document.createElement('canvas'); canvas.width = Math.ceil(viewport.width); canvas.height = Math.ceil(viewport.height);
      const context = canvas.getContext('2d'); if (!context) throw new Error('Could not render PDF page');
      await page.render({canvasContext:context,viewport}).promise;
      if (!active || active.id !== id || revision !== version) { compileQueued = !!active; return; }
      canvas.dataset.renderScale = String(scale);
      canvas.setAttribute('aria-label', `Page ${i}`);canvas.tabIndex=0;canvas.title='Double-click or press Enter to find this position in the source';
      const frame=document.createElement('div');frame.className='pdf-page-frame';frame.style.width=`${pdfZoom}%`;frame.append(canvas);
      const annotations=await page.getAnnotations({intent:'display'});
      for(const annotation of annotations){
        if(annotation.subtype!=='Link' || (!annotation.url && !annotation.dest))continue;
        const [left,bottom,right,top]=viewport.convertToViewportRectangle(annotation.rect);
        const link=document.createElement('a');link.className='pdf-annotation-link';link.setAttribute('aria-label',annotation.url?'Open PDF link':'Go to linked PDF location');
        link.style.left=`${Math.min(left,right)/viewport.width*100}%`;link.style.top=`${Math.min(top,bottom)/viewport.height*100}%`;
        link.style.width=`${Math.abs(right-left)/viewport.width*100}%`;link.style.height=`${Math.abs(bottom-top)/viewport.height*100}%`;
        if(annotation.url){link.href=annotation.url;link.addEventListener('click',event=>{event.preventDefault();void store.openExternal(annotation.url).catch(error=>report('open PDF link',error));});}
        else { link.href='#'; link.addEventListener('click',event=>{event.preventDefault();void (async()=>{const destination=typeof annotation.dest==='string'?await doc.getDestination(annotation.dest):annotation.dest;if(!destination?.length)return;const pageIndex=typeof destination[0]==='number'?destination[0]:await doc.getPageIndex(destination[0]);if(doc===pdfDocument)goToPdfPage(pageIndex+1);})().catch(error=>report('open PDF link',error));}); }
        frame.append(link);
      }
      container.append(frame);
    }
    if (!active || active.id !== id || revision !== version) { compileQueued = !!active; return; }
    $('#pdf-area').replaceChildren(container);pdfDocument=doc;pdfPageCount=doc.numPages;pdfPage=1;changePdfZoom(pdfZoom,pdfFitMode);if(pdfFitMode==='page') fitPdfPage();goToPdfPage(Math.min(requestedPdfPage,doc.numPages)); compiledProject = id; compiledPdf = result.pdf; $('#download').disabled = false; $('#compile').textContent = 'Compiled';
  } catch (error) {
    if (active && active.id === id && revision === version) {
      compiledProject = null; compiledPdf = null; $('#download').disabled = true;
      compileLog = error.message || String(error);
      showCompileError(compileLog); $('#compile').textContent = 'Errors';
      if (showingTools && selectedTool === 'logs') renderToolPanel();
    } else if (active) compileQueued = true;
  }
  finally { compileRunning = false; $('#compile').disabled = false; if (compileQueued) { compileQueued = false; scheduleCompile(); } }
}
$('#compile').onclick = () => { scheduleCompile(); clearTimeout(compileTimer); compilePreview(true); };
$('#download').onclick = () => { if (active && compiledProject === active.id && compiledPdf) store.savePdf(compiledPdf).catch(error => report('save PDF', error)); };
const purifierPath: string='../node_modules/dompurify/dist/purify.es.mjs';
const katexPath: string='../node_modules/katex/dist/katex.mjs';
const mathFormatting = Promise.all([import(purifierPath), import(katexPath)]).then(([{default:purify}, {default:katex}]) => ({
  renderMath: (text:string, displayMode:boolean):string => katex.renderToString(text,{displayMode,output:'mathml',throwOnError:false,trust:false}),
  sanitize: (html:string, dropContents:string[] = []):string => purify.sanitize(html,{ADD_FORBID_CONTENTS:dropContents,USE_PROFILES:{html:true,mathMl:true},FORBID_TAGS:['img','style','iframe'],FORBID_ATTR:['src','style']})
}));
const assistantFormatter: Promise<(text:string)=>string> = (async () => {
  const markedPath: string='../node_modules/marked/lib/marked.esm.js';
  const [{Marked}, {renderMath, sanitize}] = await Promise.all([import(markedPath), mathFormatting]);
  const math = (display:boolean) => ({
    name:display ? 'displayMath' : 'inlineMath', level:display ? 'block' : 'inline',
    start(source:string) { return source.search(display ? /\$\$|\\\[/ : /\$|\\\(/); },
    tokenizer(source:string) {
      const match=display ? /^(?:\$\$([\s\S]+?)\$\$|\\\[([\s\S]+?)\\\])(?:\n|$)/.exec(source) : /^(?:\$(?![\s$])((?:\\.|[^$\\\n])+?)(?<!\s)\$|\\\(([^\n]+?)\\\))/.exec(source);
      if(match) return {type:display ? 'displayMath' : 'inlineMath',raw:match[0],text:match[1] ?? match[2]};
    },
    renderer(token:{text:string}) { return renderMath(token.text, display); }
  });
  const parser=new Marked({extensions:[math(true),math(false)]});
  return (text:string) => sanitize(parser.parse(text));
})();
function renderChat(chat) {
  if (active?.id) window.dispatchEvent(new CustomEvent('picker-project', {detail:active.id}));
  const provider = $('#chat-provider') as HTMLSelectElement;
  const locked = Boolean(chat.provider || chat.sessionId || chat.messages.length);
  const saved = chat.provider || (locked ? 'codex' : null);
  const request=++providerDiscovery;
  providerAvailable=false;provider.disabled=true;provider.dispatchEvent(new Event('providerstate'));updateSendState();
  void store.discoverProviders().then((available:{codex:boolean;opencode:boolean})=>{
    if(request!==providerDiscovery)return;
    provider.replaceChildren();
    for(const [id,label] of [['codex','Codex'],['opencode','OpenCode']]) {
      if(!available[id] && saved!==id)continue;
      const option=document.createElement('option');option.value=id;option.disabled=!available[id];option.textContent=available[id] ? label : `${label} (not installed)`;provider.append(option);
    }
    if(!provider.options.length) {const option=document.createElement('option');option.value='';option.textContent='No providers found';provider.append(option);}
    if(saved)provider.value=saved;
    provider.disabled=!provider.value;
    providerAvailable=Boolean(available[provider.value]);
    provider.dispatchEvent(new Event('providerstate')); updateSendState();
  }).catch(()=>{if(request===providerDiscovery){const option=document.createElement('option');option.value='';option.textContent='Provider detection failed';provider.replaceChildren(option);provider.disabled=true;providerAvailable=false;provider.dispatchEvent(new Event('providerstate'));updateSendState();}});
  updateChatProgress();
  const pane = $('#chat-messages');
  if (!chat.messages.length) {
    if (welcomeDismissed) { pane.replaceChildren(); return; }
    const title = document.createElement('strong'), intro = document.createElement('p'), close = document.createElement('button');
    close.className = 'dismiss-welcome'; close.setAttribute('aria-label', 'Dismiss welcome'); close.replaceChildren(actionIcon('close'));
    close.addEventListener('mousedown', event => event.preventDefault());
    close.onclick = () => { welcomeDismissed = true; pane.replaceChildren(); };
    title.textContent = 'Welcome to Refract!'; intro.textContent = 'Ask about your LaTeX, research, or writing.';
    pane.replaceChildren(close, title, intro); return;
  }
  pane.replaceChildren(...chat.messages.map(message => {
    if (message.role === 'handoff') {
      const marker = document.createElement('div'); marker.className = 'provider-handoff';
      const labels: Record<string,string> = {codex:'Codex',opencode:'OpenCode'};
      const from = labels[message.from] || message.from, to = labels[message.to] || message.to;
      const caption = document.createElement('span'); caption.textContent = message.status === 'failed' ? 'Context handoff failed' : message.status === 'pending' ? 'Context handoff pending' : 'Context handoff';
      const endpoints = document.createElement('span'); endpoints.className = 'handoff-endpoints'; endpoints.textContent = `${from} \u2192 ${to}`;
      marker.append(caption, endpoints); marker.setAttribute('aria-label', `${caption.textContent}: ${from} to ${to}`);
      return marker;
    }
    const block = document.createElement('div'); block.className = `message ${message.role}`;
    const content=document.createElement('div');content.textContent=message.text;block.append(content);
    if(message.role==='assistant') void assistantFormatter.then(format=>{if(content.isConnected) {
      content.className='formatted-response';content.innerHTML=format(message.text);
      content.querySelectorAll<HTMLAnchorElement>('a[href]').forEach(link=>{
        const href=link.getAttribute('href')!;
        try {const url=new URL(href);if(!['http:','https:'].includes(url.protocol) || url.username || url.password) throw new Error('Unsupported link');}
        catch {link.removeAttribute('href');return;}
        link.addEventListener('click',event=>{event.preventDefault();void store.openExternal(href).catch(error=>report('open link',error));});
        link.addEventListener('auxclick',event=>event.preventDefault());
      });
    }}).catch(error=>console.error('Assistant formatting unavailable',error));
    if (message.attachments?.length) { const files = document.createElement('div'); files.className='message-attachments'; for (const item of message.attachments) files.append(attachmentPreview(item, active.id)); block.append(files); } return block;
  }));
  const latest = Array.from((pane as HTMLElement).querySelectorAll<HTMLElement>('.message.user')).at(-1);
  pane.scrollTop = latest ? pane.scrollTop + latest.getBoundingClientRect().top - pane.getBoundingClientRect().top - 12 : pane.scrollHeight;
}
$('#chat-provider').onchange = () => { providerAvailable=Boolean(($('#chat-provider') as HTMLSelectElement).value && !($('#chat-provider') as HTMLSelectElement).selectedOptions[0]?.disabled); $('#chat-provider').dispatchEvent(new Event('providerstate'));updateSendState(); };
async function sendChat() {
  if (!active || !providerAvailable || chatLoading || restoringProject === active.id || chatBusy.has(active.id) || pendingAttachments.has(draftKey(active.id,currentChat)) || !$('#chat-input').value.trim()) return;
  const id = active.id, chatId = currentChat, prompt = $('#chat-input').value, images = attachments, provider = ($('#chat-provider') as HTMLSelectElement).value;
  $('#chat-provider').disabled = true; $('#chat-provider').dispatchEvent(new Event('providerstate'));
  attachments = []; renderAttachments();
  $('#chat-input').value = ''; $('#send').disabled = true; chatBusy.add(id);chatProgress.set(id,{chatId,status:'Preparing…'});updateChatProgress(); updateCancelButton(); setEditorReadOnly(true);
  saveChatDraft();
  const pending = document.createElement('div'); pending.className = 'message user'; pending.textContent = prompt;
  if ($('#chat-messages .dismiss-welcome')) $('#chat-messages').replaceChildren(pending); else $('#chat-messages').append(pending);
  let submitted=false, chat=null;
  try {
    await (window as unknown as Window & { codexPickerReady: () => Promise<void> }).codexPickerReady();
    submitted=true;chat=await store.chatSend(id,prompt,chatId,images,provider);
  } catch (error) {
    if(String(error.message).includes('Save pending changes before sending a message.')) submitted=false;
    if(active?.id===id && currentChat===chatId) {
      if(!String(error.message).includes('Assistant operation finished, but its recovery snapshot failed')) {
        if(!$('#chat-input').value) $('#chat-input').value=prompt;
        if(!attachments.length) {attachments=images;renderAttachments();}
        saveChatDraft();
      }
      pending.remove(); if (!submitted) { $('#chat-provider').disabled = false; $('#chat-provider').dispatchEvent(new Event('providerstate')); } report('send message',error);
    } else if(!String(error.message).includes('Assistant operation finished, but its recovery snapshot failed')) {
      const key=draftKey(id,chatId),draft=readChatDraft(key);
      if(!draft.text)draft.text=prompt;if(!draft.attachments.length)draft.attachments=images;
      writeChatDraft(key,draft);
    }
  } finally {
    try {
      if(active?.id===id && submitted) {
        const files=await store.files(id);if(active?.id===id) {
          openFiles=openFiles.filter(name=>files.includes(name));const selected=currentFile;
          await refreshFiles();
          if(active?.id===id) {
            if(selected && files.includes(selected)) await openFile(selected);
            else {clearEditor();const root=active.rootFile || 'main.tex';if(files.includes(root)) await openFile(root);}
            if(currentChat===chatId) {
              const request=chatOpening, refreshed=chat || await store.chatLoad(id,chatId);
              if(active?.id===id && currentChat===chatId && request===chatOpening) renderChat(refreshed);
            }
            await refreshChats();revision++;scheduleCompile();
          }
        }
      }
    } catch(error) {if(active?.id===id) report('refresh project after assistant',error);}
    chatBusy.delete(id);chatProgress.delete(id);updateChatProgress();
    if(active) {setEditorReadOnly(chatBusy.has(active.id) || restoringProject===active.id);updateSendState();}updateCancelButton();
  }
}
const chatProgressElement=document.createElement('div');chatProgressElement.className='chat-progress';chatProgressElement.setAttribute('role','status');chatProgressElement.setAttribute('aria-live','polite');chatProgressElement.hidden=true;
$('#chat-messages').after(chatProgressElement);
function updateChatProgress() {
  const progress=active && chatProgress.get(active.id);
  chatProgressElement.hidden=!progress || progress.chatId!==currentChat;
  chatProgressElement.textContent=chatProgressElement.hidden ? '' : progress.status;
}
store.onChatProgress(update=>{
  if(!update || !chatBusy.has(update.projectId) || typeof update.status!=='string')return;
  chatProgress.set(update.projectId,{chatId:update.chatId,status:update.status});updateChatProgress();
});
const cancelChatButton=document.createElement('button');cancelChatButton.type='button';cancelChatButton.className='stop-chat';cancelChatButton.setAttribute('aria-label','Stop');cancelChatButton.title='Stop assistant';cancelChatButton.hidden=true;
$('#send').before(cancelChatButton);
function updateCancelButton() {cancelChatButton.hidden=!active || !chatBusy.has(active.id);cancelChatButton.disabled=false;cancelChatButton.setAttribute('aria-label','Stop');$('#send').hidden=!cancelChatButton.hidden;}
cancelChatButton.onclick=async()=>{
  if(!active || !chatBusy.has(active.id)) return;
  const id=active.id;cancelChatButton.disabled=true;cancelChatButton.setAttribute('aria-label','Stopping…');
  try {await store.chatCancel(id);} catch(error) {if(active?.id===id){report('stop assistant',error);updateCancelButton();}}
};
$('#send').onclick = sendChat;
$('#chat-input').addEventListener('input',saveChatDraft);
$('#chat-input').onkeydown = event => { if (event.isComposing || event.keyCode === 229) return; if (event.key === 'Enter' && !event.shiftKey) { event.preventDefault(); sendChat(); } };
const fileNameError = document.createElement('p'); fileNameError.id = 'file-name-error'; fileNameError.className = 'field-error'; fileNameError.hidden = true; fileNameError.textContent = 'Enter a file or folder name.'; fileNameError.setAttribute('role', 'alert');
$('#file-name').after(fileNameError); $('#file-name').setAttribute('aria-describedby', fileNameError.id);
$('#file-name').addEventListener('invalid', event => { event.preventDefault(); fileNameError.hidden=false; $('#file-name').setAttribute('aria-invalid','true'); });
$('#file-name').addEventListener('input', () => { fileNameError.hidden=true; $('#file-name').removeAttribute('aria-invalid'); });
async function openAddFile(destination='') {
  if(!active) return; const id=active.id;
  try {
    const tree=await store.tree(id); if(active?.id!==id) return;
    const select=$('#file-destination'); select.replaceChildren();
    const add=(value,label)=>{const option=document.createElement('option');option.value=value;option.textContent=label;select.append(option);};
    add('','Project root');
    const visit=nodes=>{for(const node of nodes)if(node.children){add(node.path,node.path);visit(node.children);}};visit(tree);
    select.value=destination; if(select.selectedIndex<0) select.value='';
    $('#file-name').value=''; const alternatives=$('#file-dialog').querySelector('.file-alternatives') as HTMLDetailsElement | null; if(alternatives) alternatives.open=false; fileNameError.hidden=true; $('#file-name').removeAttribute('aria-invalid'); $('#file-dialog').dataset.projectId=id; $('#file-dialog').returnValue='';
    $('#file-dialog').showModal(); $('#file-name').focus();
  } catch(error) { report('load folders',error); }
}
$('#add-file').onclick = () => openAddFile();
$('#file-dialog').onclose = async () => {
  const id=$('#file-dialog').dataset.projectId, destination=$('#file-destination').value;
  if (!id || active?.id!==id) return;
  if ($('#file-dialog').returnValue === 'upload-folder') {
    try { await store.importFolder(id,destination); if (active?.id === id) { await refreshFiles(); revision++; scheduleCompile(); } }
    catch (error) { if(active?.id===id) await refreshFiles().catch(()=>{}); report('upload folder', error); }
    return;
  }
  if ($('#file-dialog').returnValue === 'upload') {
    try { await store.importFiles(id,destination); if (active?.id === id) { await refreshFiles(); revision++; scheduleCompile(); } }
    catch (error) { if(active?.id===id) await refreshFiles().catch(()=>{}); report('upload files', error); }
    return;
  }
  const action = $('#file-dialog').returnValue;
  if (action !== 'create' && action !== 'folder') return;
  const input = $('#file-name').value.trim(); if (!input) return;
  const name=destination ? `${destination}/${input}` : input;
  if (action === 'folder') { try { await store.createFolder(id, name); if(active?.id===id) await refreshFiles(); } catch (error) { report('create folder', error); } return; }
  try { await store.createFile(id, name); if(active?.id===id) {await refreshFiles(); await openFile(name);} }
  catch (error) { report('create file', error); }
};
$('#project-name').onclick = () => { if (!active) return; $('#rename-name').value = active.name; $('#rename-dialog').returnValue = ''; $('#rename-dialog').showModal(); $('#rename-name').select(); };
// Text fields submit the primary action rather than the first button in DOM order.
for (const [fieldId, dialogId, action] of [
  ['rename-name', 'rename-dialog', 'rename'], ['file-name', 'file-dialog', 'create'],
  ['rename-file-name', 'rename-file-dialog', 'rename'], ['folder-move-path', 'folder-move-dialog', 'move']
] as const) {
  document.getElementById(fieldId)!.addEventListener('keydown', (event: KeyboardEvent) => {
    if (event.key !== 'Enter' || event.isComposing || event.keyCode === 229) return;
    event.preventDefault();
    const form = document.querySelector<HTMLFormElement>(`#${dialogId} form`)!;
    form.requestSubmit(form.querySelector<HTMLButtonElement>(`button[value="${action}"]`)!);
  });
}
$('#rename-dialog').onclose = async () => {
  if ($('#rename-dialog').returnValue !== 'rename' || !active) return;
  const id = active.id;
  try { const result = await store.rename(id, $('#rename-name').value); if (active?.id === id) { active = result; $('#project-name').textContent = result.name; } }
  catch (error) { report('rename project', error); }
};
async function refreshChats() {
  if (!active) return;
  const id = active.id, chats = await store.chatList(id);
  if (active?.id !== id) return;
  $('#chat-list').replaceChildren(...chats.map(chat => {
    const button = document.createElement('button'); button.textContent = chat.title; button.className='chat-open'; if(chat.id === currentChat) button.setAttribute('aria-current','true');
    button.onclick = async () => {
      const request=++chatOpening;chatLoading=true;$('#send').disabled=true;
      try { const data = await store.chatLoad(id, chat.id); if (active?.id === id && request===chatOpening) { saveChatDraft();currentChat = chat.id;restoreChatDraft(id,currentChat);renderChat(data);updateChatProgress(); void refreshChats(); } }
      catch (error) { if(active?.id===id && request===chatOpening) report('open chat', error); }
      finally {if(active?.id===id && request===chatOpening){chatLoading=false;updateSendState();}}
    };
    const row = document.createElement('div'); row.className = 'chat-row';
    const rename = document.createElement('button'); rename.className = 'chat-rename'; rename.textContent = '✎'; rename.setAttribute('aria-label', `Rename ${chat.title}`); rename.title = 'Rename chat';
    rename.onclick = () => {
      const input = document.createElement('input'); input.value = chat.title; input.maxLength = 100; input.setAttribute('aria-label', 'Chat name');
      row.replaceChildren(input); input.focus(); input.select();
      let finished = false;
      const finish = async (save: boolean) => {
        if (finished) return; finished = true;
        try { if (save && input.value.trim() && input.value.trim() !== chat.title) await store.chatRename(id, chat.id, input.value); }
        catch (error) { report('rename chat', error); }
        if (active?.id === id) await refreshChats();
      };
      input.onkeydown = event => { if (event.key === 'Enter') { event.preventDefault(); void finish(true); } else if (event.key === 'Escape') void finish(false); };
      input.onblur = () => { void finish(true); };
    };
    row.append(button, rename); return row;
  }));
}
$('#new-chat').onclick = async () => {
  if (!active) return; const id = active.id,request=++chatOpening;chatLoading=true;$('#send').disabled=true;
  try { const chatId = await store.chatCreate(id),chat=await store.chatLoad(id,chatId); if (active?.id === id && request===chatOpening) { saveChatDraft();currentChat = chatId;restoreChatDraft(id,currentChat);renderChat(chat);updateChatProgress();await refreshChats(); } }
  catch (error) { if(active?.id===id && request===chatOpening) report('create chat', error); }
  finally {if(active?.id===id && request===chatOpening){chatLoading=false;updateSendState();}}
};
$('#import').onclick = async () => { try { const project = await store.importProject(); if (project) { projects = await store.list(); renderProjects(); await openProject(project); } } catch (error) { report('import project', error); } };
$('#export-project').onclick = () => { if (active) store.exportProject(active.id).catch(error => report('export project', error)); };
function highlightLatex(text) {
  const fragment = document.createDocumentFragment();
  // Match escaped special characters before comments so escaped percent signs stay visible.
  const tokens = /\\(?:[a-zA-Z@]+\*?|[^a-zA-Z\s])|%[^\n]*|\$\$|\$|[{}\[\]]/g;
  let end = 0, match;
  while ((match = tokens.exec(text))) {
    fragment.append(document.createTextNode(text.slice(end, match.index)));
    const span = document.createElement('span');
    span.className = match[0][0] === '%' ? 'tex-comment' : match[0][0] === '\\' ? 'tex-command' : match[0][0] === '$' ? 'tex-math' : 'tex-brace';
    span.textContent = match[0]; fragment.append(span); end = tokens.lastIndex;
  }
  fragment.append(document.createTextNode(text.slice(end) + '\n'));
  $('#highlight').replaceChildren(fragment);
}
// Search within the open text file. Replacements use the editor's normal input
// path so saving, comment anchors, compilation and undo remain consistent.
const findPanel = document.createElement('div');
findPanel.className = 'editor-find'; findPanel.hidden = true;
findPanel.innerHTML = '<input aria-label="Find in file" placeholder="Find"><input aria-label="Replace with" placeholder="Replace"><label><input type="checkbox" aria-label="Match case" checked> Match case</label><label><input type="checkbox" aria-label="Whole word"> Whole word</label><button type="button" aria-label="Previous match">↑</button><button type="button" aria-label="Next match">↓</button><button type="button" data-replace>Replace</button><button type="button" data-all>All</button><span role="status" aria-live="polite"></span><button type="button" aria-label="Close find">×</button>';
const wrapLabel=document.createElement('label');wrapLabel.innerHTML='<input type="checkbox" aria-label="Wrap search" checked> Wrap';
findPanel.insertBefore(wrapLabel,findPanel.querySelector('[aria-label="Previous match"]'));
$('.editor-wrap').before(findPanel);
findPanel.setAttribute('role','search');findPanel.setAttribute('aria-label','Find and replace in current file');
const findInput = findPanel.querySelector<HTMLInputElement>('input[aria-label="Find in file"]')!;
const replaceInput = findPanel.querySelector<HTMLInputElement>('input[aria-label="Replace with"]')!;
const matchCase = findPanel.querySelector<HTMLInputElement>('input[aria-label="Match case"]')!;
const wholeWord = findPanel.querySelector<HTMLInputElement>('input[aria-label="Whole word"]')!;
const wrapSearch = findPanel.querySelector<HTMLInputElement>('input[aria-label="Wrap search"]')!;
const findStatus = findPanel.querySelector<HTMLElement>('[role="status"]')!;
const sourceEditor = $('#source') as HTMLTextAreaElement;
let findCache: {content:string;query:string;matchCase:boolean;wholeWord:boolean;positions:number[]} | undefined;
let findAnchor=0;
function openFind() {
  if (!active || $('.source-container').hidden) return;
  const selected=sourceEditor.value.slice(sourceEditor.selectionStart,sourceEditor.selectionEnd);
  if (document.activeElement===sourceEditor && selected && !selected.includes('\n')) findInput.value=selected;
  findAnchor=sourceEditor.selectionStart;
  findPanel.hidden=false;findInput.focus();findInput.select();refreshFind();
}
function closeFind() {findPanel.hidden=true;sourceEditor.focus();}
function selectFindMatch(position:number) {
  sourceEditor.setSelectionRange(position,position+findInput.value.length);
  // Measure using the editor's actual font and tab stops, including long lines.
  const style=getComputedStyle(sourceEditor),prefix=sourceEditor.value.slice(0,position);
  const measure=document.createElement('span');
  measure.style.cssText='position:fixed;visibility:hidden;white-space:pre;';
  measure.style.font=style.font;measure.style.tabSize=style.tabSize;
  measure.textContent=prefix.slice(prefix.lastIndexOf('\n')+1);document.body.append(measure);
  const x=measure.getBoundingClientRect().width;measure.remove();
  const y=(prefix.split('\n').length-1)*parseFloat(style.lineHeight);
  sourceEditor.scrollTop=Math.max(0,y-sourceEditor.clientHeight/2+parseFloat(style.paddingTop));
  sourceEditor.scrollLeft=Math.max(0,x-sourceEditor.clientWidth/2+parseFloat(style.paddingLeft));
  syncEditorScroll();updateFindStatus();
}
function refreshFind() {
  const positions=findPositions();
  if(!positions.length) {updateFindStatus();return;}
  const position=positions.find(position=>position>=findAnchor) ?? (wrapSearch.checked ? positions[0] : undefined);
  if(position===undefined) {findStatus.textContent='End of file';return;}
  selectFindMatch(position);
}
function findPositions(): number[] {
  const query=findInput.value, content=sourceEditor.value, positions: number[]=[];
  if(findCache && findCache.content===content && findCache.query===query && findCache.matchCase===matchCase.checked && findCache.wholeWord===wholeWord.checked) return findCache.positions;
  findCache={content,query,matchCase:matchCase.checked,wholeWord:wholeWord.checked,positions};
  if (!query) return positions;
  // RegExp's Unicode case folding preserves source offsets, unlike lowercasing
  // the entire document, which can change its length for some characters.
  const escaped=query.replace(/[.*+?^${}()|[\]\\]/g,'\\$&');
  const expression=new RegExp(escaped,matchCase.checked?'gu':'giu');
  const isWord = (character: string) => /[\p{L}\p{M}\p{N}\p{Pc}]/u.test(character);
  const before = (index: number): string => {
    if(index===0) return '';
    const last=content.charCodeAt(index-1), offset=last>=0xDC00 && last<=0xDFFF && index>1 ? index-2 : index-1;
    return String.fromCodePoint(content.codePointAt(offset)!);
  };
  let match:RegExpExecArray|null;
  while((match=expression.exec(content))) {
    const start=match.index,end=start+match[0].length;
    const preceding=before(start);
    const following=end<content.length ? String.fromCodePoint(content.codePointAt(end)!) : '';
    if(!wholeWord.checked || (!isWord(preceding) && !isWord(following))) positions.push(start);
  }
  return positions;
}
function updateFindStatus() {
  const positions=findPositions(), index=positions.findIndex(position=>position===sourceEditor.selectionStart && sourceEditor.selectionEnd===position+findInput.value.length);
  findStatus.textContent=positions.length ? `${index<0 ? '–' : index+1} / ${positions.length}` : findInput.value ? 'No matches' : 'Type to search';
}
function findNext(backward=false) {
  const positions=findPositions();
  if (!positions.length) {updateFindStatus();return;}
  const start=sourceEditor.selectionStart, end=sourceEditor.selectionEnd;
  const position=backward ? [...positions].reverse().find(p=>p<start) ?? (wrapSearch.checked ? positions.at(-1) : undefined) : positions.find(p=>p>=end) ?? (wrapSearch.checked ? positions[0] : undefined);
  if(position===undefined) {findStatus.textContent=backward ? 'Start of file' : 'End of file';return;}
  selectFindMatch(position);
}
function replaceMatches(all: boolean) {
  if (sourceEditor.readOnly || $('.source-container').hidden || !findInput.value) return;
  const positions=findPositions();if (!positions.length) {updateFindStatus();return;}
  const start=sourceEditor.selectionStart, end=sourceEditor.selectionEnd;
  if (!all && !(positions.includes(start) && end-start===findInput.value.length)) {findNext();return;}
  const focus=document.activeElement;
  sourceEditor.focus();
  if (all) {
    const value=sourceEditor.value, queryLength=findInput.value.length;
    let previous=0, replacement='';
    for (const position of positions) {replacement+=value.slice(previous,position)+replaceInput.value;previous=position+queryLength;}
    replacement+=value.slice(previous);sourceEditor.select();document.execCommand('insertText',false,replacement);
  } else document.execCommand('insertText',false,replaceInput.value);
  if(!all) findNext();else updateFindStatus();
  if(focus instanceof HTMLElement && findPanel.contains(focus)) focus.focus({preventScroll:true});
}
findInput.oninput=()=>{if(!composingFind) refreshFind();};
let composingFind=false;
findInput.addEventListener('compositionstart',()=>{composingFind=true;});
findInput.addEventListener('compositionend',()=>{composingFind=false;refreshFind();});
matchCase.onchange=refreshFind;
wholeWord.onchange=refreshFind;
sourceEditor.addEventListener('select',()=>{if(!findPanel.hidden)updateFindStatus();});
sourceEditor.addEventListener('input',()=>{if(!findPanel.hidden) updateFindStatus();});
findPanel.querySelector<HTMLButtonElement>('[aria-label="Previous match"]')!.onclick=()=>findNext(true);
findPanel.querySelector<HTMLButtonElement>('[aria-label="Next match"]')!.onclick=()=>findNext();
findPanel.querySelector<HTMLButtonElement>('[data-replace]')!.onclick=()=>replaceMatches(false);
findPanel.querySelector<HTMLButtonElement>('[data-all]')!.onclick=()=>replaceMatches(true);
findPanel.querySelector<HTMLButtonElement>('[aria-label="Close find"]')!.onclick=closeFind;
findPanel.addEventListener('keydown',event=>{
  if(event.isComposing || composingFind) return;
  if(event.key==='Escape'){event.preventDefault();closeFind();}
  else if(event.key==='Enter' && (event.target===findInput || event.target===replaceInput)) {
    event.preventDefault();
    if(event.target===replaceInput) replaceMatches(false);else findNext(event.shiftKey);
  }
});
document.addEventListener('keydown',event=>{
  if(event.isComposing) return;
  if ((event.ctrlKey || event.metaKey) && !event.altKey && event.key.toLowerCase()==='f' && active && !$('.source-container').hidden && displayedPaneLayout().editor) {
    event.preventDefault();openFind();
  } else if(!findPanel.hidden && event.key==='F3') {event.preventDefault();findNext(event.shiftKey);}
  else if(!findPanel.hidden && event.key==='Escape' && event.target===sourceEditor) {event.preventDefault();closeFind();}
});
const findButton=document.createElement('button');findButton.type='button';findButton.className='plain';findButton.textContent='Find';findButton.title='Find in file (Ctrl+F)';
findButton.onclick=openFind;
$('.editor-header').insertBefore(findButton,$('#tools'));
// Explicit completion keeps normal typing and indentation unchanged until requested.
const latexCompletions=[
  ['begin','\\begin{}',7],['end','\\end{}',5],['section','\\section{}',9],['subsection','\\subsection{}',12],
  ['textbf','\\textbf{}',8],['textit','\\textit{}',8],['emph','\\emph{}',6],['cite','\\cite{}',6],
  ['ref','\\ref{}',5],['label','\\label{}',7],['includegraphics','\\includegraphics{}',17],['frac','\\frac{}{}',6],
  ['sqrt','\\sqrt{}',6],['item','\\item ',6],['usepackage','\\usepackage{}',12],
  ['documentclass','\\documentclass{}',15],['subsubsection','\\subsubsection{}',15],
  ['paragraph','\\paragraph{}',11],['chapter','\\chapter{}',9],['caption','\\caption{}',9],
  ['footnote','\\footnote{}',10],['url','\\url{}',5],['href','\\href{}{}',6],
  ['input','\\input{}',7],['include','\\include{}',9],['bibliography','\\bibliography{}',14],
  ['texttt','\\texttt{}',8],['underline','\\underline{}',11],['mathrm','\\mathrm{}',8],
  ['mathbf','\\mathbf{}',8],['mathit','\\mathit{}',8],['sum','\\sum',4],['int','\\int',4],
  ['alpha','\\alpha',6],['beta','\\beta',5],['gamma','\\gamma',6],['delta','\\delta',6],
  ['infty','\\infty',6],['times','\\times',6],['cdot','\\cdot',5],['left','\\left',5],['right','\\right',6]
] as const;
const completionList=document.createElement('div');completionList.id='latex-completions';completionList.className='latex-completions';completionList.setAttribute('role','listbox');completionList.setAttribute('aria-label','LaTeX command suggestions');completionList.hidden=true;$('.editor-pane').append(completionList);
const completionButton=document.createElement('button');completionButton.type='button';completionButton.className='plain';completionButton.textContent='Commands';completionButton.title='Complete LaTeX command (Ctrl+Space)';completionButton.setAttribute('aria-keyshortcuts','Control+Space');$('.editor-header').insertBefore(completionButton,findButton);
sourceEditor.setAttribute('aria-autocomplete','list');sourceEditor.setAttribute('aria-controls',completionList.id);sourceEditor.setAttribute('aria-expanded','false');
let completionOpen=false,completionIndex=0,completionStart=0,completionEnd=0,completionMatches: Array<(typeof latexCompletions)[number]>=[];
function closeCompletions() {completionOpen=false;completionList.hidden=true;sourceEditor.setAttribute('aria-expanded','false');sourceEditor.removeAttribute('aria-activedescendant');}
function drawCompletions() {
  completionList.replaceChildren(...completionMatches.map((item,index)=>{const option=document.createElement('button');option.type='button';option.id=`latex-suggestion-${index}`;option.setAttribute('role','option');option.setAttribute('aria-selected',String(index===completionIndex));option.textContent='\\'+item[0];option.onmousedown=event=>event.preventDefault();option.onclick=()=>chooseCompletion(index);return option;}));
  sourceEditor.setAttribute('aria-activedescendant',`latex-suggestion-${completionIndex}`);
  completionList.querySelector<HTMLElement>(`#latex-suggestion-${completionIndex}`)?.scrollIntoView({block:'nearest'});
}
function positionCompletions():void {
  if(completionList.hidden) return;
  const editorBounds=sourceEditor.getBoundingClientRect(), paneBounds=$('.editor-pane').getBoundingClientRect(), style=getComputedStyle(sourceEditor);
  const before=sourceEditor.value.slice(0,sourceEditor.selectionStart), lines=before.split('\n'), line=lines[lines.length-1].replace(/\t/g,'    ');
  const canvas=document.createElement('canvas'), context=canvas.getContext('2d'); if(!context) return;
  context.font=`${style.fontStyle} ${style.fontWeight} ${style.fontSize} ${style.fontFamily}`;
  const lineHeight=parseFloat(style.lineHeight) || 24, left=editorBounds.left-paneBounds.left+parseFloat(style.paddingLeft)+context.measureText(line).width-sourceEditor.scrollLeft;
  const top=editorBounds.top-paneBounds.top+parseFloat(style.paddingTop)+(lines.length-1)*lineHeight-sourceEditor.scrollTop;
  const width=completionList.offsetWidth, height=completionList.offsetHeight, pane=$('.editor-pane') as HTMLElement;
  completionList.style.right='auto'; completionList.style.left=`${Math.max(4,Math.min(left,pane.clientWidth-width-4))}px`;
  completionList.style.top=`${Math.max(4,Math.min(top+lineHeight,pane.clientHeight-height-4))}px`;
}
function updateCompletions(allowEmpty=false) {
  if(!completionOpen)return;
  if(sourceEditor.selectionStart!==sourceEditor.selectionEnd){closeCompletions();return;}
  const caret=sourceEditor.selectionStart,match=sourceEditor.value.slice(0,caret).match(/\\[a-zA-Z]*$/),prefix=match ? match[0].slice(1).toLowerCase() : '';
  if(!match && !allowEmpty){closeCompletions();return;}
  completionStart=match ? caret-match[0].length : caret;
  completionEnd=match ? caret+(sourceEditor.value.slice(caret).match(/^[a-zA-Z]*/)?.[0].length ?? 0) : caret;
  completionMatches=latexCompletions.filter(item=>item[0].startsWith(prefix));
  if(!completionMatches.length){closeCompletions();return;}
  completionIndex=Math.min(completionIndex,completionMatches.length-1);completionList.hidden=false;sourceEditor.setAttribute('aria-expanded','true');drawCompletions();positionCompletions();
}
function chooseCompletion(index:number) {
  const item=completionMatches[index];if(!item || !completionOpen)return;
  const start=completionStart,end=completionEnd;closeCompletions();sourceEditor.focus();sourceEditor.setSelectionRange(start,end);
  document.execCommand('insertText',false,item[1]);sourceEditor.setSelectionRange(start+item[2],start+item[2]);
}
completionButton.onclick=()=>{if(!active || $('.source-container').hidden || sourceEditor.readOnly)return;sourceEditor.focus();completionOpen=true;completionIndex=0;updateCompletions(true);};
sourceEditor.addEventListener('input',()=>{if(completionOpen)updateCompletions();});
sourceEditor.addEventListener('scroll',()=>{if(completionOpen)positionCompletions();});
sourceEditor.addEventListener('blur',()=>closeCompletions());
sourceEditor.addEventListener('click',()=>closeCompletions());
sourceEditor.addEventListener('keydown',event=>{
  if (event.isComposing || event.keyCode === 229) return;
  if((event.ctrlKey || event.metaKey) && !event.altKey && event.code==='Space' && !sourceEditor.readOnly) {event.preventDefault();event.stopImmediatePropagation();completionOpen=true;completionIndex=0;updateCompletions(true);return;}
  if(!completionOpen)return;
  if(event.key==='Escape'){event.preventDefault();event.stopImmediatePropagation();closeCompletions();}
  else if(event.key==='ArrowDown' || event.key==='ArrowUp'){event.preventDefault();event.stopImmediatePropagation();completionIndex=(completionIndex+(event.key==='ArrowDown'?1:-1)+completionMatches.length)%completionMatches.length;drawCompletions();}
  else if(event.key==='Enter' || event.key==='Tab'){event.preventDefault();event.stopImmediatePropagation();chooseCompletion(completionIndex);}
  else if(['ArrowLeft','ArrowRight','Home','End','PageUp','PageDown'].includes(event.key))closeCompletions();
});
let releaseEditorTab = false;
sourceEditor.addEventListener('input', () => { releaseEditorTab = false; });
sourceEditor.addEventListener('blur', () => { releaseEditorTab = false; });
$('#source').addEventListener('keydown', (event: KeyboardEvent) => {
  if (event.isComposing || event.keyCode === 229) return;
  if (event.key === 'Escape') { releaseEditorTab = true; return; }
  if (event.key === 'Tab' && releaseEditorTab) { releaseEditorTab = false; return; }
  if (event.key !== 'Tab' || event.ctrlKey || event.altKey || event.metaKey || (event.target as HTMLTextAreaElement).readOnly) return;
  event.preventDefault();
  const editor=event.target as HTMLTextAreaElement, start=editor.selectionStart, end=editor.selectionEnd;
  // An unselected Tab inserts spaces at the caret; selected lines and Shift+Tab
  // operate on whole lines. insertText participates in Chromium's undo stack.
  if (!event.shiftKey && start===end) {document.execCommand('insertText',false,'  ');return;}
  const value=editor.value, first=start===0 ? 0 : value.lastIndexOf('\n',start-1)+1;
  const lastCharacter=end>start && value[end-1]==='\n' ? end-1 : end;
  const nextNewline=value.indexOf('\n',lastCharacter), last=nextNewline<0 ? value.length : nextNewline;
  const lines=value.slice(first,last).split('\n');
  const changes: {position:number; removed:number; added:number}[]=[];
  let offset=0;
  const replacement=lines.map(line=>{
    const removed=event.shiftKey ? (line.match(/^(?: {1,2}|\t)/)?.[0].length || 0) : 0;
    const added=event.shiftKey ? 0 : 2;
    changes.push({position:offset,removed,added});offset+=line.length+1;
    return ' '.repeat(added)+line.slice(removed);
  }).join('\n');
  if (replacement===value.slice(first,last)) return;
  const mapOffset=(position:number)=>{
    let shift=0;
    for(const change of changes) {
      if(position<change.position) break;
      if(position<change.position+change.removed) return change.position+shift+change.added;
      shift+=change.added-change.removed;
    }
    return position+shift;
  };
  const direction=editor.selectionDirection;
  editor.setSelectionRange(first,last);document.execCommand('insertText',false,replacement);
  editor.setSelectionRange(first+mapOffset(start-first),first+mapOffset(end-first),direction);
});
const expandChat = document.createElement('button'); expandChat.type = 'button'; expandChat.className = 'expand-chat'; expandChat.textContent = 'Expand chat'; expandChat.setAttribute('aria-pressed','false');
expandChat.onclick = () => {
  const expanded=!$('.editor-pane').classList.contains('chat-expanded');
  closeLayoutTools();paneLayout={...visiblePaneLayout(),agent:true,editor:!expanded};applyPaneLayout();
};
const chatTitle = document.createElement('strong'); chatTitle.className='chat-title'; chatTitle.textContent='Conversation';
$('.chat-header').prepend(chatTitle, expandChat);
$('#toggle-agent').onclick = () => togglePane('agent');
$('#collapse').onclick = () => { const sidebar = document.querySelector('.sidebar'); sidebar.classList.toggle('collapsed'); $('#collapse').setAttribute('aria-label', sidebar.classList.contains('collapsed') ? 'Expand sidebar' : 'Collapse sidebar'); };
$('#outline-toggle').onclick = () => { const list = $('#outline-list'); list.hidden = !list.hidden; $('#outline-toggle').setAttribute('aria-expanded', String(!list.hidden)); };
$('#grid-view').onclick = () => { $('#list').classList.add('grid'); $('#grid-view').classList.add('selected'); $('#list-view').classList.remove('selected'); $('#grid-view').setAttribute('aria-pressed','true'); $('#list-view').setAttribute('aria-pressed','false'); renderProjects(); };
$('#list-view').onclick = () => { $('#list').classList.remove('grid'); $('#list-view').classList.add('selected'); $('#grid-view').classList.remove('selected'); $('#list-view').setAttribute('aria-pressed','true'); $('#grid-view').setAttribute('aria-pressed','false'); renderProjects(); };
$('#zoom-fit').onclick = () => changePdfZoom(100,'width');
function attachmentPreview(item:{token:string;name:string}, projectId:string):HTMLElement {
  const container=document.createElement('span'); container.className='attachment-preview';
  if (!/\.(png|jpe?g|webp)$/i.test(item.token)) { container.textContent=item.name; return container; }
  const button=document.createElement('button'); button.type='button'; button.className='image-thumbnail'; button.setAttribute('aria-label',`Preview ${item.name}`);
  const image=document.createElement('img'); image.alt=item.name; button.append(image); container.append(button);
  void store.chatImagePreview(projectId,item.token).then((url:string)=>{image.src=url;button.onclick=()=>{const full=$('#chat-image-full') as HTMLImageElement;full.src=url;full.alt=item.name;($('#chat-image-dialog') as HTMLDialogElement).showModal();};}).catch(()=>{container.textContent=item.name;});
  return container;
}
$('#chat-image-close').onclick=()=>($('#chat-image-dialog') as HTMLDialogElement).close();
$('#chat-image-dialog').addEventListener('click',(event:MouseEvent)=>{if(event.target===event.currentTarget) ($('#chat-image-dialog') as HTMLDialogElement).close();});
function renderAttachments() {
  const pending=!!active && pendingAttachments.has(draftKey(active.id,currentChat));
  updateSendState();
  $('#attach').disabled=pending || attachments.length>=10;
  $('#attach').title=attachments.length>=10 ? 'Maximum of 10 attachments' : pending ? 'Selecting attachments' : 'Upload files & photos';
  $('#attachment-list').replaceChildren(...attachments.map((item, index) => {
    const wrapper=document.createElement('span'); wrapper.className='draft-attachment'; wrapper.append(attachmentPreview(item,active.id));
    const button=document.createElement('button'); button.type='button'; button.className='attachment-remove'; button.replaceChildren(actionIcon('close')); button.setAttribute('aria-label',`Remove ${item.name}`);
    button.onclick=()=>{attachments.splice(index,1);renderAttachments();saveChatDraft();}; wrapper.append(button); return wrapper;
  }));
}
$('#chat-input').addEventListener('paste', async (event: ClipboardEvent) => {
  const image=Array.from(event.clipboardData?.items || []).find(item=>item.type==='image/png');
  if(!image)return;
  event.preventDefault();
  if(!active)return;
  const id=active.id,chatId=currentChat,key=draftKey(id,chatId);
  if(attachments.length>=10) {report('paste image',new Error('Maximum of 10 attachments'));return;}
  if(pendingAttachments.has(key)) {report('paste image',new Error('An attachment is already being added'));return;}
  const file=image.getAsFile();if(!file)return;
  pendingAttachments.add(key);renderAttachments();
  try {
    const added=await store.chatPasteImage(id,new Uint8Array(await file.arrayBuffer()));
    if(active?.id===id && currentChat===chatId) {attachments.push(added);renderAttachments();saveChatDraft();}
    else {const draft=readChatDraft(key);draft.attachments.push(added);writeChatDraft(key,draft);}
  } catch(error) {report('paste image',error);}
  finally {pendingAttachments.delete(key);if(active?.id===id && currentChat===chatId)renderAttachments();}
});
$('#attach').onclick = async () => {
  if (!active || attachments.length>=10) return; const id = active.id,chatId=currentChat,key=draftKey(id,chatId),remaining=10-attachments.length;
  if(pendingAttachments.has(key))return;pendingAttachments.add(key);renderAttachments();
  try { const added = await store.chatAttach(id,remaining); if (active?.id === id && currentChat===chatId) { attachments.push(...added); renderAttachments();saveChatDraft(); } else {const draft=readChatDraft(key);draft.attachments.push(...added);writeChatDraft(key,draft);} }
  catch (error) { report('attach file', error); }
  finally {pendingAttachments.delete(key);if(active?.id===id && currentChat===chatId)renderAttachments();}
};
function renderToolPanel() {
  const body = $('#tool-body'); body.classList.remove('has-open-comparison'); body.replaceChildren();
  $('#tool-info').classList.toggle('active', selectedTool === 'info'); $('#tool-info').setAttribute('aria-pressed',String(selectedTool === 'info'));
  $('#tool-versions').classList.toggle('active', selectedTool === 'versions'); $('#tool-versions').setAttribute('aria-pressed',String(selectedTool === 'versions'));
  $('#tool-comments').classList.toggle('active', selectedTool === 'comments'); $('#tool-comments').setAttribute('aria-pressed',String(selectedTool === 'comments'));
  $('#tool-logs').classList.toggle('active', selectedTool === 'logs'); $('#tool-logs').setAttribute('aria-pressed',String(selectedTool === 'logs'));
  if (selectedTool === 'versions') { renderVersions(); return; }
  if (selectedTool === 'comments') { renderComments(); return; }
  if (selectedTool === 'logs') {
    const label = document.createElement('div'); label.textContent = 'Build output'; label.className = 'tool-heading';
    const log = document.createElement('pre'); log.className = 'tool-log';
    let errors = 0, warnings = 0; const diagnostics: { text: string; kind: 'error' | 'warning' }[] = [];
    for (const line of compileLog.split('\n')) { const item = document.createElement('span'); item.className = /^!|(?:^|:)\s*error\b/i.test(line) ? 'log-error' : /\bwarning\b/i.test(line) ? 'log-warning' : ''; if(item.className === 'log-error') errors++; if(item.className === 'log-warning') warnings++; if(item.className && diagnostics.length < 20) diagnostics.push({ text: line, kind: item.className === 'log-error' ? 'error' : 'warning' }); item.textContent = line + '\n'; log.append(item); }
    const summary = document.createElement('p'); summary.className = 'log-summary'; summary.textContent = compileLog ? `${errors} detected ${errors === 1 ? 'error line' : 'error lines'} · ${warnings} detected ${warnings === 1 ? 'warning line' : 'warning lines'}` : 'No build output yet.';
    const wrap = document.createElement('button'); wrap.className = 'secondary log-wrap'; wrap.textContent = 'Disable line wrap'; wrap.setAttribute('aria-pressed','true');
    wrap.onclick = () => { const enabled = wrap.getAttribute('aria-pressed') === 'true'; wrap.setAttribute('aria-pressed',String(!enabled)); wrap.textContent = enabled ? 'Enable line wrap' : 'Disable line wrap'; log.classList.toggle('unwrapped', enabled); };
    const diagnosticList = document.createElement('div'); diagnosticList.className = 'log-diagnostics';
    const uniqueDiagnostics = diagnostics.filter((item, index) => diagnostics.findIndex(other => other.kind === item.kind && other.text === item.text) === index);
    if (uniqueDiagnostics.length < diagnostics.length) { const note = document.createElement('p'); note.className='diagnostic-note'; note.textContent=`Showing ${uniqueDiagnostics.length} distinct diagnostic ${uniqueDiagnostics.length === 1 ? 'message' : 'messages'}; repeated lines remain in full output.`; diagnosticList.append(note); }
    for (const diagnostic of uniqueDiagnostics) { const item = document.createElement('p'); item.className = `diagnostic-${diagnostic.kind}`; item.textContent = diagnostic.text; diagnosticList.append(item); }
    const logSearch = document.createElement('input'); logSearch.type='search'; logSearch.className='log-search'; logSearch.placeholder='Filter output lines'; logSearch.setAttribute('aria-label','Filter build output');
    const logCount = document.createElement('span'); logCount.className='log-filter-count'; logCount.setAttribute('role','status'); let filterTimer: ReturnType<typeof setTimeout> | null = null;
    const logEmpty = document.createElement('p'); logEmpty.className='log-filter-empty'; logEmpty.textContent='No output lines match these filters.'; logEmpty.hidden=true;
    const issuesOnly=document.createElement('button'); issuesOnly.type='button'; issuesOnly.className='secondary log-issues'; issuesOnly.textContent='Issues only'; issuesOnly.setAttribute('aria-pressed','false');
    const applyFilter=()=>{const query=logSearch.value.trim().toLowerCase(), onlyIssues=issuesOnly.getAttribute('aria-pressed')==='true'; let shown=0; for(const line of Array.from(log.children)) { const match=(!query || (line.textContent || '').toLowerCase().includes(query)) && (!onlyIssues || line.classList.contains('log-error') || line.classList.contains('log-warning')); (line as HTMLElement).hidden=!match; if(match) shown++; } logCount.textContent=query || onlyIssues ? `${shown} matching ${shown === 1 ? 'line' : 'lines'}` : ''; logEmpty.hidden=shown>0 || !(query || onlyIssues); log.hidden=!logEmpty.hidden; };
    logSearch.oninput = () => { if(filterTimer) clearTimeout(filterTimer); filterTimer=setTimeout(applyFilter,150); };
    issuesOnly.onclick=()=>{issuesOnly.setAttribute('aria-pressed',String(issuesOnly.getAttribute('aria-pressed')!=='true')); applyFilter();};
    const details = document.createElement('details'); details.className = 'raw-output'; const disclosure = document.createElement('summary'); disclosure.textContent = 'Full build output'; const controls = document.createElement('div'); controls.className='log-controls'; controls.append(logSearch,issuesOnly,wrap,logCount); details.append(disclosure, controls, logEmpty, log);
    let expandedForOutput = false;
    details.ontoggle = () => { if(!details.isConnected) return; if(details.open && window.matchMedia('(min-width: 1201px)').matches && !$('#workspace').classList.contains('tools-expanded')) { expandedForOutput=true; $('#workspace').classList.add('tools-expanded'); expandTools.textContent='Collapse panel'; expandTools.setAttribute('aria-label','Collapse tools panel'); } else if(!details.open && expandedForOutput) { expandedForOutput=false; $('#workspace').classList.remove('tools-expanded'); expandTools.textContent='Expand panel'; expandTools.setAttribute('aria-label','Expand tools panel'); } };
    body.append(label, summary, diagnosticList, details);
  } else if (active) {
    const title = document.createElement('div'); title.className = 'tool-heading'; title.textContent = 'Project Info';
    const facts = document.createElement('dl'); facts.className = 'project-facts';
    for (const [label, value] of [['Name', active.name], ['Root file', active.rootFile || 'main.tex'], ['Created', new Date(active.created).toLocaleDateString()]]) {
      const row = document.createElement('div'), term = document.createElement('dt'), description = document.createElement('dd');
      term.textContent = label; if (label === 'Root file') { const link = document.createElement('button'); link.className='fact-link'; link.textContent=value; link.onclick=()=>void openFile(value); description.append(link); } else description.textContent = value; row.append(term, description); facts.append(row);
    }
    const engineLabel=document.createElement('label'); engineLabel.textContent='LaTeX engine ';
    const engine=document.createElement('select'); engine.setAttribute('aria-label','LaTeX engine');
    for(const [value,label] of [['pdflatex','pdfLaTeX'],['xelatex','XeLaTeX'],['lualatex','LuaLaTeX']]) { const option=document.createElement('option'); option.value=value; option.textContent=label; engine.append(option); }
    engine.value=active.engine || 'pdflatex';
    engine.onchange=async()=>{
      if(!active) return; const id=active.id, previous=active.engine || 'pdflatex', selected=engine.value;
      engine.disabled=true; revision++; scheduleCompile(); clearTimeout(compileTimer);
      try { const meta=await store.setEngine(id,selected); if(active?.id===id) {active=meta; scheduleCompile();} }
      catch(error) { if(active?.id===id) {engine.value=previous; report('change LaTeX engine',error); scheduleCompile();} }
      finally {engine.disabled=false;}
    };
    const draftLabel=document.createElement('label'); draftLabel.className='engine-setting';
    const draft=document.createElement('input'); draft.type='checkbox'; draft.id='draft-mode'; draft.checked=active.draftMode === true; draft.disabled=pendingDraftSaves.has(active.id);
    draftLabel.append(draft,document.createTextNode(' Draft mode (image placeholders for faster compilation)'));
    draft.onchange=async()=>{
      if(!active) return; const id=active.id, previous=active.draftMode === true;
      pendingDraftSaves.add(id); draft.disabled=true; revision++; scheduleCompile(); clearTimeout(compileTimer);
      try {const meta=await store.setDraftMode(id,draft.checked);
        for(const project of projects) if(project.id===id) project.draftMode=meta.draftMode;
        if(active?.id===id) {active.draftMode=meta.draftMode; scheduleCompile();}
      }
      catch(error) {if(active?.id===id) {draft.checked=previous; report('change draft mode',error); scheduleCompile();}}
      finally {pendingDraftSaves.delete(id); draft.disabled=false; if(active?.id===id) {const displayed=document.querySelector<HTMLInputElement>('#draft-mode'); if(displayed) {displayed.checked=active.draftMode === true; displayed.disabled=false;}}}
    };

    engineLabel.className = 'engine-setting'; const engineHint = document.createElement('small'); engineHint.id='engine-hint'; engineHint.textContent='Choose the compiler used to build this project.'; engine.setAttribute('aria-describedby',engineHint.id); engineLabel.append(engine,engineHint); body.append(title, facts, engineLabel, draftLabel);
  }
}
const toolsButton = document.querySelector<HTMLButtonElement>('#tools')!;
toolsButton.append(actionIcon('wrench'));
toolsButton.onclick = () => {
  showingTools = !showingTools;
  $('#tools-panel').hidden = !showingTools; $('#pdf-content').hidden = showingTools;
  $('#workspace').classList.remove('tools-expanded');
  expandTools.textContent='Expand panel';expandTools.setAttribute('aria-label','Expand tools panel');
  toolsButton.setAttribute('aria-expanded', String(showingTools));
  applyPaneLayout(false);
  if(showingTools) renderToolPanel();
};
const expandTools = document.createElement('button'); expandTools.type='button'; expandTools.className='expand-tools'; expandTools.textContent='Expand panel'; expandTools.setAttribute('aria-label','Expand tools panel');
expandTools.onclick = () => { const expanded = $('#workspace').classList.toggle('tools-expanded'); expandTools.textContent=expanded ? 'Collapse panel' : 'Expand panel'; expandTools.setAttribute('aria-label',expanded ? 'Collapse tools panel' : 'Expand tools panel'); };
$('.tool-tabs').append(expandTools);
$('#tool-info').onclick = () => { selectedTool = 'info'; renderToolPanel(); };
$('#tool-logs').onclick = () => { selectedTool = 'logs'; renderToolPanel(); };

// Compare ordered lines so repeated or moved lines are not mistaken for unchanged text.
function comparisonLines(previous: string, current: string): [{text:string;changed:boolean}[], {text:string;changed:boolean}[]] {
  const left=previous.split('\n'), right=current.split('\n');
  const oldRows=left.map(text=>({text,changed:true})), newRows=right.map(text=>({text,changed:true}));
  if (left.length * right.length > 1_000_000) return [oldRows,newRows];
  const width=right.length+1, lengths=new Uint32Array((left.length+1)*width);
  for(let i=left.length-1;i>=0;i--) for(let j=right.length-1;j>=0;j--) lengths[i*width+j]=left[i]===right[j] ? 1+lengths[(i+1)*width+j+1] : Math.max(lengths[(i+1)*width+j],lengths[i*width+j+1]);
  let i=0,j=0;
  while(i<left.length && j<right.length) {
    if(left[i]===right[j]) {oldRows[i].changed=false;newRows[j].changed=false;i++;j++;}
    else if(lengths[(i+1)*width+j]>=lengths[i*width+j+1]) i++; else j++;
  }
  return [oldRows,newRows];
}
async function renderVersions() {
  if (!active) return; const id = active.id;
  try {
    const versions = await store.versionList(id);
    if (active?.id !== id || !showingTools || selectedTool !== 'versions') return;
    const body = document.getElementById('tool-body')!, heading = document.createElement('div'); body.classList.remove('has-open-comparison'); heading.className = 'tool-heading'; heading.textContent = 'Versions'; body.replaceChildren(heading);
    for (const [index, version] of versions.entries()) {
      const row = document.createElement('div'); row.className = 'version-row';
      const label = document.createElement('span'); label.textContent = `v${versions.length - index} \u00b7 ${new Date(version.created).toLocaleString()} · ${version.files.length} ${version.files.length === 1 ? 'file' : 'files'}`;
      const restore = document.createElement('button'); restore.textContent = 'Restore file';
      restore.disabled = !version.files.includes(currentFile) || chatBusy.has(id) || !!restoringProject;
      restore.onclick = async () => {
        const file = currentFile;
        if (!confirm(`Restore ${file} from this version?`)) return;
        try { await store.versionRestoreFile(id, version.id, file); if (active?.id === id) { await openFile(file); revision++; scheduleCompile(); await renderVersions(); } }
        catch (error) { report('restore file', error); }
      };
      const restoreProject = document.createElement('button'); restoreProject.textContent = 'Restore project';
      restoreProject.disabled = chatBusy.has(id) || !!restoringProject;
      restoreProject.onclick = async () => {
        if (restoringProject || chatBusy.has(id)) return;
        if (!confirm('Restore all project files from this version? The current project will be saved as a version first.')) return;
        restoringProject = id; setEditorReadOnly(true); revision++; scheduleCompile(); clearTimeout(compileTimer);
        try {
          const saveResults = await Promise.all([...(pendingSaves.get(id)?.values() || [])]);
          if (saveResults.some(Boolean)) throw new Error('Save the pending changes before restoring.');
          const {project:meta,cleanupPending} = await store.versionRestoreProject(id, version.id);
          if (active?.id === id) {
            active = meta; openFiles = []; clearEditor(); await refreshFiles();
            const files = await store.files(id), preferred = meta.rootFile || 'main.tex';
            if (active?.id === id) { if (files.includes(preferred)) await openFile(preferred); else if (files.length) await openFile(files[0]); else clearEditor(); }
          }
          if (cleanupPending) {if(active?.id===id)showRestoreNotice(true);alert('The project was restored, but temporary restore data remains in the project folder because it could not be safely cleaned.');}
        } catch (error) { if (active?.id === id) report('restore project', error); }
        finally {
          restoringProject = null;
          if (active?.id === id) { setEditorReadOnly(chatBusy.has(id)); compileQueued = false; scheduleCompile(); if (showingTools && selectedTool === 'versions') await renderVersions(); }
        }
      };
      const compare = document.createElement('details'); compare.className = 'version-comparison';
      const summary = document.createElement('summary'); summary.textContent = 'Compare with current';
      const changes = document.createElement('div'); compare.append(summary,changes);
      compare.ontoggle = async () => {
        row.classList.toggle('comparison-selected', compare.open); body.classList.toggle('has-open-comparison', compare.open);
        if (!compare.open) return;
        if (window.matchMedia('(min-width: 1201px)').matches) { $('#workspace').classList.add('tools-expanded'); expandTools.textContent='Collapse panel'; expandTools.setAttribute('aria-label','Collapse tools panel'); }
        changes.textContent = 'Loading changes…';
        try {
          const differences = await store.versionCompare(id,version.id);
          if (active?.id !== id || !compare.open) return;
          changes.replaceChildren();
          if (!differences.length) { changes.textContent = 'No file changes'; return; }
          const overview = document.createElement('p'); overview.className='comparison-overview'; overview.textContent=`${differences.length} ${differences.length === 1 ? 'file differs' : 'files differ'} from the current project.`; changes.append(overview);
          for (const difference of differences) {
            const item = document.createElement('details'), name = document.createElement('summary');
            name.textContent = `${difference.status}: ${difference.name}`; item.append(name);
            const columns = document.createElement('div'); columns.className = 'version-columns';
            const hint = document.createElement('p'); hint.className = 'comparison-hint'; hint.textContent = 'Highlighted lines show changes between the version and the current file.'; columns.append(hint);
            const previousText=String(difference.previous ?? ''), currentText=String(difference.current ?? '');
            if(previousText.split('\n').length * currentText.split('\n').length > 1_000_000) hint.textContent='File is too large for line alignment; all lines are highlighted.';
            const [oldRows,newRows]=comparisonLines(previousText,currentText);
            for (const [title,value,rows] of [['Version',difference.previous,oldRows],['Current',difference.current,newRows]] as const) {
              const heading = document.createElement('strong'); heading.textContent = title;
              const text = document.createElement('pre'); text.className = 'version-content';
              if (value === null) text.textContent = 'Not present, binary, or too large to display';
              else { for (const line of rows) { const row = document.createElement('span'); row.className = line.changed ? 'version-line changed' : 'version-line'; row.textContent = line.text || ' '; text.append(row); } }
              const column = document.createElement('section'); column.className = 'version-column'; column.classList.toggle('no-unique', value !== null && !text.querySelector('.changed')); column.append(heading,text); columns.append(column);
            }
            const changedOnly = document.createElement('button'); changedOnly.type='button'; changedOnly.className='secondary comparison-filter'; columns.classList.add('changed-only'); changedOnly.textContent='Show all lines'; changedOnly.setAttribute('aria-pressed','true');
            changedOnly.onclick = () => { const enabled = columns.classList.toggle('changed-only'); changedOnly.setAttribute('aria-pressed', String(enabled)); changedOnly.textContent = enabled ? 'Show all lines' : 'Show only differing lines'; };
            const comparisonControls=document.createElement('div'); comparisonControls.className='comparison-controls';
            const wrapLines=document.createElement('button'); wrapLines.type='button'; wrapLines.className='secondary comparison-wrap'; columns.classList.add('wrap-lines'); wrapLines.textContent='Unwrap lines'; wrapLines.setAttribute('aria-pressed','true');
            wrapLines.onclick=()=>{const enabled=columns.classList.toggle('wrap-lines'); wrapLines.setAttribute('aria-pressed',String(enabled)); wrapLines.textContent=enabled ? 'Unwrap lines' : 'Wrap lines';};
            comparisonControls.append(changedOnly,wrapLines); columns.insertBefore(comparisonControls, columns.children[1]);
            item.append(columns);
            changes.append(item);
          }
        } catch (error) { changes.textContent = `Could not compare: ${error.message}`; }
      };
      const actions = document.createElement('div'); actions.className = 'version-actions'; actions.append(restore, restoreProject);
      row.append(label, actions); body.append(row,compare);
    }
  } catch (error) { report('load versions', error); }
}
$('#tool-versions').onclick = () => { selectedTool = 'versions'; renderToolPanel(); };
interface CommentRecord { id:string; file:string; quote:string; text:string; resolved:boolean; created?:string; anchor?:{status:string}; replies?:{text:string}[] }
interface CommentDraftState { text:string; search:string; replies:Map<string,string>; expanded:Set<string> }
interface CommentFocusRestore { key:string; moved:boolean }
const commentDraftStates = new Map<string,CommentDraftState>();
const pendingCommentActions = new Set<string>();
let commentRenderRequest = 0;
async function renderComments(restoreFocus?:CommentFocusRestore) {
  if (!active) return; const id:string = active.id, request = ++commentRenderRequest;
  let drafts = commentDraftStates.get(id);
  if (!drafts) { drafts = {text:'',search:'',replies:new Map(),expanded:new Set()}; commentDraftStates.set(id,drafts); }
  const draft = drafts;
  const actionKey = (kind:string, commentId=''):string => JSON.stringify([id,kind,commentId]);
  const perform = async (key:string, button:HTMLButtonElement, action:()=>Promise<void>, description:string):Promise<void> => {
    if (pendingCommentActions.has(key)) return;
    const originKey = document.activeElement === button ? button.dataset.commentFocus : undefined;
    const restore:CommentFocusRestore|undefined = originKey ? {key:originKey === 'submit' ? 'new' : originKey.replace('reply-submit:', 'reply:'),moved:false} : undefined;
    const trackFocus = (event:FocusEvent):void => {
      if (restore && event.target !== document.body && event.target !== document.documentElement && event.target !== button) restore.moved = true;
    };
    document.addEventListener('focusin',trackFocus);
    pendingCommentActions.add(key); button.disabled = true;
    try { await action(); } catch (error) { report(description,error); }
    finally {
      pendingCommentActions.delete(key);
      try { if (active?.id === id && showingTools && selectedTool === 'comments') await renderComments(restore); }
      finally { document.removeEventListener('focusin',trackFocus); }
    }
  };
  try {
    const comments:CommentRecord[] = await store.commentList(id);
    if (request !== commentRenderRequest || active?.id !== id || !showingTools || selectedTool !== 'comments') return;
    const focusedKey = restoreFocus && !restoreFocus.moved ? restoreFocus.key
      : (document.activeElement instanceof HTMLElement ? document.activeElement.dataset.commentFocus : undefined);
    const body = document.getElementById('tool-body')!, heading = document.createElement('div'); heading.className = 'tool-heading'; heading.textContent = 'All Comments';
    const search = document.createElement('input'); search.type = 'search'; search.className = 'comment-search'; search.placeholder = 'Search comments'; search.setAttribute('aria-label', 'Search comments'); search.value = draft.search; search.dataset.commentFocus = 'search';
    const text = document.createElement('textarea'); text.placeholder = 'Add a comment'; text.setAttribute('aria-label','Add a comment'); text.className = 'comment-input'; text.value = draft.text; text.dataset.commentFocus = 'new'; text.oninput = () => { draft.text = text.value; };
    const add = document.createElement('button'); add.textContent = 'Comment'; add.className = 'primary comment-submit'; add.dataset.commentFocus = 'submit'; add.disabled = pendingCommentActions.has(actionKey('add'));
    const list = document.createElement('div');
    function display() {
      const query = search.value.toLowerCase();
      const filtered = comments.filter(c => `${c.text} ${c.quote} ${c.file}`.toLowerCase().includes(query));
      list.replaceChildren(...filtered.map(c => {
        const card = document.createElement('div'); card.className = 'comment-card';
        const reference = document.createElement('button'); reference.className = 'comment-reference'; reference.dataset.commentFocus = `reference:${c.id}`; reference.textContent = `${c.file}${c.quote ? ': ' + c.quote : ''}`;
        const anchorStatus = document.createElement('small');
        if (c.anchor?.status === 'orphaned') anchorStatus.textContent = 'Selection unavailable';
        reference.onclick = async () => {
          if (currentFile !== c.file) await openFile(c.file);
          if (!c.quote || active?.id !== id || currentFile !== c.file) return;
          const source = $('#source').value;
          try {
            const position = await store.commentLocate(id,c.id);
            if (active?.id !== id || currentFile !== c.file || $('#source').value !== source) return;
            if (!position) { anchorStatus.textContent = 'Selection unavailable'; return; }
            anchorStatus.textContent = ''; $('#source').focus(); $('#source').setSelectionRange(position.start,position.end);
          } catch (error) { report('find comment selection',error); }
        };
        const meta = document.createElement('div'); meta.className = 'comment-meta';
        const status = document.createElement('span'); status.className = c.resolved ? 'comment-status resolved' : 'comment-status'; status.textContent = c.resolved ? 'Resolved' : 'Open';
        const time = document.createElement('time'); if (c.created) { time.dateTime = c.created; time.textContent = new Date(c.created).toLocaleString(); } meta.append(status,time);
        const content = document.createElement('p'); content.textContent = c.text;
        const resolve = document.createElement('button'); resolve.className = 'plain'; resolve.textContent = c.resolved ? 'Reopen' : 'Resolve';
        resolve.dataset.commentFocus = `resolve:${c.id}`; resolve.disabled = pendingCommentActions.has(actionKey('resolve',c.id));
        resolve.onclick = () => perform(actionKey('resolve',c.id),resolve,()=>store.commentResolve(id,c.id,!c.resolved),'update comment');
        const replies = document.createElement('div'); replies.className = 'comment-replies';
        for (const reply of c.replies || []) { const message=document.createElement('p'); message.textContent=reply.text; replies.append(message); }
        const replyInput=document.createElement('textarea'); replyInput.className='comment-input'; replyInput.placeholder='Write a reply'; replyInput.setAttribute('aria-label','Write a reply'); replyInput.value = draft.replies.get(c.id) ?? ''; replyInput.dataset.commentFocus = `reply:${c.id}`; replyInput.oninput = () => { draft.replies.set(c.id,replyInput.value); };
        const replyButton=document.createElement('button'); replyButton.className='plain'; replyButton.textContent='Reply'; replyButton.dataset.commentFocus = `reply-submit:${c.id}`; replyButton.disabled = pendingCommentActions.has(actionKey('reply',c.id));
        replyButton.onclick = () => {
          const value = replyInput.value;
          if (!value.trim() || active?.id !== id) return;
          return perform(actionKey('reply',c.id),replyButton,async()=>{
            await store.commentReply(id,c.id,value);
            if (draft.replies.get(c.id) === value) draft.replies.delete(c.id);
          },'reply to comment');
        };
        const replyForm = document.createElement('details'); replyForm.className='comment-reply-form'; replyForm.open = draft.expanded.has(c.id) || !!draft.replies.get(c.id); replyForm.ontoggle = () => { if (!replyForm.isConnected) return; if (replyForm.open) draft.expanded.add(c.id); else draft.expanded.delete(c.id); }; const replySummary=document.createElement('summary'); replySummary.textContent='Reply'; replySummary.dataset.commentFocus = `reply-toggle:${c.id}`; replyForm.append(replySummary,replyInput,replyButton);
        const actions = document.createElement('div'); actions.className='comment-actions'; actions.append(replyForm,resolve);
        card.append(meta,reference,anchorStatus,content,replies,actions); return card;
      }));
      if (!filtered.length) { const empty = document.createElement('p'); empty.className = 'panel-empty'; empty.textContent = query ? 'No comments match your search.' : 'No comments yet. Add one to start a discussion.'; list.append(empty); }
    }
    search.oninput = () => { draft.search = search.value; display(); };
    add.onclick = () => {
      const value = text.value;
      if (!value.trim() || active?.id !== id) return;
      const file = currentFile, selection = commentSelection?.id === id && commentSelection.file === file ? commentSelection : null, quote = selection?.quote || '';
      return perform(actionKey('add'),add,async()=>{
        await store.commentAdd(id,file,quote,value,selection ? {start:selection.start,end:selection.end} : null);
        if (draft.text === value) draft.text = '';
      },'add comment');
    };
    body.replaceChildren(heading,search,text,add,list); display();
    if (focusedKey) Array.from(body.querySelectorAll<HTMLElement>('[data-comment-focus]')).find(control => control.dataset.commentFocus === focusedKey)?.focus({preventScroll:true});
  } catch (error) { report('load comments',error); }
}
$('#tool-comments').onclick = () => { selectedTool = 'comments'; renderToolPanel(); };

$('#source').addEventListener('select', () => { if (!active) return; const editor = $('#source'); commentSelection = {id: active.id, file: currentFile, start:editor.selectionStart, end:editor.selectionEnd, quote: editor.value.slice(editor.selectionStart, editor.selectionEnd)}; });

let menuFile = null, menuFolder = null;
$('#folder-add').onclick=()=>{const folder=menuFolder;$('#folder-menu').hidden=true;if(folder) openAddFile(folder);};
$('#folder-delete').onclick=async()=>{
  const name=menuFolder,id=active?.id;$('#folder-menu').hidden=true;
  if(!name || !id || restoringProject) return;
  if(chatBusy.has(id)){report('delete folder',new Error('Wait for the assistant to finish.'));return;}
  if(!confirm(`Move ${name} and its contents to Deleted files?`)) return;
  restoringProject=id;setEditorReadOnly(true);revision++;scheduleCompile();clearTimeout(compileTimer);
  try {
    const saves=await Promise.all([...(pendingSaves.get(id)?.values() || [])]);if(saves.some(Boolean)) throw new Error('Save pending changes before deleting the folder.');
    await store.trashFolder(id,name);
    if(active?.id===id) {
      const inside=file=>file.toLowerCase().startsWith(name.toLowerCase()+'/');
      const removed=inside(currentFile);openFiles=openFiles.filter(file=>!inside(file));await refreshFiles();renderFileTabs();
      if(removed) {clearEditor();if(openFiles.length) await openFile(openFiles.at(-1));}
    }
  } catch(error) {if(active?.id===id) report('delete folder',error);}
  finally {restoringProject=null;if(active?.id===id){setEditorReadOnly(chatBusy.has(id));compileQueued=false;scheduleCompile();}}
};
$('#folder-move').onclick=()=>{if(!menuFolder || !active)return;$('#folder-menu').hidden=true;$('#folder-move-path').value=menuFolder;$('#folder-move-dialog').dataset.projectId=active.id;$('#folder-move-dialog').dataset.oldPath=menuFolder;$('#folder-move-dialog').returnValue='';$('#folder-move-dialog').showModal();$('#folder-move-path').select();};
$('#folder-move-dialog').onclose=async()=>{
  const dialog=$('#folder-move-dialog'), id=dialog.dataset.projectId, oldName=dialog.dataset.oldPath, newName=$('#folder-move-path').value.trim().replaceAll('\\','/');
  if(dialog.returnValue!=='move' || !id || active?.id!==id || !oldName || !newName || newName===oldName || restoringProject) return;
  if(chatBusy.has(id)) {report('move folder',new Error('Wait for the assistant to finish.'));return;}
  restoringProject=id;setEditorReadOnly(true);revision++;scheduleCompile();clearTimeout(compileTimer);
  try {
    const saves=await Promise.all([...(pendingSaves.get(id)?.values() || [])]);if(saves.some(Boolean)) throw new Error('Save pending changes before moving the folder.');
    const meta=await store.moveFolder(id,oldName,newName);
    if(active?.id===id) {
      const remap=name=>name.toLowerCase().startsWith(oldName.toLowerCase()+'/') ? newName+name.slice(oldName.length) : name;
      const current=remap(currentFile);openFiles=openFiles.map(remap);active=meta;await refreshFiles();renderFileTabs();
      if(current!==currentFile) await openFile(current);
    }
  } catch(error) {if(active?.id===id) report('move folder',error);}
  finally {restoringProject=null;if(active?.id===id){setEditorReadOnly(chatBusy.has(id));compileQueued=false;scheduleCompile();}}
};
$('#file-rename').onclick = () => {
  if (!menuFile) return; $('#file-menu').hidden = true;
  $('#rename-file-name').value = menuFile.split('/').pop(); $('#rename-file-dialog').returnValue = ''; $('#rename-file-dialog').showModal(); $('#rename-file-name').select();
};
$('#rename-file-dialog').onclose = async () => {
  if ($('#rename-file-dialog').returnValue !== 'rename' || !active || !menuFile) return;
  const id = active.id, oldName = menuFile, basename = $('#rename-file-name').value.trim();
  if (!basename || /[\\/]/.test(basename)) return;
  const segments = oldName.split('/'); segments.pop(); segments.push(basename); const newName = segments.join('/');
  try {
    const meta = await store.renameFile(id, oldName, newName);
    if (active?.id === id) { openFiles = openFiles.map(f => f === oldName ? newName : f); active = meta; await refreshFiles(); if (currentFile === oldName) await openFile(newName); renderFileTabs(); revision++; scheduleCompile(); }
  } catch (error) { report('rename file', error); }
};
$('#file-download').onclick = () => { $('#file-menu').hidden = true; if (active && menuFile) store.downloadFile(active.id, menuFile).catch(error => report('download file',error)); };
$('#file-delete').onclick = async () => {
  $('#file-menu').hidden = true; if (!active || !menuFile) return;
  const id = active.id, name = menuFile; if (!confirm(`Delete ${name}?`)) return;
  try { await store.trashFile(id,name); if (active?.id === id) { await refreshFiles(); openFiles = openFiles.filter(f => f !== name); if (currentFile === name) { currentFile = ''; if (openFiles.length) await openFile(openFiles.at(-1)); else clearEditor(); } renderFileTabs(); revision++; scheduleCompile(); } }
  catch (error) { report('delete file',error); }
};
async function renderTrash(id) {
  const items=await store.trashList(id); if(active?.id!==id) return;
  const list=$('#trash-list'); list.replaceChildren();
  if(!items.length) { const title=document.createElement('p'), hint=document.createElement('p'); title.className='recovery-empty-title'; title.textContent='No deleted files'; hint.className='recovery-empty-hint'; hint.textContent='Files and folders deleted from this project appear here for restoration.'; list.append(title,hint); return; }
  for(const item of items) {
    const row=document.createElement('div'), label=document.createElement('span'), button=document.createElement('button');
    row.className='trash-row'; label.textContent=item.kind==='folder' ? `${item.name}/` : item.name; button.textContent='Restore';
    const description = document.createElement('div'), kind = document.createElement('small'); description.className='trash-description'; const deleted = Date.parse(item.deleted); kind.textContent=`${item.kind==='folder' ? 'Folder' : 'File'}${Number.isFinite(deleted) ? ` · Deleted ${new Date(deleted).toLocaleString()}` : ''}`; description.append(label,kind);
    button.onclick=async()=>{button.disabled=true;try {const name=await store.restoreTrash(id,item.id);if(active?.id===id){await refreshFiles();revision++;scheduleCompile();await renderTrash(id);if(item.kind!=='folder') await openFile(name);else if(!currentFile && (active.rootFile || 'main.tex').toLowerCase().startsWith(name.toLowerCase()+'/')) await openFile(active.rootFile);}}catch(error){report('restore file',error);button.disabled=false;}};
    row.append(description,button);list.append(row);
  }
}
$('#deleted-files').onclick=async()=>{if(!active)return;try{await renderTrash(active.id);$('#trash-dialog').showModal();}catch(error){report('load deleted files',error);}};
$('#trash-close').onclick=()=>$('#trash-dialog').close();
document.addEventListener('click', event => { if (!(event.target as Element).closest('#file-menu, #folder-menu')) dismissFileMenus(true); });
$('#duplicate').onclick = async () => {
  if (!active) return; const id = active.id;
  if (!confirm(`Duplicate ${active.name}?`)) return;
  try { const project = await store.duplicate(id); projects = await store.list(); renderProjects(); await openProject(project); }
  catch (error) { report('duplicate project', error); }
};

function clearEditor() {
  findPanel.hidden=true;findCache=undefined;
  saveEditorView();currentFile = ''; document.querySelectorAll('#file-list button').forEach(button => {button.classList.remove('active-file');button.removeAttribute('aria-current');}); $('#source').value = ''; updateEditor();
  clearFilePreview(); $('#numbers').hidden = true; $('.source-container').hidden = true; renderFileTabs();
}
function renderFileTabs() {
  $('#file-tabs').replaceChildren(...openFiles.map(file => {
    const tab = document.createElement('div'); tab.className = 'file-tab' + (file === currentFile ? ' active' : '');
    const open = document.createElement('button'); open.textContent = file.split('/').pop(); open.title = file; if(file===currentFile) open.setAttribute('aria-current','page'); open.onclick = () => openFile(file);
    const close = document.createElement('button'); close.replaceChildren(actionIcon('close')); close.className = 'close-tab'; close.setAttribute('aria-label', `Close ${file}`);
    close.onclick = async () => {
      const hadFocus=document.activeElement===close;
      openFiles = openFiles.filter(name => name !== file);
      if (currentFile === file) { if (openFiles.length) await openFile(openFiles.at(-1)); else clearEditor(); }
      renderFileTabs();
      if(hadFocus) {
        const selected=Array.from(document.querySelectorAll<HTMLButtonElement>('#file-tabs button[title]')).find(button=>button.title===currentFile);
        (selected ?? document.querySelector<HTMLButtonElement>('.tabs button.active'))?.focus();
      }
    };
    tab.append(open, close); return tab;
  }));
}
let searchTimer: ReturnType<typeof setTimeout> | undefined;
let searchRequest = 0;
$('.file-search-field').prepend(actionIcon('search'));
const clearFileSearch = document.createElement('button'); clearFileSearch.type='button'; clearFileSearch.className='clear-file-search'; clearFileSearch.replaceChildren(actionIcon('close')); clearFileSearch.setAttribute('aria-label','Clear file search'); clearFileSearch.hidden=true;
$('#file-search-input').after(clearFileSearch);
clearFileSearch.onclick = () => { $('#file-search-input').value=''; clearFileSearch.hidden=true; clearTimeout(searchTimer); void runProjectSearch(); $('#file-search-input').focus(); };
$('#file-search').addEventListener('keydown', event => {
  if (event.key !== 'Escape' || event.isComposing || !$('#file-search-input').value) return;
  event.preventDefault();
  event.stopPropagation();
  clearFileSearch.click();
});
async function runProjectSearch() {
  if (!active || $('#file-search').hidden) return;
  const request = ++searchRequest, id = active.id, query = $('#file-search-input').value;
  if (!query.trim()) {
    $('#file-search-results').replaceChildren();
    return;
  }
  try {
    const results = await store.search(id,query);
    if (request !== searchRequest || active?.id !== id || $('#file-search').hidden) return;
    const count = document.createElement('p'); count.className = 'search-count'; count.textContent = `${results.length} ${results.length === 1 ? 'result' : 'results'}`;
    $('#file-search-results').replaceChildren(count, ...results.map(result => {
      const button = document.createElement('button'), title = document.createElement('strong'), text = document.createElement('span');
      title.textContent = `${result.file}:${result.line}`;
      const at = result.text.toLowerCase().indexOf(query.toLowerCase());
      if (query && at >= 0) { const mark = document.createElement('mark'); mark.textContent = result.text.slice(at,at+query.length); text.append(document.createTextNode(result.text.slice(0,at)),mark,document.createTextNode(result.text.slice(at+query.length))); } else text.textContent = result.text;
      button.append(title,text);
      button.onclick = async () => {
        await openFile(result.file);
        if (active?.id !== id || currentFile !== result.file) return;
        const editor = $('#source'), lines = editor.value.split('\n');
        const offset = lines.slice(0,result.line-1).reduce((n,line) => n + line.length + 1, 0) + result.column;
        editor.focus(); editor.setSelectionRange(offset,offset + query.length);
      };
      return button;
    }));
    if (query.trim() && !results.length) { const empty = document.createElement('p'); empty.className='search-empty'; empty.textContent='No matches. Try another search.'; $('#file-search-results').append(empty); }
  } catch (error) { if (active?.id === id) report('search project',error); }
}
$('#file-search-input').oninput = () => { clearFileSearch.hidden = !$('#file-search-input').value; clearTimeout(searchTimer); ++searchRequest; if ($('#file-search-input').value.trim()) searchTimer = setTimeout(runProjectSearch,200); else void runProjectSearch(); };
// Keep file tabs and editor actions in distinct, independently scrollable rows.
const editorHeader = $('.editor-header'), editorActions = document.createElement('div'); editorActions.className='editor-actions'; editorActions.setAttribute('role','group'); editorActions.setAttribute('aria-label','Editor actions');
editorHeader.querySelectorAll(':scope > button').forEach(button => editorActions.append(button)); editorHeader.append(editorActions);
// Project-level controls live above both panes; file commands stay with the editor.
const workspace = document.getElementById('workspace')!;
const workspaceShell = document.createElement('div'); workspaceShell.className = 'workspace-shell';
workspaceShell.append(...Array.from(workspace.children));
const workspaceBar = document.createElement('div'); workspaceBar.className = 'workspace-topbar'; workspaceBar.hidden = true;
const projectTrail = document.createElement('div'); projectTrail.className = 'project-trail';
const projectNameControl = document.getElementById('project-name') as HTMLButtonElement;
projectNameControl.title = 'Rename project'; projectTrail.append(projectNameControl);
const workspaceActions = document.createElement('div');
workspaceActions.className = 'workspace-actions';
type PaneName='agent'|'editor'|'preview';
type PaneLayout=Record<PaneName,boolean>;
const layoutStorageKey='refract.workspace-panes.v1';
let paneLayout:PaneLayout={agent:true,editor:true,preview:true};
try {
  const saved:unknown=JSON.parse(localStorage.getItem(layoutStorageKey) || 'null');
  if(saved && typeof saved==='object' && ['agent','editor','preview'].every(key=>typeof (saved as Record<string,unknown>)[key]==='boolean')) {
    const value=saved as PaneLayout;paneLayout={agent:value.agent,editor:value.editor,preview:value.preview};
  }
} catch { /* Unavailable storage or invalid preferences use the default layout. */ }
function visiblePaneLayout():PaneLayout {
  const narrow=window.matchMedia('(max-width:1200px)').matches;
  return {...paneLayout,preview:paneLayout.preview && !(narrow && (paneLayout.editor || paneLayout.agent))};
}
const paneButtons=new Map<PaneName,HTMLButtonElement>();
const paneToggles=document.createElement('div');paneToggles.className='pane-toggles';paneToggles.setAttribute('role','group');paneToggles.setAttribute('aria-label','Visible panes');
for(const [name,label,controls] of [['agent','Agent','chat'],['editor','Editor','editor-pane'],['preview','Preview','pdf-content']] as const) {
  const button=document.createElement('button');button.type='button';button.id=`pane-${name}`;
  button.className='pane-toggle';button.setAttribute('aria-label',label);button.setAttribute('aria-controls',name==='editor'?'editor-content':controls);
  button.append(actionIcon(name));button.onclick=()=>togglePane(name);paneButtons.set(name,button);paneToggles.append(button);
}
workspaceShell.querySelector('.editor-wrap')!.id='editor-content';
const oldAgent=workspaceShell.querySelector<HTMLButtonElement>('#toggle-agent')!;oldAgent.remove();
workspaceActions.append(paneToggles, toolsButton);
function closeLayoutTools():void {
  showingTools=false;$('#tools-panel').hidden=true;$('#pdf-content').hidden=false;toolsButton.setAttribute('aria-expanded', 'false');
  $('#workspace').classList.remove('tools-expanded');
  expandTools.textContent='Expand panel';expandTools.setAttribute('aria-label','Expand tools panel');
}
function togglePane(name:PaneName):void {
  const wasVisible=displayedPaneLayout()[name];
  closeLayoutTools();
  const next=visiblePaneLayout();next[name]=!wasVisible;
  if(next[name] && window.matchMedia('(max-width:1200px)').matches) {
    if(name==='preview') {next.agent=false;next.editor=false;} else next.preview=false;
  }
  if(!next.agent && !next.editor && !next.preview) next.editor=true;
  paneLayout=next;applyPaneLayout();
}
function applyPaneLayout(persist=true):void {
  if(!paneLayout.agent && !paneLayout.editor && !paneLayout.preview) paneLayout.editor=true;
  const visible=visiblePaneLayout();
  workspace.classList.add('pane-layout');
  workspace.classList.remove('editor-focus','show-compact-preview');
  workspace.classList.toggle('pane-left-hidden',!visible.agent && !visible.editor);
  workspace.classList.toggle('pane-preview-hidden',!visible.preview && !showingTools);
  workspace.classList.toggle('pane-editor-hidden',!visible.editor);
  workspace.classList.toggle('pane-agent-hidden',!visible.agent);
  workspace.classList.toggle('pane-tools',showingTools);
  const editorPane=workspaceShell.querySelector('.editor-pane')!;
  editorPane.classList.toggle('agent-hidden',!visible.agent);
  editorPane.classList.toggle('chat-expanded',visible.agent && !visible.editor);
  expandChat.textContent=visible.editor?'Expand chat':'Show editor';expandChat.setAttribute('aria-pressed',String(!visible.editor));
  syncPaneButtons();
  if(persist) {try {localStorage.setItem(layoutStorageKey,JSON.stringify(paneLayout));} catch { /* Layout remains usable without storage. */ }}
  requestAnimationFrame(()=>{if(pdfFitMode==='page')fitPdfPage();else schedulePdfSharpen();});
}
function displayedPaneLayout():PaneLayout {
  const visible=visiblePaneLayout();
  if(showingTools) {
    visible.preview=false;
    if(workspace.classList.contains('tools-expanded') || window.matchMedia('(max-width:1200px)').matches) {visible.editor=false;visible.agent=false;}
  }
  return visible;
}
function syncPaneButtons():void {
  const displayed=displayedPaneLayout();
  for(const [name,button] of paneButtons) {
    button.setAttribute('aria-pressed',String(displayed[name]));
    const label=name==='agent'?'agent':name==='editor'?'editor':'preview';
    button.title=displayed[name]?`Hide ${label}`:`Show ${label}`;
    if(displayed[name] && Object.values(displayed).filter(Boolean).length===1) button.title+=' (restores editor)';
  }
}
// Tool expansion is transient and must update the indicators without saving it.
new MutationObserver(syncPaneButtons).observe(workspace,{attributes:true,attributeFilter:['class']});
window.addEventListener('resize',()=>applyPaneLayout(false));
workspaceBar.append(projectTrail, workspaceActions); document.querySelector('.app-titlebar')!.append(workspaceBar); workspace.append(workspaceShell);applyPaneLayout(false);
const fileSectionLabel = document.createElement('div'); fileSectionLabel.className = 'sidebar-label'; fileSectionLabel.textContent = 'Project'; document.getElementById('project-side')!.prepend(fileSectionLabel);
// Normalize the remaining static and generated toolbar actions.
for (const [selector,icon] of [['#add-file','plus'],['#new-chat','plus'],['#collapse','menu'],['#chat-image-close','close'],['[aria-label="Previous compiled PDF page"]','previous'],['[aria-label="Next compiled PDF page"]','next'],['[aria-label="Zoom out compiled PDF"]','minus'],['[aria-label="Zoom in compiled PDF"]','plus']] as const) {
  document.querySelector(selector)?.replaceChildren(actionIcon(icon));
}
const sidebarTabs=document.querySelector('.tabs')!;
const sidebarActions=document.createElement('div');sidebarActions.className='sidebar-actions';sidebarActions.setAttribute('role','group');sidebarActions.setAttribute('aria-label','File and project actions');
for(const id of ['add-file','deleted-files','duplicate']) sidebarActions.append(document.getElementById(id)!);
sidebarTabs.append(sidebarActions);
for(const [id,icon,label] of [['deleted-projects','trash','Deleted projects'],['deleted-files','trash','Deleted files'],['duplicate','copy','Duplicate project']] as const) {
  const button=document.getElementById(id)!;button.replaceChildren(actionIcon(icon));button.title=label;button.setAttribute('aria-label',label);
}
document.querySelector('.sidebar-footer')!.remove();
$('#outline-toggle').replaceChildren(actionIcon('next'),document.createTextNode('Outline'));
$('#compile').title='Compile document';
// Enable interaction only after all handlers have been registered.
document.body.inert=false;


// Resize adjacent workspace panes using pointer capture, including outside the handle.
function resizeHandle(id: string, axis: "x" | "y", apply: (delta: number) => void): void {
  const handle = document.getElementById(id)!;
  handle.addEventListener("pointerdown", (event: PointerEvent) => {
    if (event.button !== 0) return;
    event.preventDefault();
    handle.setPointerCapture(event.pointerId);
    let last = axis === "x" ? event.clientX : event.clientY;
    const move = (next: PointerEvent): void => {
      const position = axis === "x" ? next.clientX : next.clientY;
      apply(position - last);
      updateResizeAccessibility();
      last = position;
    };
    const end = (): void => { handle.removeEventListener("pointermove", move); };
    handle.addEventListener("pointermove", move);
    handle.addEventListener("lostpointercapture", end, { once: true });
  });
  handle.addEventListener("keydown", (event: KeyboardEvent) => {
    const negative = axis === "x" ? "ArrowLeft" : "ArrowUp";
    const positive = axis === "x" ? "ArrowRight" : "ArrowDown";
    if (event.key === negative || event.key === positive) {
      event.preventDefault();
      apply(event.key === negative ? -10 : 10);
      updateResizeAccessibility();
    }
  });
}
const sidebarResize = document.querySelector<HTMLElement>(".sidebar")!;
const workspaceResize = document.getElementById("workspace")!;
const editorResize = document.querySelector<HTMLElement>(".editor-pane")!;
const chatResize = document.getElementById("chat")!;
resizeHandle("sidebar-resize", "x", delta => {
  sidebarResize.style.setProperty("--sidebar-width", `${Math.max(150, Math.min(window.innerWidth - 400, sidebarResize.offsetWidth + delta))}px`);
});
resizeHandle("workspace-resize", "x", delta => {
  const available = workspaceResize.clientWidth - 32;
  const width = Math.max(300, Math.min(available - 260, editorResize.offsetWidth + delta));
  editorResize.style.flex = `0 0 ${width}px`;
});
resizeHandle("chat-resize", "y", delta => {
  chatResize.style.height = `${Math.max(220, Math.min(editorResize.clientHeight - 100, chatResize.offsetHeight - delta))}px`;
});

function updateResizeAccessibility(): void {
  const setRange = (id: string, target: HTMLElement, minimum: number, maximum: number, size: number): void => {
    const handle = document.getElementById(id)!;
    const value = Math.round(size);
    handle.setAttribute('aria-controls', target.id);
    handle.setAttribute('aria-valuemin', String(Math.min(minimum, value)));
    handle.setAttribute('aria-valuemax', String(Math.max(maximum, value)));
    handle.setAttribute('aria-valuenow', String(value));
    handle.setAttribute('aria-valuetext', `${value} pixels`);
  };
  setRange('sidebar-resize', sidebarResize, 150, window.innerWidth - 400, sidebarResize.offsetWidth);
  setRange('workspace-resize', editorResize, 300, workspaceResize.clientWidth - 292, editorResize.offsetWidth);
  const fraction = window.matchMedia('(max-width:1200px)').matches ? .55 : .65;
  setRange('chat-resize', chatResize, 220, Math.floor(editorResize.clientHeight * fraction), chatResize.offsetHeight);
}
const resizeAccessibilityObserver = new ResizeObserver(updateResizeAccessibility);
for (const pane of [sidebarResize, workspaceResize, editorResize, chatResize]) resizeAccessibilityObserver.observe(pane);
window.addEventListener('resize', updateResizeAccessibility);
updateResizeAccessibility();

// The entry point lives inside #projects, so it is absent from the editor workspace.
const settingsDialog = document.querySelector<HTMLDialogElement>('#settings-dialog')!;
const updateTrack = document.querySelector<HTMLSelectElement>('#update-track')!;
const updateStatus = document.querySelector<HTMLParagraphElement>('#update-status')!;
const updateDownload = document.querySelector<HTMLDivElement>('#update-download')!;
const updateProgress = document.querySelector<HTMLProgressElement>('#update-progress')!;
const updatePercent = document.querySelector<HTMLSpanElement>('#update-percent')!;
const checkUpdates = document.querySelector<HTMLButtonElement>('#check-updates')!;
const restartUpdate = document.querySelector<HTMLButtonElement>('#restart-update')!;
let restartingUpdate = false;
let settingsPoll: ReturnType<typeof setInterval> | undefined;
function renderUpdateStatus(status: import('./src/updates.cjs').UpdateStatus): void {
  updateDownload.hidden = status.phase !== 'downloading';
  const percent = status.downloadPercent;
  updatePercent.hidden = percent === undefined;
  if (percent === undefined) {
    updateProgress.removeAttribute('value');
  } else {
    updateProgress.value = percent;
    updatePercent.textContent = `${percent}%`;
  }
  restartUpdate.hidden = status.phase !== 'ready' && status.phase !== 'restarting';
  restartUpdate.disabled = restartingUpdate || status.phase !== 'ready' || updateTrack.disabled;
  updateTrack.value = status.track;
  if (updateStatus.textContent !== status.message) updateStatus.textContent = status.message;
}
async function refreshUpdateStatus(): Promise<void> {
  try { renderUpdateStatus(await store.updateStatus()); }
  catch {
    updateDownload.hidden = true;
    updateStatus.textContent = 'Could not read update status. Close settings and try again.';
  }
}
$('#settings').onclick = () => {
  settingsDialog.showModal(); void refreshUpdateStatus();
  settingsPoll = setInterval(() => { if (!restartingUpdate) void refreshUpdateStatus(); }, 2000);
};
$('#settings-close').onclick = () => settingsDialog.close();
settingsDialog.addEventListener('close', () => { clearInterval(settingsPoll); $('#settings').focus(); });
updateTrack.onchange = async () => {
  restartUpdate.hidden = true;
  updateDownload.hidden = true;
  updateTrack.disabled = true; checkUpdates.disabled = true;
  updateStatus.textContent = 'Saving track and checking for updates...';
  try { renderUpdateStatus(await store.updateTrack(updateTrack.value)); }
  catch { await refreshUpdateStatus(); updateStatus.textContent = 'Could not save the update track. Please try again.'; }
  finally { updateTrack.disabled = false; checkUpdates.disabled = false; await refreshUpdateStatus(); }
};

checkUpdates.onclick = async () => {
  restartUpdate.disabled = true;
  checkUpdates.disabled = true; updateTrack.disabled = true;
  updateStatus.textContent = 'Checking for updates...';
  try { renderUpdateStatus(await store.updateCheck()); }
  catch { updateStatus.textContent = 'Could not check for updates. Please try again.'; }
  finally { checkUpdates.disabled = false; updateTrack.disabled = false; await refreshUpdateStatus(); }
};

restartUpdate.onclick = async () => {
  if (restartingUpdate) return;
  restartingUpdate = true;
  restartUpdate.disabled = true; checkUpdates.disabled = true; updateTrack.disabled = true;
  updateStatus.textContent = 'Saving your work and restarting to update...';
  try { await store.updateRestart(); }
  catch {
    restartingUpdate = false; checkUpdates.disabled = false; updateTrack.disabled = false;
    await refreshUpdateStatus();
    updateStatus.textContent = 'Could not restart to update. Please try again.';
  }
};
