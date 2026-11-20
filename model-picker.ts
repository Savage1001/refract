(() => {
type ReasoningEffort = 'minimal' | 'low' | 'medium' | 'high' | 'xhigh' | 'max' | 'ultra';
interface CodexPreferences { model: string | null; reasoningEffort: string | null; favorites: string[]; fastMode?: boolean }
interface DiscoveredModel { id: string; model: string; displayName: string; description: string; supportedReasoningEfforts: { effort: string; description: string }[]; defaultReasoningEffort: string | null; isDefault: boolean }
type DiscoveryResult = { ok: true; models: DiscoveredModel[] } | { ok: false; error: string };
interface PickerApi { discoverOpenCodeModels(projectId?: string): Promise<{ok:true;models:{id:string;name:string;variants:string[]}[]}|{ok:false;error:string}>; openCodePreferencesLoad(): Promise<CodexPreferences>; openCodePreferencesSave(value: CodexPreferences): Promise<void>; discoverCodexModels(): Promise<DiscoveryResult>; codexPreferencesLoad(): Promise<CodexPreferences>; codexPreferencesSave(value: CodexPreferences): Promise<void> }
interface PickerModel { id: string; label: string; description: string; efforts: string[] }
interface PickerWindow extends Window { store: PickerApi }
const api = (window as unknown as PickerWindow).store;
const host = document.getElementById('model-picker')!;
const trigger = document.createElement('button');
trigger.type = 'button'; trigger.className = 'model-trigger'; trigger.setAttribute('aria-haspopup', 'dialog'); trigger.setAttribute('aria-expanded', 'false'); trigger.disabled = true;
const panel = document.createElement('div'); panel.className = 'model-panel'; panel.hidden = true; panel.setAttribute('role', 'dialog'); panel.setAttribute('aria-label', 'Choose provider and model');
const pickerHeader = document.createElement('div'); pickerHeader.className = 'model-header';
const pickerTitle = document.createElement('strong'); pickerTitle.textContent = 'Choose a model';
const pickerClose = document.createElement('button'); pickerClose.type = 'button'; pickerClose.className = 'model-close'; pickerClose.textContent = 'Close'; pickerClose.setAttribute('aria-label','Close model picker'); pickerClose.onclick = () => close(); pickerHeader.append(pickerTitle,pickerClose);
const search = document.createElement('input'); search.type = 'search'; search.placeholder = 'Search models'; search.setAttribute('aria-label', 'Search models');
const list = document.createElement('div'); list.className = 'model-list';
const effort = document.createElement('div'); effort.className = 'effort-options'; effort.setAttribute('role', 'group'); effort.setAttribute('aria-label', 'Reasoning effort');
const sidebar = document.createElement('div'); sidebar.className = 'model-sidebar';
const favoritesTab = document.createElement('button'); favoritesTab.type = 'button'; favoritesTab.className = 'model-favorites-tab'; favoritesTab.textContent = '★'; favoritesTab.setAttribute('aria-label', 'Favorite models');
favoritesTab.dataset.label = 'Favorites';
function providerLogo(provider: 'codex' | 'opencode'): HTMLPictureElement {
  const picture = document.createElement('picture'); picture.className = `provider-logo provider-logo-${provider}`; picture.setAttribute('aria-hidden', 'true');
  const source = document.createElement('source'); source.media = '(prefers-color-scheme: dark)'; source.srcset = `provider-logos/${provider === 'codex' ? 'openai-white' : 'opencode-dark'}.svg`;
  const image = document.createElement('img'); image.alt = ''; image.src = `provider-logos/${provider === 'codex' ? 'openai-black' : 'opencode-light'}.svg`;
  picture.append(source, image); return picture;
}
const codexTab = document.createElement('button'); codexTab.type = 'button'; codexTab.append(providerLogo('codex')); codexTab.setAttribute('aria-label', 'Codex models');
codexTab.dataset.label = 'Codex';
const provider = document.getElementById('chat-provider') as HTMLSelectElement;
const providerLabel = document.querySelector<HTMLLabelElement>('.provider-label')!;
provider.hidden = true; providerLabel.hidden = true;
const openCodeTab = document.createElement('button'); openCodeTab.type = 'button'; openCodeTab.append(providerLogo('opencode')); openCodeTab.setAttribute('aria-label', 'OpenCode models');
openCodeTab.dataset.label = 'OpenCode';
sidebar.append(favoritesTab, codexTab, openCodeTab);
let favoritesOnly = false;
function selectProvider(id: string): void {
  if ((provider.disabled && provider.value !== id) || !Array.from(provider.options).some(option => option.value === id && !option.disabled)) return;
  favoritesOnly = false;
  if (provider.value !== id) { provider.value = id; provider.dispatchEvent(new Event('change')); }
  render();
}
provider.addEventListener('providerstate', () => render());
provider.addEventListener('change', () => render());
favoritesTab.onclick = () => { favoritesOnly = true; render(); };
codexTab.onclick = () => selectProvider('codex');
openCodeTab.onclick = () => selectProvider('opencode');
const notice = document.createElement('p'); notice.className = 'model-notice'; notice.setAttribute('role', 'status');
const retry = document.createElement('button'); retry.type = 'button'; retry.textContent = 'Retry saving'; retry.hidden = true;
const modelsPane = document.createElement('div'); modelsPane.className = 'model-pane'; const modelsTitle=document.createElement('div'); modelsTitle.className='model-list-title'; modelsTitle.textContent='Models'; modelsPane.append(modelsTitle,search,list);
const effortPane = document.createElement('div'); effortPane.className = 'effort-pane';
const effortTitle = document.createElement('div'); effortTitle.className = 'effort-title'; effortTitle.textContent = 'Reasoning';
const effortHint = document.createElement('p'); effortHint.className = 'effort-hint'; effortHint.textContent = 'More effort can improve difficult answers but may take longer.';
effortPane.append(effortTitle, effortHint, effort);
const pickerContent = document.createElement('div'); pickerContent.className = 'model-content'; pickerContent.append(modelsPane, effortPane);
const footer = document.createElement('div'); footer.className = 'model-footer';
const speed = document.createElement('div'); speed.className = 'model-speed'; speed.setAttribute('role', 'group'); speed.setAttribute('aria-label', 'Codex speed');
const standard = document.createElement('button'); standard.type = 'button'; standard.textContent = 'Standard';
function fastIcon(): SVGSVGElement {
  const icon = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
  icon.classList.add('fast-icon'); icon.setAttribute('viewBox', '0 0 24 24'); icon.setAttribute('aria-hidden', 'true'); icon.setAttribute('focusable', 'false');
  const bolt = document.createElementNS('http://www.w3.org/2000/svg', 'path');
  bolt.setAttribute('d', 'M13 2 3 14h8l-1 8L21 10h-8l1-8Z');
  icon.append(bolt); return icon;
}
const fast = document.createElement('button'); fast.type = 'button'; fast.append(fastIcon(), 'Fast'); fast.setAttribute('aria-label', 'Fast'); fast.title = 'Faster responses with higher usage. Availability depends on your model and account.';
standard.onclick = () => setFastMode(false); fast.onclick = () => setFastMode(true);
function setFastMode(enabled: boolean): void { if (activeId() !== 'codex' || states.codex.loadFailed) return; states.codex.preferences.fastMode = enabled; changed('codex'); }
speed.append(standard, fast); footer.append(speed);
panel.append(pickerHeader, sidebar, pickerContent, footer); host.append(trigger, panel, notice, retry);
const supportedEfforts = new Set<string>(['minimal', 'low', 'medium', 'high', 'xhigh', 'max', 'ultra']);
type ProviderId = 'codex' | 'opencode';
interface PickerState { models: PickerModel[]; preferences: CodexPreferences; status: 'loading' | 'ready' | 'error'; error: string; loadFailed: boolean; saveError: boolean; revision: number; savedRevision: number; savePromise: Promise<void> | null; initialized: Promise<void> | null }
function newState(): PickerState { return {models:[], preferences:{model:null,reasoningEffort:null,favorites:[]}, status:'loading', error:'', loadFailed:false, saveError:false, revision:0, savedRevision:0, savePromise:null, initialized:null}; }
const states: Record<ProviderId, PickerState> = {codex:newState(),opencode:newState()};
function activeId(): ProviderId { return provider.value === 'opencode' ? 'opencode' : 'codex'; }
function activeState(): PickerState { return states[activeId()]; }
function updateNotice(): void {
  const state = activeState();
  notice.textContent = state.loadFailed ? 'Could not load model preferences. Restart to retry; saved preferences were not changed.' : state.saveError ? 'Could not save model preferences. Retry saving.' : state.status === 'error' ? `Could not load models: ${state.error}` : '';
  retry.hidden = !state.saveError;
}
function save(id: ProviderId): Promise<void> {
  const state = states[id]; if (state.savePromise) return state.savePromise;
  state.saveError = false; updateNotice();
  state.savePromise = (async () => {
    while (state.savedRevision < state.revision) {
      const version = state.revision, value = {...state.preferences, favorites:[...state.preferences.favorites]};
      try { await (id === 'codex' ? api.codexPreferencesSave(value) : api.openCodePreferencesSave(value)); state.savedRevision = version; }
      catch { state.saveError = true; break; }
    }
  })().finally(() => { state.savePromise = null; updateNotice(); });
  return state.savePromise;
}
function changed(id: ProviderId = activeId()): void { states[id].revision++; void save(id); render(); }
retry.onclick = () => { void save(activeId()); };
function compareVersions(a: PickerModel, b: PickerModel): number {
  const version = (model: PickerModel): number[] => (model.label.match(/\d+(?:\.\d+)*/)?.[0] ?? model.id.match(/\d+(?:\.\d+)*/)?.[0] ?? '0').split('.').map(Number);
  const left = version(a), right = version(b);
  for (let i = 0; i < Math.max(left.length, right.length); i++) { const difference = (right[i] ?? 0) - (left[i] ?? 0); if (difference) return difference; }
  return a.label.localeCompare(b.label);
}
function preferredEffort(model: PickerModel | undefined): string | null {
  const choices = model?.efforts ?? [];
  const order = ['none', 'minimal', 'low', 'medium', 'high', 'xhigh', 'max', 'ultra'];
  const known = choices.filter(value => order.includes(value));
  if (known.length) return known.reduce((best, value) => Math.abs(order.indexOf(value) - 3) < Math.abs(order.indexOf(best) - 3) ? value : best);
  return choices[Math.floor((choices.length - 1) / 2)] ?? null;
}
function effortLabel(value: string): string {
  return value === 'xhigh' ? 'Extra High' : value.replace(/\b\p{L}/gu, letter => letter.toUpperCase());
}
function render(): void {
  const focusedKey = document.activeElement instanceof HTMLElement && panel.contains(document.activeElement) ? document.activeElement.dataset.focusKey : undefined;
  const state = activeState(), models = state.models, preferences = state.preferences;
  const selected = models.find(m => m.id === preferences.model);
  const openCode = provider.value === 'opencode';
  footer.hidden = openCode || state.loadFailed;
  standard.setAttribute('aria-pressed', String(!preferences.fastMode));
  fast.setAttribute('aria-pressed', String(!!preferences.fastMode));
  trigger.classList.toggle('is-opencode', openCode);
  modelsPane.hidden = state.loadFailed; effortPane.hidden = state.loadFailed;
  for (const [tab, id] of [[codexTab, 'codex'], [openCodeTab, 'opencode']] as const) {
    tab.disabled = (provider.disabled && provider.value !== id) || !Array.from(provider.options).some(option => option.value === id && !option.disabled);
    tab.setAttribute('aria-pressed', String(!favoritesOnly && provider.value === id));
  }
  trigger.textContent = state.loadFailed ? 'Model preferences unavailable' : (selected?.label ?? (preferences.model ? `${openCode ? preferences.model.slice(preferences.model.indexOf('/') + 1) : preferences.model} (unavailable)` : (openCode ? 'Loading models...' : 'Default model'))) + (preferences.reasoningEffort ? ` \u00b7 ${effortLabel(preferences.reasoningEffort)}` : '');
  if (!openCode && preferences.fastMode) trigger.append(' \u00b7 ', fastIcon(), ' Fast');
  favoritesTab.setAttribute('aria-pressed', String(favoritesOnly));
  effortTitle.textContent = 'Reasoning';
  list.replaceChildren();
  const query = search.value.trim().toLowerCase();
  const matches = models.filter(m => (!favoritesOnly || preferences.favorites.includes(m.id)) && `${m.label} ${m.id} ${m.description}`.toLowerCase().includes(query)).slice().sort((a,b) => a.label.localeCompare(b.label, undefined, {sensitivity:'base'}));
  for (const model of matches) {
    const row = document.createElement('div'); row.className = 'model-row';
    const button = document.createElement('button'); button.type = 'button'; button.dataset.focusKey = `model:${model.id}`; button.setAttribute('aria-pressed', String(preferences.model === model.id)); button.textContent = `${preferences.model === model.id ? '✓ ' : ''}${model.label}`; button.prepend(providerLogo(activeId())); if (activeId() !== 'opencode') button.title = model.description || model.id; if (activeId() === 'opencode') { button.dataset.provider = model.description; button.classList.add('model-provider-tooltip'); } button.onclick = () => choose(model.id);
    const favorite = document.createElement('button'); favorite.type = 'button'; favorite.className = 'model-favorite'; favorite.dataset.focusKey = `favorite:${model.id}`; const isFavorite = preferences.favorites.includes(model.id); favorite.textContent = isFavorite ? '★' : '☆'; favorite.setAttribute('aria-label', `${isFavorite ? 'Remove' : 'Add'} ${model.label} ${isFavorite ? 'from' : 'to'} favorites`); favorite.onclick = () => { preferences.favorites = isFavorite ? preferences.favorites.filter(id => id !== model.id) : [...preferences.favorites, model.id]; changed(); };
    row.classList.toggle('is-selected', preferences.model === model.id); row.append(button, favorite); list.append(row);
  }
  if (!openCode && !favoritesOnly && 'default model'.includes(query)) {
    const row = document.createElement('div'); row.className = 'model-row';
    row.classList.toggle('is-selected', preferences.model === null);
    const button = document.createElement('button'); button.type = 'button'; button.textContent = 'Default model'; button.dataset.focusKey = 'model:default';
    button.setAttribute('aria-pressed', String(preferences.model === null));
    button.onclick = () => { if (preferences.model !== null || preferences.reasoningEffort !== null) { preferences.model = null; preferences.reasoningEffort = null; changed(); } close(); };
    row.append(button); list.append(row);
  }
  if (!matches.length && (!list.children.length || state.status !== 'ready')) { const message = document.createElement('p'); message.textContent = state.status === 'loading' ? 'Loading models…' : state.status === 'error' ? 'Models could not be loaded.' : 'No matching models.'; list.append(message); }
  if (!openCode && state.status === 'error') {
    const retry = document.createElement('button');
    retry.type = 'button';
    retry.textContent = 'Retry model discovery';
    retry.className = 'model-retry';
    retry.addEventListener('click', () => {
      if (state.status !== 'error') return;
      state.status = 'loading';
      state.initialized = initialize('codex', undefined, state.preferences);
      render();
      search.focus();
    });
    list.append(retry);
  }
  effort.replaceChildren();
  const fasterEffort = ['low','minimal'].find(value=>selected?.efforts.includes(value));
  if(fasterEffort) {
    const faster = document.createElement('button');faster.type='button';faster.textContent='Faster replies';faster.dataset.focusKey='effort:faster';
    faster.title='Use lower reasoning effort for simple requests. Difficult requests may benefit from more effort.';
    faster.onclick=()=>{preferences.reasoningEffort=fasterEffort;changed();};effort.append(faster);
  }
  const choices: { value: string | null; label: string }[] = (selected?.efforts ?? []).map(value => ({ value, label: effortLabel(value) }));
  if (!openCode && selected) choices.unshift({value:null, label:'Default effort'});
  if (preferences.reasoningEffort && !selected?.efforts.includes(preferences.reasoningEffort)) choices.push({ value: preferences.reasoningEffort, label: `${effortLabel(preferences.reasoningEffort)} (unavailable)` });
  for (const choice of choices) {
    const button = document.createElement('button'); button.type = 'button'; button.textContent = choice.label; button.dataset.focusKey = `effort:${choice.value ?? 'default'}`; button.disabled = !selected || (choice.value !== null && !selected.efforts.includes(choice.value));
    button.setAttribute('aria-pressed', String(choice.value === preferences.reasoningEffort));
    button.onclick = () => { preferences.reasoningEffort = choice.value; changed(); }; effort.append(button);
  }
  updateNotice();
  if (focusedKey && !panel.hidden) {
    const replacement = Array.from(panel.querySelectorAll<HTMLElement>('[data-focus-key]')).find(control => control.dataset.focusKey === focusedKey);
    (replacement ?? search).focus({preventScroll:true});
  }
  if (!panel.hidden && !query) requestAnimationFrame(() => {
    const selectedRow=list.querySelector<HTMLElement>('.model-row.is-selected');
    if (!selectedRow || panel.hidden) return;
    const rowBounds=selectedRow.getBoundingClientRect(), listBounds=list.getBoundingClientRect();
    if(rowBounds.top < listBounds.top || rowBounds.bottom > listBounds.bottom) list.scrollTop += rowBounds.top-listBounds.top;
  });
}
function choose(model: string): void { const state = activeState(); if (model !== state.preferences.model) { state.preferences.model = model; const selected = state.models.find(m => m.id === model); if (state.preferences.reasoningEffort && !selected?.efforts.includes(state.preferences.reasoningEffort)) state.preferences.reasoningEffort = activeId() === 'codex' ? null : preferredEffort(selected); else if (activeId() === 'opencode' && !state.preferences.reasoningEffort) state.preferences.reasoningEffort = preferredEffort(selected); changed(); } if (activeId() === 'codex') close(); else render(); }
function positionPanel(): void {
  const anchor = trigger.getBoundingClientRect();
  const width = panel.getBoundingClientRect().width;
  const editor = document.querySelector<HTMLElement>('.editor-pane')?.getBoundingClientRect();
  const minimumLeft = Math.max(12, Math.min(editor ? editor.left + 8 : 12, window.innerWidth - width - 12));
  const above=Math.max(0,anchor.top-18), below=Math.max(0,window.innerHeight-anchor.bottom-18);
  const placeAbove=above>=400 || above>=below;
  panel.style.maxHeight = `${Math.max(0,placeAbove ? above : below)}px`;
  panel.style.left = `${Math.max(minimumLeft, Math.min(anchor.right - width, window.innerWidth - width - 12))}px`;
  panel.style.top = placeAbove ? 'auto' : `${anchor.bottom+6}px`;
  panel.style.bottom = placeAbove ? `${window.innerHeight-anchor.top+6}px` : 'auto';
}
window.addEventListener('resize', () => { if (!panel.hidden) positionPanel(); });
function dismiss(restoreFocus = false): void { panel.hidden = true; trigger.setAttribute('aria-expanded', 'false'); if (restoreFocus) trigger.focus(); }
function close(): void { dismiss(true); }
// Nonmodal picker follows keyboard focus, without taking focus from its destination.
document.addEventListener('focusin', event => { if (event.target instanceof Node && !host.contains(event.target)) dismiss(); });
const pickerVisibility = new IntersectionObserver(entries => { if (entries.some(entry => !entry.isIntersecting) && !panel.hidden) dismiss(); });
pickerVisibility.observe(trigger);
trigger.onclick = () => { if (!panel.hidden) return close(); panel.hidden = false; positionPanel(); trigger.setAttribute('aria-expanded', 'true'); search.value = ''; render(); (modelsPane.hidden ? codexTab : search).focus(); };
search.oninput = render;
panel.onkeydown = event => { if (event.isComposing || event.keyCode === 229) return; if (event.key === 'Escape') { event.preventDefault(); close(); } else if ((event.key === 'ArrowDown' || event.key === 'ArrowUp') && (document.activeElement === search || list.contains(document.activeElement))) { const buttons = Array.from(list.querySelectorAll('button')); if (!buttons.length) return; event.preventDefault(); const index = buttons.indexOf(document.activeElement as HTMLButtonElement); buttons[(index + (event.key === 'ArrowDown' ? 1 : index < 0 ? 0 : -1) + buttons.length) % buttons.length].focus(); } else if (event.key === 'Enter' && document.activeElement === search) { event.preventDefault(); (list.querySelector('.model-row button') as HTMLButtonElement | null)?.click(); } };
document.addEventListener('click', event => { if (!event.composedPath().includes(host)) { dismiss(); } });
let openCodeProject: string | undefined;
let openCodeGeneration = 0;
let codexGeneration = 0;
let openCodePreferencesLoaded = false;
async function initialize(id: ProviderId, projectId?: string, existingPreferences?: CodexPreferences): Promise<void> {
  const state = states[id];
  const generation = id === 'opencode' ? ++openCodeGeneration : ++codexGeneration;
  let discovered: PickerModel[] | undefined; let discoveryError = '';
  const discovery = (async () => {
    try {
      if (id === 'codex') {
        const result = await api.discoverCodexModels(); if (result.ok === false) throw new Error(result.error);
        discovered = result.models.map(m => ({id:m.model,label:m.displayName,description:m.description,efforts:m.supportedReasoningEfforts.map(e=>e.effort).filter((value): value is ReasoningEffort => supportedEfforts.has(value))}));
      } else {
        const result = await api.discoverOpenCodeModels(projectId); if (result.ok === false) throw new Error(result.error);
        discovered = result.models.map(m => ({id:m.id,label:m.name,description:({opencode:'OpenCode',lmstudio:'LM Studio'} as Record<string,string>)[m.id.split('/')[0].toLowerCase()] ?? m.id.split('/')[0],efforts:m.variants}));
      }
    } catch (error) { discoveryError = error instanceof Error ? error.message : 'Discovery failed'; }
  })();
  let loaded: CodexPreferences | undefined;
  const revision = state.revision;
  try { loaded = existingPreferences ?? (id === 'opencode' && openCodePreferencesLoaded ? state.preferences : await (id === 'codex' ? api.codexPreferencesLoad() : api.openCodePreferencesLoad())); } catch {}
  await discovery;
  if (generation !== (id === 'opencode' ? openCodeGeneration : codexGeneration)) return;
  state.loadFailed = !loaded; if (loaded && state.revision === revision) state.preferences = loaded;
  if (id === 'opencode' && loaded) openCodePreferencesLoaded = true;
  state.status = discovered ? 'ready' : 'error'; state.error = discoveryError; if (discovered) state.models = discovered;
  if (id === 'opencode' && !state.loadFailed && state.status === 'ready') {
    let modified = false;
    if (!state.preferences.model && state.models.length) { state.preferences.model = [...state.models].sort(compareVersions)[0].id; modified = true; }
    const selected = state.models.find(m => m.id === state.preferences.model);
    if (selected && (!state.preferences.reasoningEffort || !selected.efforts.includes(state.preferences.reasoningEffort))) { const fallback = preferredEffort(selected); if (fallback !== state.preferences.reasoningEffort) { state.preferences.reasoningEffort = fallback; modified = true; } }
    if (modified) changed(id);
  }
  trigger.disabled = false; render();
}
states.codex.initialized = initialize('codex');
window.addEventListener('codex-connection-changed', () => {
  const state = states.codex;
  state.status = 'loading';
  state.models = [];
  state.initialized = initialize('codex');
  render();
});
states.opencode.initialized = initialize('opencode');
window.addEventListener('picker-project', (event: CustomEvent<string>) => { if (event.detail === openCodeProject) return; openCodeProject = event.detail; states.opencode.status = 'loading'; states.opencode.initialized = initialize('opencode', openCodeProject); render(); });
(window as unknown as Window & { codexPickerReady: () => Promise<void> }).codexPickerReady = async () => {
  const state = activeState(); await state.initialized;
  if (state.loadFailed) throw new Error('Could not load model preferences. Restart to retry.');
  if (state.savePromise) await state.savePromise;
  if (state.saveError || state.savedRevision < state.revision) throw new Error('Could not save model preferences. Retry saving before sending.');
  if (activeId() === 'opencode' && !state.preferences.model) throw new Error('Select an available model before sending.');
};

})();
