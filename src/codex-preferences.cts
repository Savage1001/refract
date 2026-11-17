import fsModule = require('node:fs');
const fs = fsModule.promises;
import path = require('node:path');
import crypto = require('node:crypto');

export type ReasoningEffort = 'minimal' | 'low' | 'medium' | 'high' | 'xhigh' | 'max' | 'ultra';
export interface CodexPreferencesAPI { load(): Promise<CodexPreferences>; save(value: CodexPreferences): Promise<void> }
export interface CodexPreferences { model: string | null; reasoningEffort: string | null; favorites: string[]; fastMode?: boolean }
const defaultCodexPreferences = (): CodexPreferences => ({ model: null, reasoningEffort: null, favorites: [] });
const efforts = new Set<ReasoningEffort>(['minimal', 'low', 'medium', 'high', 'xhigh', 'max', 'ultra']);
function modelId(value: unknown): value is string {
  return typeof value === 'string' && value.length > 0 && value.length <= 200 && value.trim() === value && !/[\u0000-\u001f\u007f]/.test(value);
}
function validateCodexPreferences(value: unknown, openCode = false): CodexPreferences {
  if (!value || typeof value !== 'object' || Array.isArray(value)) throw new Error('Invalid Codex preferences');
  const data = value as Record<string, unknown>;
  if (Object.keys(data).some(key => !['model', 'reasoningEffort', 'favorites', ...(openCode ? [] : ['fastMode'])].includes(key)) ||
      (data.fastMode !== undefined && typeof data.fastMode !== 'boolean') ||
      !(data.model === null || modelId(data.model)) ||
      !(data.reasoningEffort === null || (typeof data.reasoningEffort === 'string' && (openCode ? /^[^\s#\/\x00-\x1f\x7f]{1,200}$/.test(data.reasoningEffort) : efforts.has(data.reasoningEffort as ReasoningEffort)))) ||
      !Array.isArray(data.favorites) || data.favorites.length > 500 || !data.favorites.every(modelId) || new Set(data.favorites).size !== data.favorites.length) throw new Error('Invalid Codex preferences');
  return { model: data.model as string | null, reasoningEffort: data.reasoningEffort as ReasoningEffort | null, favorites: [...data.favorites], ...(data.fastMode !== undefined ? {fastMode: data.fastMode as boolean} : {}) };
}
class CodexPreferencesStore {
  private pending: Promise<void> = Promise.resolve();
  constructor(private readonly file: string, private readonly openCode = false) {}
  async load(): Promise<CodexPreferences> {
    await this.pending;
    try { return validateCodexPreferences(JSON.parse(await fs.readFile(this.file, 'utf8')), this.openCode); }
    catch (error) { if ((error as NodeJS.ErrnoException).code === 'ENOENT') return defaultCodexPreferences(); throw error; }
  }
  save(value: unknown): Promise<void> {
    const data = validateCodexPreferences(value, this.openCode);
    const operation = this.pending.catch(() => {}).then(async () => {
      await fs.mkdir(path.dirname(this.file), { recursive: true });
      const temp = `${this.file}.${crypto.randomUUID()}.tmp`;
      try { await fs.writeFile(temp, JSON.stringify(data), { flag: 'wx' }); await fs.rename(temp, this.file); }
      finally { await fs.unlink(temp).catch(() => {}); }
    });
    this.pending = operation;
    return operation;
  }
}


module.exports = { CodexPreferencesStore, validateCodexPreferences, defaultCodexPreferences, modelId };


