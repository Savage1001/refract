(() => {
type Provider = 'codex' | 'opencode';
type Scope = 'all' | 'favorites' | Provider;
type Preferences = import('./src/codex-preferences.cjs').CodexPreferences;
type Selection = Omit<Preferences, 'favorites'>;
type CodexMode = import('./src/codex-account.cjs').CodexMode;
type Discovery<T> = {ok: true; models: T[]} | {ok: false; error: string};
interface Model {
  id: string;
  label: string;
  source?: string;
  description?: string;
  efforts: string[];
  defaultEffort?: string;
  isDefault?: boolean;
  disambiguator?: string;
}
interface State {
  mode?: CodexMode;
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
  discoverCodexModels(mode?: CodexMode): Promise<Discovery<import('./src/codex-models.cjs').CodexModel>>;
  discoverOpenCodeModels(project?: string): Promise<Discovery<{id: string; name: string; variants: string[]}>>;
  codexPreferencesLoad(mode?: CodexMode): Promise<Preferences>;
  openCodePreferencesLoad(): Promise<Preferences>;
  codexPreferencesSave(value: Preferences, mode?: CodexMode): Promise<void>;
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
function formatModelName(name: string): string {
  return name.replace(/-+/g, ' ').trim();
}

function modelNameFromId(id: string): string {
  return formatModelName(id.slice(id.lastIndexOf('/') + 1));
}

function openCodeSource(id: string): string {
  const prefix = id.split('/')[0];
  return sourceNames[prefix.toLowerCase()] ?? prefix;
}
const effortOrder = ['none', 'minimal', 'low', 'medium', 'high', 'xhigh', 'max', 'ultra'];
const numeric = new Intl.Collator(undefined, {numeric: true, sensitivity: 'base'});
const newState = (): State => ({models: [], status: 'loading', revision: 0, savedRevision: 0, selectionRevision: 0});
const states = {codex: newState(), opencode: newState()};
let scope: Scope = 'all';
let openingScopePending = false;
let firstProvider: Provider = 'codex';
let project: string | undefined;
let generation = 0;
let codexGeneration = 0;
const codexStates: Record<CodexMode, State> = {
  cli: {...newState(), mode: 'cli'},
  account: {...newState(), mode: 'account'},
};
let detected = new Set<Provider>();
let resultKey: string | undefined;
let detailsOpen = false;

function element<K extends keyof HTMLElementTagNameMap>(tag: K, className = '', content?: string): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  node.className = className;
  if (content) node.textContent = content;
  return node;
}
function button(className: string, label: string, onClick: (event: MouseEvent) => void): HTMLButtonElement {
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
function defaultEffort(model: Model): string | null {
  if (model.efforts.includes('low')) return 'low';
  if (model.defaultEffort && model.efforts.includes(model.defaultEffort)) return model.defaultEffort;
  return effortOrder.find(effort => model.efforts.includes(effort)) ?? model.efforts[0] ?? null;
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
  return models.find(model => model.id === preferences?.model);
}
function modelKey(id: Provider, model: string): string { return JSON.stringify([id, model]); }

const trigger = button('model-trigger', '', toggle);
trigger.setAttribute('aria-haspopup', 'dialog');
trigger.setAttribute('aria-expanded', 'false');
trigger.setAttribute('aria-controls', 'model-picker-panel');
const panel = element('div', 'model-panel');
panel.id = 'model-picker-panel';
panel.hidden = true;
panel.setAttribute('role', 'dialog');
panel.setAttribute('aria-label', 'Choose a model');
const done = button('model-done', '×', close);
done.setAttribute('aria-label', 'Close model picker');
done.title = 'Close model picker (Esc)';
const searchBar = element('div', 'model-search');
const search = element('input');
search.type = 'search';
search.placeholder = 'Search models…';
search.setAttribute('aria-label', 'Search models');
search.setAttribute('aria-controls', 'model-picker-results');
searchBar.append(element('span', 'model-search-icon'), search);
const filters = element('div', 'model-filters');
filters.setAttribute('role', 'group');
filters.setAttribute('aria-label', 'Filter models');
const filterButtons = (['favorites', ...providers] as const).map(id => {
  const control = button('model-filter', '', () => {
    openingScopePending = false;
    scope = id;
    detailsOpen = false;
    search.value = '';
    resetResults();
    if (id === 'favorites') {
      for (const provider of providers) {
        const state = states[provider];
        if (!state.preferences && !state.initialized) void loadPreferences(provider).then(render);
      }
    }
    render();
    if (!matchMedia('(pointer: coarse)').matches) search.focus({preventScroll: true});
  });
  control.dataset.scope = id;
  control.title = labels[id];
  control.setAttribute('aria-label', labels[id]);
  if (id === 'codex' || id === 'opencode') control.append(providerLogo(id));
  if (id === 'favorites') {
    control.setAttribute('aria-label', 'Favorite models');
    control.append(element('span', 'model-filter-star'));
  }
  if (id === 'codex' || id === 'opencode') control.setAttribute('aria-label', `${labels[id]} models`);
  filters.append(control);
  return {id, control};
});

filters.addEventListener('keydown', event => {
  if (event.key === 'ArrowRight') {
    event.preventDefault();
    search.focus({preventScroll: true});
    return;
  }
  if (!['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(event.key)) return;
  const index = filterButtons.findIndex(({control}) => control === document.activeElement);
  if (index < 0) return;
  event.preventDefault();
  const count = filterButtons.length;
  const next = event.key === 'Home' ? 0 : event.key === 'End' ? count - 1 :
    (index + (event.key === 'ArrowDown' ? 1 : -1) + count) % count;
  filterButtons[next].control.focus();
});

const body = element('div', 'model-content');
const list = element('div', 'model-list');
list.id = 'model-picker-results';
list.setAttribute('role', 'region');
list.setAttribute('aria-label', 'Model results');
const settings = element('section', 'model-settings');
settings.setAttribute('aria-label', 'Current model settings');
settings.id = 'model-picker-settings';
const settingsToggle = button('model-settings-toggle', 'Settings', () => {
  detailsOpen = !detailsOpen;
  updateDetailsVisibility();
  if (detailsOpen) settings.querySelector<HTMLElement>('[tabindex="0"], .model-retry:enabled')?.focus();
  else search.focus();
});
settingsToggle.setAttribute('aria-controls', settings.id);
const settingsGroup = element('div', 'model-settings-group');
settingsGroup.append(settings);
const catalogPane = element('div', 'model-catalog');
catalogPane.append(searchBar, list);
body.append(filters, catalogPane, settingsGroup);
const footer = element('div', 'model-footer');
const speedSlot = element('div', 'model-speed-slot');
footer.append(speedSlot, settingsToggle, done);
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
  if ((issue && issue.area === 'settings') || (!issue && id === activeId() && installed(id))) {
    detailsOpen = true;
    updateDetailsVisibility();
    (settings.querySelector<HTMLElement>('.model-retry:enabled,[role=radio]:enabled') ?? settingsToggle).focus();
  } else {
    openingScopePending = false;
    scope = !issue && !installed(id) ? 'favorites' : id;
    resetResults();
    render();
    const retry = Array.from(list.querySelectorAll<HTMLElement>('.model-retry:enabled'));
    const action = !issue ? retry.find(control => control.dataset.focusKey === `${id}:save`) : retry[0];
    (action ?? search).focus();
  }
}
notice.hidden = true;
panel.append(body, footer, announcements, selectionAnnouncement);
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
        id === 'codex' ? api.codexPreferencesSave(value, state.mode) : api.openCodePreferencesSave(value)
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
  if (preferences.model !== model.id) {
    preferences.model = model.id;
    preferences.reasoningEffort = defaultEffort(model);
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
  if (state.status === 'error' && (id === 'opencode' || !state.preferences.model))
    return {message: `Retry loading ${labels[id]} models`, area: 'models'};
  if (!model && state.status === 'ready') return {message: `Choose an available ${labels[id]} model`, area: 'models'};
  if (model && state.preferences.reasoningEffort && !model.efforts.includes(state.preferences.reasoningEffort))
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
  const signInRequired = !pending() && (!id || (id === 'codex' && !installed(id))) &&
    provider.dataset.pickerStatus !== 'error' && provider.dataset.codexSignInRequired === 'true';
  const state = id && states[id];
  const model = id && selectedModel(id);
  const preferences = state?.preferences;
  let label = 'Choose a model';
  if (pending()) label = 'Checking providers…';
  else if (signInRequired) label = 'Sign in';
  else if (!id) label = provider.dataset.pickerStatus === 'error' ? 'Provider check failed' : 'No providers found';
  else if (!installed(id)) label = `${labels[id]} unavailable`;
  else if (state.loadFailed) label = 'Preferences unavailable';
  else if (model) label = model.label;
  else if (state.status === 'loading') label = preferences?.model ? modelNameFromId(preferences.model) : 'Loading models…';
  else if (preferences?.model) label = `${modelNameFromId(preferences.model)} (unavailable)`;
  else if (state.status === 'error') label = 'Models unavailable';

  const configuredEffort = preferences?.reasoningEffort;
  const effort = configuredEffort && model?.efforts.includes(configuredEffort) && effortLabel(configuredEffort);
  const speed = id === 'codex' && preferences?.fastMode ? 'Fast' : '';
  const options = [effort, speed].filter(Boolean).join(' · ');
  const identity = model?.disambiguator ?? model?.source ?? '';
  const fullLabel = [id && installed(id) ? labels[id] : '', label, identity !== labels[id] || model?.disambiguator ? identity : '', options].filter(Boolean).join(' · ');
  trigger.replaceChildren();
  const logo = signInRequired ? 'codex' : id;
  if (logo) trigger.append(providerLogo(logo));
  trigger.append(element('span', 'model-trigger-label', label));
  if (model?.disambiguator) trigger.append(element('span', 'model-trigger-options', identity));
  if (options && model && installed(id)) trigger.append(element('span', 'model-trigger-options', options));
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
function codexSignInStatus(): HTMLElement {
  const busy = provider.dataset.codexSigningIn === 'true';
  const control = retryButton(busy ? 'Connecting…' : 'Sign in with ChatGPT', 'codex:sign-in',
    () => provider.dispatchEvent(new Event('picker-sign-in')));
  control.classList.add('model-sign-in');
  control.prepend(providerLogo('codex'));
  control.disabled = busy;
  return statusBlock('Codex requires sign-in', provider.dataset.codexSignInError ||
    (busy ? 'Preparing ChatGPT sign-in. Follow the prompts in your browser.' : 'Connect your ChatGPT account to use Codex.'), control);
}
function providerStatus(id: Provider): HTMLElement | undefined {
  const state = states[id];
  if (id === 'codex' && provider.dataset.pickerStatus !== 'error' &&
      provider.dataset.codexSignInRequired === 'true') return codexSignInStatus();
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
  const label = model.label.toLowerCase();
  return terms.every(term => {
    const normalized = formatModelName(term);
    return text.includes(term) || (normalized !== '' && label.includes(normalized));
  });
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
  const lines = element('span', 'model-row-copy');
  const line = element('span', 'model-row-line');
  line.append(element('span', 'model-row-name', model.label));
  if (current) {
    const marker = element('span', 'model-check', '✓');
    marker.setAttribute('aria-hidden', 'true');
    line.append(marker);
  }
  lines.append(line);
  const detail = model.disambiguator ?? (model.source !== labels[id] ? model.source : '');
  const secondary = [detail, !available ? 'Unavailable' : ''].filter(Boolean).join(' · ');
  if (secondary && (model.disambiguator || !available)) lines.append(element('span', 'model-row-source', secondary));
  choice.setAttribute('aria-label', model.label);
  choice.title = [model.label, model.source !== labels[id] ? model.source : '', model.id !== model.label ? model.id : '', model.description].filter(Boolean).join('\n');
  choice.append(providerLogo(id), lines);
  const description = [labels[id] === secondary ? '' : labels[id], secondary];
  if (provider.dataset.handoff && provider.dataset.handoff !== id && available && canSelect(id)) {
    const handoff = element('span', 'model-handoff', 'Handoff');
    handoff.title = 'Your next message will hand off the conversation to this provider.';
    choice.append(handoff);
    description.push(handoff.title);
  }
  choice.setAttribute('aria-description', description.filter(Boolean).join('. '));
  row.append(choice);

  if (preferences && (available || scope === 'favorites')) {
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
      search.dispatchEvent(new Event('input'));
      search.focus();
    }));
}
function renderResults(): void {
  list.replaceChildren();
  const terms = search.value.trim().toLowerCase().split(/\s+/).filter(Boolean);
  const resultScope = terms.length ? 'all' : scope;
  if (pending()) {
    list.append(statusBlock('Checking providers…', 'Looking for Codex and OpenCode.'));
    announce('Checking providers.');
    if (resultScope !== 'favorites') return;
  }
  const detectionFailed = provider.dataset.pickerStatus === 'error';
  const allMissing = resultScope === 'all' && !providers.some(installed);
  if (detectionFailed || allMissing) {
    const needsCodexSetup = !detectionFailed && provider.dataset.codexSignInRequired === 'true';
    const needsRuntime = !detectionFailed && provider.dataset.codexSetupRequired === 'true';
    const title = needsCodexSetup ? 'Codex requires sign-in' : detectionFailed ? 'Provider check failed'
      : needsRuntime ? 'Connect Codex in Settings' : 'No providers found';
    list.append(needsCodexSetup ? codexSignInStatus() : statusBlock(title, detectionFailed ? 'Refract couldn’t check for Codex or OpenCode.' : needsRuntime ? 'Open Settings on the home screen to finish setting up Codex.'
      : 'Install and authenticate Codex or OpenCode to use the assistant.',
      retryButton('Check providers', 'providers:retry', retryProviders)));
    if (resultScope !== 'favorites') {
      if (terms.length) list.append(emptySearch());
      announce(terms.length ? `No matching models. ${title}.` : title);
      return;
    }
  }
  const ordered = firstProvider === 'opencode' ? ['opencode', 'codex'] as const : providers;
  const visibleProviders = resultScope === 'all' || resultScope === 'favorites'
    ? ordered.filter(id => installed(id) || (resultScope === 'all' && id === 'codex' && provider.dataset.codexSignInRequired === 'true') || (resultScope === 'favorites' && (states[id].preferences?.favorites.length || states[id].saveError || states[id].loadFailed || states[id].loadingPreferences)))
    : [resultScope];

  let count = 0;
  let hasStatus = false;
  for (const id of visibleProviders) {
    const state = states[id];
    const preferences = state.preferences;
    const ready = state.status === 'ready' && installed(id) && Boolean(preferences);
    const catalog = ready ? state.models : [];
    const availableIds = new Set(catalog.map(model => model.id));
    let models = [...catalog];
    if (id === 'opencode') models.sort((a, b) => numeric.compare(a.label, b.label) || numeric.compare(a.id, b.id));
    if (resultScope === 'favorites') {
      const byId = new Map(models.map(model => [model.id, model]));
      models = distinguishNames((preferences?.favorites ?? []).map(key => {
        const known = byId.get(key);
        if (known) return {...known};
        return {id: key, label: modelNameFromId(key), source: id === 'opencode' ? openCodeSource(key) : undefined, efforts: []};
      }));
    }
    models = models.filter(model => matchesQuery(model, id, terms));
    const status = state.saveError && (id !== activeId() || !installed(id))
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
    for (const model of models) group.append(modelRow(id, model, availableIds.has(model.id)));
    count += models.length;
    if (status) {
      group.append(status);
      hasStatus = true;
    }
    list.append(group);
  }
  if (!count) {
    if (terms.length) list.append(emptySearch());
    else if (resultScope === 'favorites' && !hasStatus) list.append(statusBlock('No favorites yet', 'Use the star beside a model to find it here.'));
  }
  bindResultShortcuts();
  syncResultTabStop();
  const loading = visibleProviders.some(id => installed(id) && states[id].status === 'loading');
  const resultText = count ? `${count} ${count === 1 ? 'model' : 'models'} in ${labels[resultScope]}` :
    loading ? 'Loading models' : terms.length ? 'No matching models' : resultScope === 'favorites' ? 'No favorites yet' : 'No models available';
  announce(`${resultText}.${hasStatus && !loading ? ' Check provider status in the results.' : ''}`);
}

function renderSettings(): void {
  settings.replaceChildren();
  speedSlot.replaceChildren();
  const id = activeId();
  const state = id && states[id];
  const model = id && selectedModel(id);
  const preferences = state?.preferences;
  const heading = element('div', 'model-settings-heading');
  const headingLabel = id === 'opencode' ? 'Variant' : 'Reasoning';
  heading.append(element('h3', '', headingLabel));
  settings.append(heading);
  if (!id || !installed(id) || pending()) {
    settings.append(element('p', 'model-setting-description', 'Choose an available model to configure it here.'));
    return;
  }

  const current = element('strong', 'model-settings-model', model?.label ?? (preferences?.model ? modelNameFromId(preferences.model) : 'No model selected'));
  current.title = model ? [model.label, model.disambiguator ?? model.source, model.description].filter(Boolean).join('\n') : current.textContent;
  heading.append(current);
  const identity = model?.disambiguator ?? model?.source;
  if (identity && identity !== labels[id]) settings.append(element('p', 'model-settings-identity', identity));
  if (!model || state.status === 'loading') {
    settings.append(element('p', 'model-setting-description', state.status === 'loading' ? 'Checking model availability.' :
      state.loadFailed ? 'Retry loading preferences in the model list.' : 'Choose an available model to continue.'));
  }
  if (provider.dataset.handoff && provider.dataset.handoff !== id) {
    settings.append(element('p', 'model-handoff-note', 'Your next message will hand off this conversation to the selected provider.'));
  }
  if (busy()) settings.append(element('p', 'model-setting-description', 'Settings are paused while a reply is in progress.'));
  if (state.saveError) {
    const retry = retryButton('Retry saving', `${id}:save`, () => {void save(id); render();});
    retry.disabled = busy();
    settings.append(statusBlock('Settings weren’t saved', 'Retry to save your latest choices before sending.', retry));
  }
  if (!preferences) return;

  if (model?.efforts.length) {
    const group = element('div', 'model-efforts');
    group.setAttribute('role', 'radiogroup');
    group.setAttribute('aria-label', id === 'codex' ? 'Reasoning effort' : 'Model variant');
    for (const value of model.efforts) {
      const control = button('model-effort', effortLabel(value), () => {
        if (preferences.reasoningEffort === value) return;
        preferences.reasoningEffort = value;
        changed(id);
        render();
      });
      const checked = preferences.reasoningEffort === value;
      control.dataset.focusKey = `settings:effort:${value}`;
      control.setAttribute('role', 'radio');
      control.setAttribute('aria-checked', String(checked));
      control.disabled = !canSelect(id) || state.status !== 'ready';
      control.tabIndex = -1;
      if (value === defaultEffort(model)) {
        control.setAttribute('aria-description', id === 'codex' ? 'Default reasoning effort' : 'Default model variant');
        const tag = element('span', 'model-effort-default', 'Default');
        tag.setAttribute('aria-hidden', 'true');
        control.append(tag);
      }
      if (checked) {
        const mark = element('span', 'model-effort-mark', '✓');
        mark.setAttribute('aria-hidden', 'true');
        control.append(mark);
      }
      group.append(control);
    }
    const tabStop = group.querySelector<HTMLButtonElement>('[aria-checked=true]:enabled') ?? group.querySelector<HTMLButtonElement>('button:enabled');
    if (tabStop) tabStop.tabIndex = 0;
    group.onkeydown = event => {
      if (!['ArrowDown', 'ArrowUp', 'ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(event.key)) return;
      const controls = Array.from(group.querySelectorAll<HTMLButtonElement>('button:enabled'));
      const index = controls.indexOf(document.activeElement as HTMLButtonElement);
      if (index < 0) return;
      event.preventDefault();
      const direction = ['ArrowDown', 'ArrowRight'].includes(event.key) ? 1 : -1;
      const next = event.key === 'Home' ? 0 : event.key === 'End' ? controls.length - 1 :
        (index + direction + controls.length) % controls.length;
      controls[next].focus();
      controls[next].click();
    };
    settings.append(group);
  } else if (model) {
    const message = id === 'codex' ? 'Reasoning is configured by this model.' : 'This model offers no variants.';
    settings.append(element('p', 'model-setting-description', message));
  }
  if (id === 'codex') {
    const speed = element('div', 'model-speed');
    speed.setAttribute('role', 'group');
    speed.setAttribute('aria-label', 'Codex speed');
    speed.setAttribute('aria-description', 'Fast may use more of your limit.');
    for (const [enabled, label] of [[false, 'Standard'], [true, 'Fast']] as const) {
      const control = button('', label, () => {
        if ((preferences.fastMode ?? false) === enabled) return;
        preferences.fastMode = enabled;
        changed(id);
        render();
      });
      control.dataset.focusKey = `settings:speed:${label}`;
      control.disabled = !canSelect(id);
      control.setAttribute('aria-pressed', String((preferences.fastMode ?? false) === enabled));
      if (label === 'Fast') {
        control.title = 'Fast may use more of your limit.';
        const bolt = element('span', 'model-speed-bolt', '⚡︎');
        bolt.setAttribute('aria-hidden', 'true');
        control.prepend(bolt);
      }
      speed.append(control);
    }
    speedSlot.append(speed);
  }
}
function updateDetailsVisibility(): void {
  const compact = panel.classList.contains('model-panel-compact');
  settings.hidden = compact && !detailsOpen;
  panel.classList.toggle('model-panel-details', compact && detailsOpen);
  settingsToggle.setAttribute('aria-expanded', String(!settings.hidden));
  const setting = activeId() === 'opencode' ? 'Variant' : 'Reasoning';
  settingsToggle.textContent = settings.hidden ? setting : 'Models';
  settingsToggle.setAttribute('aria-label', settings.hidden ? 'Show ' + setting.toLowerCase() + ' settings' : 'Back to models');
}

function render(): void {
  updateTrigger();
  if (panel.hidden) return;
  if (openingScopePending) {
    const id = activeId();
    const selected = id && states[id].preferences;
    scope = selected?.model && selected.favorites.includes(selected.model) ? 'favorites' : id ?? 'all';
    firstProvider = id ?? 'codex';
    openingScopePending = pending() || !selected;
  }
  const focused = document.activeElement as HTMLElement;
  const focusKey = panel.contains(focused) ? focused.dataset.focusKey : undefined;
  const oldStars = Array.from(list.querySelectorAll<HTMLButtonElement>('.model-favorite:enabled'));
  const favoriteIndex = oldStars.findIndex(star => star === focused);
  const listTop = list.scrollTop;
  const settingsTop = settingsGroup.scrollTop;
  const searching = Boolean(search.value.trim());
  panel.dataset.scope = searching ? 'all' : scope;
  panel.toggleAttribute('data-searching', searching);
  filters.hidden = searching;
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
function bindResultShortcuts(): void {
  resultButtons().slice(0, 9).forEach((choice, index) => {
    const shortcut = index + 1;
    choice.setAttribute('aria-keyshortcuts', `Control+${shortcut}`);
    const badge = element('kbd', 'model-shortcut', `Ctrl+${shortcut}`);
    badge.setAttribute('aria-hidden', 'true');
    choice.parentElement!.append(badge);
  });
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
  const width = Math.min(554, innerWidth - margin * 2);
  const above = anchor.top - margin - gap;
  const below = innerHeight - anchor.bottom - margin - gap;
  const overlay = Math.max(above, below) < 310;
  const placeAbove = above >= 450 || above >= below;
  panel.dataset.placement = overlay ? 'overlay' : placeAbove ? 'above' : 'below';
  const available = overlay ? innerHeight - margin * 2 : placeAbove ? above : below;
  const compact = width < 470;
  if (panel.classList.contains('model-panel-compact') !== compact) {
    const focusInSettings = settings.contains(document.activeElement);
    const toggleFocused = document.activeElement === settingsToggle;
    panel.classList.toggle('model-panel-compact', compact);
    if (compact && focusInSettings) detailsOpen = true;
    updateDetailsVisibility();
    if (!compact && toggleFocused) {
      (settings.querySelector<HTMLElement>('[role=radio][aria-checked=true]:enabled') ??
        settings.querySelector<HTMLElement>('button:enabled') ?? search).focus();
    }
  }
  panel.style.width = `${width}px`;
  const maxHeight = `${Math.max(0, Math.min(414, available))}px`;
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
  openingScopePending = true;
  detailsOpen = false;
  search.value = '';
  selectionAnnouncement.textContent = '';
  resetResults();
  render();
  followAnchor();
  search.focus({preventScroll: true});
}
search.oninput = () => {
  resetResults();
  render();
};
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
  if (event.ctrlKey && !event.altKey && !event.shiftKey && !event.metaKey && /^[1-9]$/.test(event.key)
      && !panel.classList.contains('model-panel-details')) {
    const control = list.querySelector<HTMLButtonElement>(`[aria-keyshortcuts="Control+${event.key}"]:enabled`);
    if (control) {
      event.preventDefault();
      control.focus();
      control.click();
    }
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
    const result = await api.discoverCodexModels(states.codex.mode);
    if (result.ok === false) throw new Error(result.error);
    return result.models.map(model => ({
      id: model.model, label: formatModelName(model.displayName), description: model.description,
      efforts: model.supportedReasoningEfforts.map(option => option.effort).filter(value => effortOrder.includes(value) && value !== 'none'),
      defaultEffort: model.defaultReasoningEffort ?? undefined, isDefault: model.isDefault
    }));
  }
  const result = await api.discoverOpenCodeModels(project);
  if (result.ok === false) throw new Error(result.error);
  return result.models.map(model => {
    return {id: model.id, label: formatModelName(model.name), source: openCodeSource(model.id), efforts: model.variants};
  });
}
function loadPreferences(id: Provider): Promise<Preferences | undefined> {
  const state = states[id];
  if (state.preferences) return Promise.resolve(state.preferences);
  if (state.loadingPreferences) return state.loadingPreferences;
  const read = id === 'codex' ? () => api.codexPreferencesLoad(state.mode) : () => api.openCodePreferencesLoad();
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
    model.disambiguator ??= model.source && sourceCounts.get(key) === 1 ? model.source : model.id;
  }
  return models;
}

async function initialize(id: Provider, request: number): Promise<void> {
  const state = states[id];
  const [, catalog] = await Promise.allSettled([loadPreferences(id), discover(id)]);
  if (request !== (id === 'codex' ? codexGeneration : generation) || !installed(id)) return;
  state.status = catalog.status === 'fulfilled' ? 'ready' : 'error';
  state.models = catalog.status === 'fulfilled' ? distinguishNames(catalog.value) : [];
  state.error = catalog.status === 'rejected' && catalog.reason instanceof Error ? catalog.reason.message : undefined;

  if (state.preferences && state.status === 'ready') {
    const {preferences} = state;
    let modified = false;
    let model = selectedModel(id);
    if (!preferences.model && state.models.length) {
      if (id === 'codex') {
        model = state.models.find(candidate => candidate.isDefault) ?? state.models[0];
      } else {
        const version = (model: Model) => (model.label.match(/\d+(?:\.\d+)*/)?.[0] ?? model.id.match(/\d+(?:\.\d+)*/)?.[0] ?? '0');
        model = [...state.models].sort((a, b) => numeric.compare(version(b), version(a)) || numeric.compare(a.label, b.label))[0];
      }
      preferences.model = model.id;
      modified = true;
    }
    if (model && !model.efforts.includes(preferences.reasoningEffort ?? '')) {
      const fallback = defaultEffort(model);
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
  state.initialized = initialize(id, id === 'opencode' ? ++generation : ++codexGeneration);
  render();
}
function invalidateOpenCodeCatalog(): void {
  generation++;
  states.opencode.initialized = undefined;
  states.opencode.status = 'loading';
}
function syncProviders(): void {
  if (pending() && states.opencode.status === 'loading' && states.opencode.initialized) invalidateOpenCodeCatalog();
  if (!pending()) {
    const mode = provider.dataset.codexMode;
    if ((mode === 'cli' || mode === 'account') && mode !== states.codex.mode) {
      codexGeneration++;
      states.codex = codexStates[mode];
      states.codex.models = [];
      states.codex.initialized = undefined;
      states.codex.status = 'loading';
    }
    const available = new Set(providers.filter(installed));
    const previous = detected;
    detected = available;
    for (const id of available) {
      if (!previous.has(id) || !states[id].initialized) {
        startDiscovery(id);
      }
    }
  }
  render();
}
provider.addEventListener('providerstate', syncProviders);
provider.addEventListener('change', render);
window.addEventListener('picker-project', (event: CustomEvent<string | undefined>) => {
  if (event.detail === project) return;
  project = event.detail;
  invalidateOpenCodeCatalog();
  if (!pending() && installed('opencode')) startDiscovery('opencode');
  else render();
});
syncProviders();

pickerWindow.codexPickerReady = async (id = activeId()) => {
  if (!id || !installed(id)) throw new Error('Choose an available provider before sending.');
  const state = states[id];
  const initialized = state.initialized;
  await initialized;
  if (state !== states[id] || initialized !== state.initialized) {
    throw new Error('The model catalog changed. Check your model and try again.');
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
  if (state !== states[id] || initialized !== state.initialized) {
    throw new Error('The model catalog changed. Check your model and try again.');
  }
  return {model: modelId, reasoningEffort, ...(id === 'codex' ? {fastMode: fastMode ?? false} : {})};
};
})();
