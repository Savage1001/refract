(() => {
type Provider = 'codex' | 'opencode';
type Scope = 'all' | 'favorites' | Provider;
type Preferences = import('./src/codex-preferences.cjs').CodexPreferences;
type Selection = Omit<Preferences, 'favorites'>;
type Discovery<T> = {ok: true; models: T[]} | {ok: false; error: string};
interface Model {
  id: string | null;
  label: string;
  source?: string;
  description?: string;
  efforts: string[];
  defaultEffort?: string;
  disambiguator?: string;
}
interface State {
  models: Model[];
  status: 'loading' | 'ready' | 'error';
  error?: string;
  preferences?: Preferences;
  loadFailed?: boolean;
  revision: number;
  savedRevision: number;
  selectionRevision: number;
  loadingPreferences?: Promise<Preferences | undefined>;
  saveError?: boolean;
  savePromise?: Promise<void>;
  write?: Promise<boolean>;
  initialized?: Promise<void>;
}
interface PickerApi {
  discoverCodexModels(): Promise<Discovery<import('./src/codex-models.cjs').CodexModel>>;
  discoverOpenCodeModels(project?: string): Promise<Discovery<{id: string; name: string; variants: string[]}>>;
  codexPreferencesLoad(): Promise<Preferences>;
  openCodePreferencesLoad(): Promise<Preferences>;
  codexPreferencesSave(value: Preferences): Promise<void>;
  openCodePreferencesSave(value: Preferences): Promise<void>;
}

const pickerWindow = window as unknown as Window & {
  store: PickerApi;
  codexPickerReady: (id?: Provider) => Promise<Selection>;
};
const api = pickerWindow.store;
const host = document.querySelector<HTMLElement>('#model-picker')!;
const provider = document.querySelector<HTMLSelectElement>('#chat-provider')!;
provider.hidden = true;
document.querySelector<HTMLElement>('.provider-label')!.hidden = true;
const providers = ['codex', 'opencode'] as const;
const labels = {all: 'All models', favorites: 'Favorites', codex: 'Codex', opencode: 'OpenCode'};
const sourceNames: Record<string, string> = {
  opencode: 'OpenCode', lmstudio: 'LM Studio', anthropic: 'Anthropic',
  openai: 'OpenAI', openrouter: 'OpenRouter', google: 'Google'
};
function openCodeSource(id: string): string {
  const prefix = id.split('/')[0];
  return sourceNames[prefix.toLowerCase()] ?? prefix;
}
const effortOrder = ['none', 'minimal', 'low', 'medium', 'high', 'xhigh', 'max', 'ultra'];
const numeric = new Intl.Collator(undefined, {numeric: true, sensitivity: 'base'});
const cliDefault: Model = {id: null, label: 'CLI default', description: 'Use the model and reasoning configured in Codex.', efforts: []};
const newState = (): State => ({models: [], status: 'loading', revision: 0, savedRevision: 0, selectionRevision: 0});
const states = {codex: newState(), opencode: newState()};
let scope: Scope = 'all';
let firstProvider: Provider = 'codex';
let project: string | undefined;
let generation = 0;
let catalogProject: string | undefined;
let detected = new Set<Provider>();
let resultKey: string | undefined;
let detailsOpen = false;

function element<K extends keyof HTMLElementTagNameMap>(tag: K, className = '', content?: string): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  node.className = className;
  if (content) node.textContent = content;
  return node;
}
function button(className: string, label: string, onClick: () => void): HTMLButtonElement {
  const control = element('button', className, label);
  control.type = 'button';
  control.onclick = onClick;
  return control;
}
function providerLogo(id: Provider): HTMLPictureElement {
  const picture = element('picture', `provider-logo provider-logo-${id}`);
  picture.setAttribute('aria-hidden', 'true');
  const source = element('source');
  source.media = '(prefers-color-scheme: dark)';
  source.srcset = `provider-logos/${id === 'codex' ? 'openai-white' : 'opencode-dark'}.svg`;
  const image = element('img');
  image.alt = '';
  image.src = `provider-logos/${id === 'codex' ? 'openai-black' : 'opencode-light'}.svg`;
  picture.append(source, image);
  return picture;
}
function effortLabel(value: string): string {
  return value === 'xhigh' ? 'Extra high' : value.replace(/[-_]/g, ' ').replace(/^\p{L}/u, letter => letter.toUpperCase());
}
function preferredEffort(model: Model): string | null {
  const known = model.efforts.filter(value => effortOrder.includes(value));
  if (!known.length) return model.efforts[Math.floor((model.efforts.length - 1) / 2)] ?? null;
  const distance = (value: string) => Math.abs(effortOrder.indexOf(value) - 3);
  return known.reduce((best, value) => distance(value) < distance(best) ? value : best);
}
function activeId(): Provider | undefined {
  return provider.value === 'codex' || provider.value === 'opencode' ? provider.value : undefined;
}
function installed(id: Provider): boolean {
  return Array.from(provider.options).some(option => option.value === id && !option.disabled);
}
function pending(): boolean { return provider.dataset.pickerStatus === 'loading'; }
function busy(): boolean { return provider.dataset.pickerStatus === 'busy'; }
function canSelect(id: Provider): boolean {
  return installed(id) && !pending() && !busy() && (!provider.disabled || activeId() === id);
}
function selectedModel(id: Provider): Model | undefined {
  const {preferences, models} = states[id];
  if (!preferences) return;
  return preferences.model === null && id === 'codex' ? cliDefault : models.find(model => model.id === preferences.model);
}
function modelKey(id: Provider, model: string | null): string { return JSON.stringify([id, model]); }

const trigger = button('model-trigger', '', toggle);
trigger.setAttribute('aria-haspopup', 'dialog');
trigger.setAttribute('aria-expanded', 'false');
trigger.setAttribute('aria-controls', 'model-picker-panel');
const panel = element('div', 'model-panel');
panel.id = 'model-picker-panel';
panel.hidden = true;
panel.setAttribute('role', 'dialog');
panel.setAttribute('aria-labelledby', 'model-picker-title');
const header = element('div', 'model-header');
const title = element('h2', '', 'Choose a model');
title.id = 'model-picker-title';
header.append(title, button('model-done', 'Done', close));
const searchBar = element('div', 'model-search');
const search = element('input');
search.type = 'search';
search.placeholder = 'Search models';
search.setAttribute('aria-label', 'Search models');
search.setAttribute('aria-controls', 'model-picker-results');
searchBar.append(search);
const filters = element('div', 'model-filters');
filters.setAttribute('role', 'group');
filters.setAttribute('aria-label', 'Filter models');
const filterButtons = (['all', 'favorites', ...providers] as const).map(id => {
  const control = button('model-filter', labels[id], () => {
    scope = id;
    resetResults();
    if (id === 'favorites') {
      for (const provider of providers) {
        const state = states[provider];
        if (!state.preferences && !state.initialized) void loadPreferences(provider).then(render);
      }
    }
    render();
  });
  if (id === 'codex' || id === 'opencode') control.prepend(providerLogo(id));
  if (id === 'favorites') control.setAttribute('aria-label', 'Favorite models');
  if (id === 'codex' || id === 'opencode') control.setAttribute('aria-label', `${labels[id]} models`);
  filters.append(control);
  return {id, control};
});

const body = element('div', 'model-content');
const list = element('div', 'model-list');
list.id = 'model-picker-results';
list.setAttribute('role', 'region');
list.setAttribute('aria-label', 'Model results');
const settings = element('section', 'model-settings');
settings.setAttribute('aria-label', 'Current model settings');
settings.id = 'model-picker-settings';
const settingsToggle = button('model-settings-toggle', 'Model settings', () => {
  detailsOpen = !detailsOpen;
  updateDetailsVisibility();
});
settingsToggle.setAttribute('aria-controls', settings.id);
const settingsGroup = element('div', 'model-settings-group');
settingsGroup.append(settingsToggle, settings);
body.append(list, settingsGroup);
const announcements = element('span', 'model-announcement');
announcements.setAttribute('role', 'status');
const selectionAnnouncement = element('span', 'model-announcement');
selectionAnnouncement.setAttribute('role', 'status');
const notice = button('model-notice', '', openRecovery);
notice.id = 'model-picker-recovery';
function openRecovery(): void {
  const issue = sendIssue();
  const id = issue ? activeId() : saveFailure();
  if (!id) return;
  if (panel.hidden) toggle();
  if ((issue && issue.area === 'settings') || (!issue && id === activeId())) {
    detailsOpen = true;
    updateDetailsVisibility();
    (settings.querySelector<HTMLElement>('.model-retry:enabled,select:enabled') ?? settingsToggle).focus();
  } else {
    scope = id;
    resetResults();
    render();
    (list.querySelector<HTMLElement>('.model-retry:enabled') ?? search).focus();
  }
}
notice.hidden = true;
panel.append(header, searchBar, filters, body, announcements, selectionAnnouncement);
host.append(trigger, panel, notice);

function save(id: Provider): Promise<void> {
  const state = states[id];
  if (state.savePromise) return state.savePromise;
  state.saveError = false;
  state.savePromise = (async () => {
    while (state.savedRevision < state.revision) {
      const version = state.revision;
      const value = {...state.preferences!, favorites: [...state.preferences!.favorites]};
      state.write = Promise.resolve().then(() =>
        id === 'codex' ? api.codexPreferencesSave(value) : api.openCodePreferencesSave(value)
      ).then(() => {
        state.savedRevision = version;
        return true;
      }, () => {
        state.saveError = true;
        return false;
      });
      if (!await state.write) break;
    }
  })().finally(() => {
    state.write = undefined;
    state.savePromise = undefined;
    render();
  });
  return state.savePromise;
}
function changed(id: Provider, affectsSelection = true): void {
  const state = states[id];
  state.revision++;
  if (affectsSelection) state.selectionRevision = state.revision;
  void save(id);
}

function choose(id: Provider, model: Model): void {
  const {preferences} = states[id];
  if (!preferences || !canSelect(id)) return;
  if (preferences.model !== model.id || (model.id === null && preferences.reasoningEffort !== null)) {
    preferences.model = model.id;
    if (model.id === null) preferences.reasoningEffort = null;
    else if (!model.efforts.includes(preferences.reasoningEffort ?? '')) {
      preferences.reasoningEffort = id === 'codex' ? null : preferredEffort(model);
    }
    changed(id);
  }
  if (activeId() !== id) {
    provider.value = id;
    provider.dispatchEvent(new Event('change'));
  }
  render();
  selectionAnnouncement.textContent = [model.label, labels[id], model.disambiguator, 'selected.'].filter(Boolean).join(', ');
}

function saveFailure(): Provider | undefined {
  const id = activeId();
  return id && states[id].saveError ? id : providers.find(candidate => states[candidate].saveError);
}

interface SendIssue {message: string; area: 'models' | 'settings'; loading?: boolean}
function sendIssue(id = activeId()): SendIssue | undefined {
  if (!id || !installed(id)) return;
  const state = states[id];
  if (state.loadFailed) return {message: 'Retry loading model preferences', area: 'models'};
  if (!state.preferences || state.status === 'loading') {
    return {message: `Checking ${labels[id]} models…`, area: 'models', loading: true};
  }
  if (state.saveError && state.savedRevision < state.selectionRevision)
    return {message: 'Retry saving model settings', area: 'settings'};
  const model = selectedModel(id);
  if (id === 'opencode' && state.status === 'error')
    return {message: 'Retry loading OpenCode models', area: 'models'};
  if (!model && state.status === 'ready') return {message: `Choose an available ${labels[id]} model`, area: 'models'};
  if (model?.id && state.preferences.reasoningEffort && !model.efforts.includes(state.preferences.reasoningEffort))
    return {message: id === 'codex' ? 'Choose an available reasoning effort' : 'Choose an available model variant', area: 'settings'};
}
function publishReadiness(): void {
  const issue = sendIssue();
  const message = issue?.message ?? '';
  if ((provider.dataset.pickerBlock ?? '') !== message) {
    provider.dataset.pickerBlock = message;
    provider.dispatchEvent(new Event('picker-readiness'));
  }
  const failedSave = saveFailure();
  notice.hidden = !message && !failedSave;
  notice.classList.toggle('is-loading', Boolean(issue?.loading));
  notice.textContent = message || (failedSave ? `${labels[failedSave]} settings weren’t saved` : '');
  if (notice.hidden) trigger.removeAttribute('aria-describedby');
  else trigger.setAttribute('aria-describedby', notice.id);
}
function updateTrigger(): void {
  const id = activeId();
  const state = id && states[id];
  const model = id && selectedModel(id);
  const preferences = state?.preferences;
  let label = 'Choose a model';
  if (pending()) label = 'Checking providers…';
  else if (!id) label = provider.dataset.pickerStatus === 'error' ? 'Provider check failed' : 'No providers found';
  else if (!installed(id)) label = `${labels[id]} unavailable`;
  else if (state.loadFailed) label = 'Preferences unavailable';
  else if (model) label = model.label;
  else if (state.status === 'loading') label = preferences?.model ?? 'Loading models…';
  else if (preferences?.model) label = `${preferences.model.split('/').at(-1)} (unavailable)`;
  else if (state.status === 'error') label = 'Models unavailable';

  const effort = preferences?.reasoningEffort && effortLabel(preferences.reasoningEffort);
  const speed = id === 'codex' && preferences?.fastMode ? 'Fast' : '';
  const options = [effort, speed].filter(Boolean).join(' · ');
  const identity = model?.disambiguator ?? model?.source ?? '';
  const fullLabel = [id && installed(id) ? labels[id] : '', label, identity !== labels[id] || model?.disambiguator ? identity : '', options].filter(Boolean).join(' · ');
  trigger.replaceChildren();
  if (id) trigger.append(providerLogo(id));
  trigger.append(element('span', 'model-trigger-label', label));
  if (model?.disambiguator) trigger.append(element('span', 'model-trigger-options', identity));
  if (options && model && installed(id)) trigger.append(element('span', 'model-trigger-options', options));
  const arrow = element('span', 'model-chevron', '⌄');
  arrow.setAttribute('aria-hidden', 'true');
  trigger.append(arrow);
  trigger.setAttribute('aria-label', `Choose provider and model. ${fullLabel}`);
  trigger.title = fullLabel;
  publishReadiness();
}

function statusBlock(title: string, description: string, action?: HTMLButtonElement): HTMLElement {
  const status = element('div', 'model-status');
  status.append(element('strong', '', title), element('p', '', description));
  if (action) status.append(action);
  return status;
}
function retryButton(label: string, key: string, action: () => void): HTMLButtonElement {
  const control = button('model-retry', label, action);
  control.dataset.focusKey = key;
  return control;
}
function retryProviders(): void { provider.dispatchEvent(new Event('picker-retry-providers')); }
function providerStatus(id: Provider): HTMLElement | undefined {
  const state = states[id];
  if (state.loadFailed) return statusBlock('Preferences couldn’t be loaded', 'Your saved choices are kept.',
    retryButton('Retry preferences', `${id}:preferences`, () => {
      if (installed(id)) startDiscovery(id);
      else void loadPreferences(id).then(render);
      render();
    }));
  if (!installed(id)) {
    if (scope === 'favorites') return state.loadingPreferences
      ? statusBlock('Loading favorites…', `Reading saved ${labels[id]} models.`) : undefined;
    return statusBlock(`${labels[id]} unavailable`, 'Install and authenticate the CLI, then check again.',
      retryButton('Check providers', 'providers:retry', retryProviders));
  }
  if (state.status === 'error') return statusBlock('Models couldn’t be loaded', state.error ?? 'Check your provider setup and retry.',
    retryButton('Retry model discovery', `${id}:discovery`, () => startDiscovery(id)));
  if (state.status === 'loading') return statusBlock('Loading models…', `Checking the ${labels[id]} catalog and preferences.`);
  if (!state.models.length) return statusBlock('No models available', `Check your ${labels[id]} setup to add models.`,
    retryButton('Check for models', `${id}:discovery`, () => startDiscovery(id)));
}
function matchesQuery(model: Model, id: Provider, terms: string[]): boolean {
  const text = [model.label, model.id, model.source, model.description, labels[id]].filter(Boolean).join(' ').toLowerCase();
  return terms.every(term => text.includes(term));
}
function modelRow(id: Provider, model: Model, available: boolean): HTMLElement {
  const {preferences} = states[id];
  const key = modelKey(id, model.id);
  const current = activeId() === id && preferences?.model === model.id;
  const row = element('div', 'model-row');
  row.classList.toggle('is-selected', current);
  const choice = button('model-choice', '', () => choose(id, model));
  choice.dataset.result = key;
  choice.dataset.focusKey = `model:${key}`;
  choice.setAttribute('aria-pressed', String(current));
  choice.disabled = !available || !canSelect(id);
  const marker = element('span', 'model-check', current ? '✓' : '');
  marker.setAttribute('aria-hidden', 'true');
  const lines = element('span', 'model-row-copy');
  lines.append(element('span', 'model-row-name', model.label));
  const detail = model.disambiguator ?? (model.source !== labels[id] ? model.source : '');
  const secondary = [detail, !available ? 'Unavailable' : ''].filter(Boolean).join(' · ');
  if (secondary) lines.append(element('span', 'model-row-source', secondary));
  choice.setAttribute('aria-label', model.label);
  choice.title = [model.label, model.id && model.id !== model.label ? model.id : '', model.description].filter(Boolean).join('\n');
  choice.append(marker, lines);
  const description = [labels[id] === secondary ? '' : labels[id], secondary];
  if (provider.dataset.handoff && provider.dataset.handoff !== id && available && canSelect(id)) {
    const handoff = element('span', 'model-handoff', 'Handoff');
    handoff.title = 'Your next message will hand off the conversation to this provider.';
    choice.append(handoff);
    description.push(handoff.title);
  }
  choice.setAttribute('aria-description', description.filter(Boolean).join('. '));
  row.append(choice);

  if (model.id !== null && preferences && (available || scope === 'favorites')) {
    const modelId = model.id;
    const favored = preferences.favorites.includes(modelId);
    const action = favored ? `Remove ${model.label} from favorites` : `Add ${model.label} to favorites`;
    const star = button('model-favorite', favored ? '★' : '☆', () => {
      preferences.favorites = favored ? preferences.favorites.filter(value => value !== modelId) : [...preferences.favorites, modelId];
      changed(id, false);
      render();
    });
    star.disabled = busy();
    star.dataset.focusKey = `favorite:${key}`;
    star.setAttribute('aria-label', action);
    star.setAttribute('aria-description', [labels[id], model.disambiguator ?? (model.source === labels[id] ? '' : model.source)].filter(Boolean).join(', '));
    star.setAttribute('aria-pressed', String(favored));
    star.title = action;
    row.append(star);
  }
  return row;
}

function announce(message: string): void {
  if (announcements.textContent !== message) announcements.textContent = message;
}
function emptySearch(): HTMLElement {
  return statusBlock('No matching models', 'Try another name or provider.',
    retryButton('Clear search', 'search:clear', () => {
      search.value = '';
      resetResults();
      render();
      search.focus();
    }));
}
function renderResults(): void {
  list.replaceChildren();
  const terms = search.value.trim().toLowerCase().split(/\s+/).filter(Boolean);
  if (pending()) {
    list.append(statusBlock('Checking providers…', 'Looking for Codex and OpenCode.'));
    announce('Checking providers.');
    return;
  }
  const detectionFailed = provider.dataset.pickerStatus === 'error';
  const allMissing = scope === 'all' && !providers.some(installed);
  if (detectionFailed || allMissing) {
    const title = detectionFailed ? 'Provider check failed' : 'No providers found';
    list.append(statusBlock(title, detectionFailed ? 'Refract couldn’t check for Codex or OpenCode.' : 'Install and authenticate Codex or OpenCode to use the assistant.',
      retryButton('Check providers', 'providers:retry', retryProviders)));
    if (scope !== 'favorites') {
      if (terms.length) list.append(emptySearch());
      announce(terms.length ? `No matching models. ${title}.` : title);
      return;
    }
  }
  const ordered = firstProvider === 'opencode' ? ['opencode', 'codex'] as const : providers;
  const visibleProviders = scope === 'all' || scope === 'favorites'
    ? ordered.filter(id => installed(id) || (scope === 'favorites' && (states[id].preferences?.favorites.length || states[id].loadFailed || states[id].loadingPreferences)))
    : [scope];

  let count = 0;
  let hasStatus = false;
  for (const id of visibleProviders) {
    const state = states[id];
    const preferences = state.preferences;
    const ready = state.status === 'ready' && installed(id) && Boolean(preferences);
    const catalog = ready ? state.models : [];
    const availableModels = new Set(catalog);
    let models = [...catalog].sort((a, b) => numeric.compare(a.label, b.label) || numeric.compare(a.id, b.id));
    if (scope === 'favorites') {
      const byId = new Map(models.map(model => [model.id, model]));
      models = (preferences?.favorites ?? []).map(key => byId.get(key) ?? {
        id: key, label: key.split('/').at(-1)!, source: id === 'opencode' ? openCodeSource(key) : undefined, efforts: []
      });
    } else if (id === 'codex' && preferences && installed(id) && !state.loadFailed) {
      models.unshift(cliDefault);
    }
    models = models.filter(model => matchesQuery(model, id, terms));
    const status = state.saveError && id !== activeId()
      ? statusBlock('Settings weren’t saved', 'Retry to save your latest choices.',
        retryButton('Retry saving', `${id}:save`, () => {void save(id); render();}))
      : providerStatus(id);
    if (!models.length && !status) continue;

    const group = element('section', 'model-results-group');
    const heading = element('h3', 'model-group-title', labels[id]);
    heading.prepend(providerLogo(id));
    heading.id = `model-group-${id}`;
    group.setAttribute('aria-labelledby', heading.id);
    group.append(heading);
    for (const model of models) group.append(modelRow(id, model, model.id === null || availableModels.has(model)));
    count += models.length;
    if (status) {
      group.append(status);
      hasStatus = true;
    }
    list.append(group);
  }
  if (!count) {
    if (terms.length) list.append(emptySearch());
    else if (scope === 'favorites' && !hasStatus) list.append(statusBlock('No favorites yet', 'Use the star beside a model to find it here.'));
  }
  syncResultTabStop();
  const loading = visibleProviders.some(id => installed(id) && states[id].status === 'loading');
  const resultText = count ? `${count} ${count === 1 ? 'model' : 'models'} in ${labels[scope]}` :
    loading ? 'Loading models' : terms.length ? 'No matching models' : scope === 'favorites' ? 'No favorites yet' : 'No models available';
  announce(`${resultText}.${hasStatus && !loading ? ' Check provider status in the results.' : ''}`);
}

function renderSettings(): void {
  settings.replaceChildren();
  const id = activeId();
  const state = id && states[id];
  const model = id && selectedModel(id);
  const preferences = state?.preferences;
  const current = element('div', 'model-current');
  current.append(element('h3', '', 'Current selection'));
  settings.append(current);
  if (!id || !installed(id) || pending()) {
    current.append(element('p', '', 'Choose an available model to configure it here.'));
    return;
  }
  const providerLine = element('div', 'model-current-provider', labels[id]);
  providerLine.prepend(providerLogo(id));
  current.append(providerLine);
  if (!model) {
    current.append(element('strong', '', preferences?.model?.split('/').at(-1) ?? 'No model selected'));
    current.append(element('p', '', state.status === 'loading' ? 'Checking model availability.' :
      state.loadFailed ? 'Retry loading preferences in the model list.' : 'Choose an available model to continue.'));
  } else {
    current.append(element('strong', '', model.label));
    const identity = model.disambiguator ?? model.source;
    if (identity && identity !== labels[id]) current.append(element('p', '', identity));
    if (model.description) current.append(element('p', '', model.description));
    if (model.id !== null && state.status === 'loading') current.append(element('p', '', 'Checking model availability.'));
  }
  if (provider.dataset.handoff && provider.dataset.handoff !== id) {
    current.append(element('p', 'model-handoff-note', 'Your next message will hand off this conversation to the selected provider.'));
  }
  if (busy()) current.append(element('p', '', 'Settings are paused while a reply is in progress.'));
  if (state.saveError) {
    const retry = retryButton('Retry saving', `${id}:save`, () => {void save(id); render();});
    retry.disabled = busy();
    settings.append(statusBlock('Settings weren’t saved', 'Retry to save your latest choices before sending.', retry));
  }
  if (!preferences) return;

  if (model?.efforts.length) {
    const reasoning = element('div', 'model-setting');
    const title = id === 'codex' ? 'Reasoning effort' : 'Model variant';
    const label = element('label', 'model-setting-title', title);
    const select = element('select');
    select.id = 'model-effort';
    label.htmlFor = select.id;
    select.dataset.focusKey = 'settings:effort';
    const description = element('p', 'model-setting-description', id === 'codex' ?
      'Higher effort can help with harder tasks and may take longer.' : 'Variants are defined by the model provider.');
    description.id = 'model-effort-help';
    select.setAttribute('aria-describedby', description.id);
    if (id === 'codex') select.add(new Option(model.defaultEffort ? `Default (${effortLabel(model.defaultEffort)})` : 'Model default', ''));
    for (const value of model.efforts) select.add(new Option(effortLabel(value), value));
    if (preferences.reasoningEffort && !model.efforts.includes(preferences.reasoningEffort)) {
      const stale = new Option(`${effortLabel(preferences.reasoningEffort)} (unavailable)`, preferences.reasoningEffort);
      stale.disabled = true;
      select.add(stale);
    }
    select.value = preferences.reasoningEffort ?? '';
    select.disabled = !canSelect(id) || state.status !== 'ready';
    select.onchange = () => {
      preferences.reasoningEffort = select.value || null;
      changed(id);
      render();
    };
    reasoning.append(label, select, description);
    settings.append(reasoning);
  } else if (model && model.id !== null && preferences.reasoningEffort) {
    const reset = retryButton('Use default reasoning', 'settings:reset-effort', () => {
      preferences.reasoningEffort = null;
      changed(id);
      render();
    });
    reset.disabled = !canSelect(id);
    settings.append(statusBlock('Reasoning effort unavailable', 'This model uses its default reasoning.', reset));
  } else if (model && model.id !== null) {
    settings.append(element('p', 'model-setting-description', id === 'codex' ? 'This model uses its default reasoning.' : 'This model offers no variants.'));
  }
  if (id === 'codex') {
    const speedSetting = element('div', 'model-setting');
    const speedLabel = element('div', 'model-setting-title', 'Response speed');
    const speed = element('div', 'model-speed');
    speed.setAttribute('role', 'group');
    speed.setAttribute('aria-label', 'Codex speed');
    for (const [enabled, label] of [[undefined, 'Inherit'], [false, 'Standard'], [true, 'Fast']] as const) {
      const control = button('', label, () => {
        if (preferences.fastMode === enabled) return;
        if (enabled === undefined) delete preferences.fastMode;
        else preferences.fastMode = enabled;
        changed(id);
        render();
      });
      control.dataset.focusKey = `settings:speed:${label}`;
      control.disabled = !canSelect(id);
      control.setAttribute('aria-pressed', String(preferences.fastMode === enabled));
      speed.append(control);
    }
    const hint = element('p', 'model-setting-description', 'Inherit uses the CLI setting. Fast may use more of your limit.');
    hint.id = 'model-speed-help';
    speed.setAttribute('aria-describedby', hint.id);
    speedSetting.append(speedLabel, speed, hint);
    settings.append(speedSetting);
  }
}

function updateDetailsVisibility(): void {
  const compact = panel.classList.contains('model-panel-compact');
  settings.hidden = compact && !detailsOpen;
  panel.classList.toggle('model-panel-details', compact && detailsOpen);
  settingsToggle.setAttribute('aria-expanded', String(!settings.hidden));
  settingsToggle.textContent = settings.hidden ? 'Show current model settings' : 'Back to models';
}

function render(): void {
  updateTrigger();
  if (panel.hidden) return;
  const focused = document.activeElement as HTMLElement;
  const focusKey = panel.contains(focused) ? focused.dataset.focusKey : undefined;
  const oldStars = Array.from(list.querySelectorAll<HTMLButtonElement>('.model-favorite:enabled'));
  const favoriteIndex = oldStars.findIndex(star => star === focused);
  const listTop = list.scrollTop;
  const settingsTop = settingsGroup.scrollTop;
  for (const {id, control} of filterButtons) control.setAttribute('aria-pressed', String(id === scope));
  renderResults();
  renderSettings();
  updateDetailsVisibility();
  if (focusKey) {
    const targets = Array.from(panel.querySelectorAll<HTMLElement>('[data-focus-key]'));
    const replacement = targets.find(control => control.dataset.focusKey === focusKey && !control.matches(':disabled'));
    const fallback = detailsOpen && panel.classList.contains('model-panel-compact') ? settingsToggle : search;
    const stars = list.querySelectorAll<HTMLButtonElement>('.model-favorite:enabled');
    const neighbor = favoriteIndex >= 0 ? stars[Math.min(favoriteIndex, stars.length - 1)] : undefined;
    (replacement ?? neighbor ?? fallback).focus({preventScroll: true});
  }
  list.scrollTop = listTop;
  settingsGroup.scrollTop = settingsTop;
  positionPanel();
}
function resultButtons(): HTMLButtonElement[] {
  return Array.from(list.querySelectorAll<HTMLButtonElement>('.model-choice:enabled'));
}
function syncResultTabStop(): void {
  const choices = resultButtons();
  const target = choices.find(choice => choice.dataset.result === resultKey) ??
    choices.find(choice => choice.getAttribute('aria-pressed') === 'true') ?? choices[0];
  for (const choice of choices) {
    choice.tabIndex = choice === target ? 0 : -1;
    const favorite = choice.parentElement!.querySelector<HTMLButtonElement>('.model-favorite');
    if (favorite) favorite.tabIndex = choice === target ? 0 : -1;
  }
}
function resetResults(): void {
  resultKey = undefined;
  list.scrollTop = 0;
}
function positionPanel(): void {
  if (panel.hidden) return;
  const anchor = trigger.getBoundingClientRect();
  const margin = 10;
  const gap = 8;
  const width = Math.min(604, innerWidth - margin * 2);
  const above = anchor.top - margin - gap;
  const below = innerHeight - anchor.bottom - margin - gap;
  const overlay = Math.max(above, below) < 310;
  const placeAbove = above >= 450 || above >= below;
  const available = overlay ? innerHeight - margin * 2 : placeAbove ? above : below;
  const compact = width < 470;
  if (panel.classList.contains('model-panel-compact') !== compact) {
    const focusInSettings = settings.contains(document.activeElement);
    const toggleFocused = document.activeElement === settingsToggle;
    panel.classList.toggle('model-panel-compact', compact);
    if (compact && focusInSettings) detailsOpen = true;
    updateDetailsVisibility();
    if (!compact && toggleFocused) {
      (settings.querySelector<HTMLElement>('select:enabled,button:enabled') ?? search).focus();
    }
  }
  panel.style.width = `${width}px`;
  const maxHeight = `${Math.max(0, Math.min(490, available))}px`;
  panel.style.maxHeight = maxHeight;
  panel.style.setProperty('--picker-height', maxHeight);
  panel.style.left = `${Math.max(margin, Math.min(anchor.right - width, innerWidth - width - margin))}px`;
  panel.style.top = overlay ? `${margin}px` : placeAbove ? 'auto' : `${anchor.bottom + gap}px`;
  panel.style.bottom = !overlay && placeAbove ? `${innerHeight - anchor.top + gap}px` : 'auto';
}
let positionFrame = 0;
let anchorGeometry = '';
function followAnchor(): void {
  const {x, y, width, height} = trigger.getBoundingClientRect();
  const geometry = `${x},${y},${width},${height},${innerWidth},${innerHeight}`;
  if (geometry !== anchorGeometry) {
    anchorGeometry = geometry;
    positionPanel();
  }
  positionFrame = requestAnimationFrame(followAnchor);
}
function dismiss(restoreFocus = false): void {
  if (panel.hidden) return;
  panel.hidden = true;
  cancelAnimationFrame(positionFrame);
  trigger.setAttribute('aria-expanded', 'false');
  if (restoreFocus) trigger.focus();
}
function close(): void { dismiss(true); }
function toggle(): void {
  if (!panel.hidden) return close();
  panel.hidden = false;
  trigger.setAttribute('aria-expanded', 'true');
  scope = 'all';
  firstProvider = activeId() ?? 'codex';
  detailsOpen = false;
  search.value = '';
  selectionAnnouncement.textContent = '';
  resetResults();
  render();
  followAnchor();
  search.focus({preventScroll: true});
}
search.oninput = () => {resetResults(); render();};
list.addEventListener('focusin', event => {
  const choice = (event.target as HTMLElement).closest('.model-row')?.querySelector<HTMLButtonElement>('.model-choice:enabled');
  if (choice) {
    resultKey = choice.dataset.result;
    syncResultTabStop();
  }
});
host.addEventListener('keydown', event => {
  if (panel.hidden) return;
  if (event.isComposing || event.keyCode === 229) return;
  if (event.key === 'Escape') {
    event.preventDefault();
    close();
    return;
  }
  const focused = document.activeElement as HTMLElement;
  const choice = focused.closest('.model-row')?.querySelector<HTMLButtonElement>('.model-choice:enabled');
  if (focused !== search && !choice) return;
  const choices = resultButtons();
  if (!choices.length) return;
  if (focused === search && event.key === 'Enter' && search.value.trim()) {
    event.preventDefault();
    choices[0].focus();
    if (choices.length === 1) choices[0].click();
  } else if (event.key === 'ArrowDown' || event.key === 'ArrowUp' || (choice && ['Home', 'End'].includes(event.key))) {
    event.preventDefault();
    const index = choice ? choices.indexOf(choice) : -1;
    const next = event.key === 'Home' ? 0 : event.key === 'End' ? choices.length - 1 :
      event.key === 'ArrowDown' ? (index + 1) % choices.length : index < 0 ? choices.length - 1 : (index - 1 + choices.length) % choices.length;
    choices[next].focus();
  } else if (choice && focused === choice && event.key === 'ArrowRight') {
    choice.parentElement!.querySelector<HTMLButtonElement>('.model-favorite:enabled')?.focus();
  } else if (choice && focused !== choice && event.key === 'ArrowLeft') {
    choice.focus();
  }
});
document.addEventListener('focusin', event => {
  if (event.target instanceof Node && !host.contains(event.target)) dismiss();
});
document.addEventListener('pointerdown', event => {
  if (!event.composedPath().includes(host)) dismiss();
});
new IntersectionObserver(entries => {
  if (entries.some(entry => !entry.isIntersecting)) dismiss();
}).observe(trigger);

async function discover(id: Provider): Promise<Model[]> {
  if (id === 'codex') {
    const result = await api.discoverCodexModels();
    if (result.ok === false) throw new Error(result.error);
    return result.models.map(model => ({
      id: model.model, label: model.displayName, description: model.description,
      efforts: model.supportedReasoningEfforts.map(option => option.effort).filter(value => effortOrder.includes(value) && value !== 'none'),
      defaultEffort: model.defaultReasoningEffort
    }));
  }
  const result = await api.discoverOpenCodeModels(project);
  if (result.ok === false) throw new Error(result.error);
  return result.models.map(model => {
    return {id: model.id, label: model.name, source: openCodeSource(model.id), efforts: model.variants};
  });
}
function loadPreferences(id: Provider): Promise<Preferences | undefined> {
  const state = states[id];
  if (state.preferences) return Promise.resolve(state.preferences);
  if (state.loadingPreferences) return state.loadingPreferences;
  const read = id === 'codex' ? () => api.codexPreferencesLoad() : () => api.openCodePreferencesLoad();
  state.loadFailed = false;
  state.loadingPreferences = Promise.resolve().then(read).then(value => {
    state.preferences = value;
    return value;
  }).catch(() => {
    state.loadFailed = true;
    return undefined;
  }).finally(() => {state.loadingPreferences = undefined;});
  return state.loadingPreferences;
}
function distinguishNames(models: Model[]): Model[] {
  const nameCounts = new Map<string, number>();
  const sourceCounts = new Map<string, number>();
  for (const model of models) {
    const key = JSON.stringify([model.label, model.source]);
    nameCounts.set(model.label, (nameCounts.get(model.label) ?? 0) + 1);
    sourceCounts.set(key, (sourceCounts.get(key) ?? 0) + 1);
  }
  for (const model of models) {
    if (nameCounts.get(model.label) === 1) continue;
    const key = JSON.stringify([model.label, model.source]);
    model.disambiguator = model.source && sourceCounts.get(key) === 1 ? model.source : model.id;
  }
  return models;
}

async function initialize(id: Provider, request: number): Promise<void> {
  const state = states[id];
  const [, catalog] = await Promise.allSettled([loadPreferences(id), discover(id)]);
  if (id === 'opencode' && request !== generation) return;
  state.status = catalog.status === 'fulfilled' ? 'ready' : 'error';
  state.models = catalog.status === 'fulfilled' ? distinguishNames(catalog.value) : [];
  state.error = catalog.status === 'rejected' && catalog.reason instanceof Error ? catalog.reason.message : undefined;

  if (id === 'opencode' && state.preferences && state.status === 'ready') {
    const {preferences} = state;
    let modified = false;
    if (!preferences.model && state.models.length) {
      const version = (model: Model) => (model.label.match(/\d+(?:\.\d+)*/)?.[0] ?? model.id.match(/\d+(?:\.\d+)*/)?.[0] ?? '0');
      preferences.model = [...state.models].sort((a, b) => numeric.compare(version(b), version(a)) || numeric.compare(a.label, b.label))[0].id;
      modified = true;
    }
    const model = selectedModel(id);
    if (model && !model.efforts.includes(preferences.reasoningEffort ?? '')) {
      const fallback = preferredEffort(model);
      if (preferences.reasoningEffort !== fallback) {
        preferences.reasoningEffort = fallback;
        modified = true;
      }
    }
    if (modified) changed(id);
  }
  render();
}
function startDiscovery(id: Provider): void {
  const state = states[id];
  state.status = 'loading';
  state.loadFailed = false;
  if (id === 'opencode') catalogProject = project;
  state.initialized = initialize(id, id === 'opencode' ? ++generation : 0);
  render();
}
function syncProviders(): void {
  if (!pending()) {
    const available = new Set(providers.filter(installed));
    const previous = detected;
    detected = available;
    for (const id of available) {
      if (!previous.has(id) || !states[id].initialized || (id === 'opencode' && catalogProject !== project)) {
        startDiscovery(id);
      }
    }
  }
  render();
}
provider.addEventListener('providerstate', syncProviders);
provider.addEventListener('change', render);
window.addEventListener('picker-project', (event: CustomEvent<string>) => {
  if (event.detail === project) return;
  project = event.detail;
  if (states.opencode.initialized && installed('opencode')) startDiscovery('opencode');
});
syncProviders();

pickerWindow.codexPickerReady = async (id = activeId()) => {
  if (!id || !installed(id)) throw new Error('Choose an available provider before sending.');
  const state = states[id];
  const initialized = state.initialized;
  await initialized;
  if (id === 'opencode' && initialized !== state.initialized) {
    throw new Error('The OpenCode catalog changed. Try sending again from this project.');
  }
  const issue = sendIssue(id);
  if (issue) throw new Error(`Open the model picker and ${issue.message[0].toLowerCase()}${issue.message.slice(1)} before sending.`);
  if (!state.preferences) throw new Error('Open the model picker and retry loading preferences.');
  const {model: modelId, reasoningEffort, fastMode} = state.preferences;
  const revision = state.selectionRevision;
  while (state.savedRevision < revision && state.write) {
    if (!await state.write) break;
  }
  if (state.savedRevision < revision) throw new Error('Open the model picker and retry saving before sending.');
  return {model: modelId, reasoningEffort, ...(id === 'codex' && fastMode !== undefined ? {fastMode} : {})};
};
})();
