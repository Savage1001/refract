import fs = require('node:fs');
import path = require('node:path');

class UpdateDiagnostics {
  constructor(private directory: string, private version: string) {}
  record(event: string, details: Record<string, unknown> = {}): void {
    try {
      fs.mkdirSync(this.directory, {recursive: true});
      fs.appendFileSync(path.join(this.directory, 'updates.jsonl'), JSON.stringify({
        timestamp: new Date().toISOString(), pid: process.pid, version: this.version, event, ...details
      }) + '\n');
    } catch (error) { console.error('Could not persist update diagnostics', error); }
  }
  async stage(name: string, operation: () => Promise<unknown>, stalled: () => void, delayMs = 30_000): Promise<void> {
    const started = Date.now();
    this.record('shutdown-stage-start', {stage: name});
    const timer = setTimeout(() => {
      this.record('shutdown-stage-stalled', {stage: name, elapsedMs: Date.now() - started});
      stalled();
    }, delayMs);
    try {
      await operation();
      this.record('shutdown-stage-end', {stage: name, elapsedMs: Date.now() - started});
    } catch (error) {
      this.record('shutdown-stage-error', {stage: name, elapsedMs: Date.now() - started, error: String(error)});
      throw error;
    } finally { clearTimeout(timer); }
  }
}

export = {UpdateDiagnostics};
