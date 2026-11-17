const { modelId } = require('./codex-preferences.cjs');
import childProcess = require('node:child_process');
const { spawn } = childProcess;
import path = require('node:path');
const { join } = path;
import readline = require('node:readline');
const { createInterface } = readline;

export interface CodexReasoningEffort { effort: string; description: string }
export interface CodexModel { id: string; model: string; displayName: string; description: string; supportedReasoningEfforts: CodexReasoningEffort[]; defaultReasoningEffort: string | null; isDefault: boolean }
export type ModelDiscoveryResult = { ok: true; models: CodexModel[] } | { ok: false; error: string };
export interface CodexDiscoveryApi { discoverCodexModels(): Promise<ModelDiscoveryResult> }

/** Query the installed CLI. Its catalog may be cached and is not an entitlement check. */
function discoverCodexModels(options: { script?: string; timeoutMs?: number } = {}): Promise<ModelDiscoveryResult> {
  return new Promise(resolve => {
    const script = options.script ?? join(process.env.APPDATA || '', 'npm', 'node_modules', '@openai', 'codex', 'bin', 'codex.js');
    const child = spawn('node', [script, 'app-server'], { windowsHide: true, stdio: ['pipe', 'pipe', 'pipe'] });
    let done = false, initialized = false, cursor: string | null = null;
    const models: CodexModel[] = [];
    const seen = new Set<string>();
    const finish = (result: ModelDiscoveryResult) => { if (done) return; done = true; clearTimeout(timer); child.kill(); resolve(result); };
    const timer = setTimeout(() => finish({ ok: false, error: 'Codex model discovery timed out' }), options.timeoutMs ?? 15000);
    const send = (value: unknown) => child.stdin.write(JSON.stringify(value) + '\n');
    child.on('error', () => finish({ ok: false, error: 'Could not start Codex model discovery' }));
    child.stdin.on('error', () => finish({ ok: false, error: 'Codex model discovery connection failed' }));
    child.on('close', () => finish({ ok: false, error: 'Codex model discovery exited unexpectedly' }));
    createInterface({ input: child.stdout }).on('line', line => {
      let msg: any;
      try { msg = JSON.parse(line); } catch { return; }
      if (msg.id !== 1 && msg.id !== 2) return;
      if (msg.error) return finish({ ok: false, error: 'Codex model discovery request failed' });
      if (msg.id === 1 && !initialized) {
        initialized = true;
        send({ method: 'initialized' });
        send({ id: 2, method: 'model/list', params: { cursor: null, limit: 100 } });
      } else if (msg.id === 2 && initialized) {
        const page = msg.result;
        if (!page || !Array.isArray(page.data) || (page.nextCursor != null && typeof page.nextCursor !== 'string')) return finish({ ok: false, error: 'Invalid Codex model catalog' });
        for (const item of page.data) {
          if (!item || typeof item.id !== 'string' || typeof item.model !== 'string' || typeof item.displayName !== 'string' || !Array.isArray(item.supportedReasoningEfforts) || !item.supportedReasoningEfforts.every((e: any) => e && typeof (e.reasoningEffort ?? e.effort) === 'string' && typeof e.description === 'string')) return finish({ ok: false, error: 'Invalid Codex model catalog' });
          if (!modelId(item.id) || !modelId(item.model)) continue;
          models.push({ id: item.id, model: item.model, displayName: item.displayName, description: typeof item.description === 'string' ? item.description : '', supportedReasoningEfforts: item.supportedReasoningEfforts.map((e: {reasoningEffort?: string; effort?: string; description: string}) => ({effort: e.reasoningEffort ?? e.effort!, description: e.description})), defaultReasoningEffort: typeof item.defaultReasoningEffort === 'string' ? item.defaultReasoningEffort : null, isDefault: item.isDefault === true });
        }
        cursor = page.nextCursor ?? null;
        if (cursor) { if (seen.has(cursor)) return finish({ ok: false, error: 'Invalid Codex model catalog pagination' }); seen.add(cursor); send({ id: 2, method: 'model/list', params: { cursor, limit: 100 } }); }
        else finish({ ok: true, models });
      }
    });
    send({ id: 1, method: 'initialize', params: { clientInfo: { name: 'refract', title: 'Refract', version: '0.1.1' } } });
  });
}


export = { discoverCodexModels };


