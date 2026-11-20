import fs = require('node:fs/promises');
import path = require('node:path');
const { createHash, randomUUID } = require('node:crypto') as typeof import('node:crypto');
import type { RuntimeProgress } from './managed-codex.cjs';
const { authorizeChatGPT, renewChatGPT, revokeChatGPT, SignInRequired } = require('./chatgpt-sign-in.cjs') as typeof import('./chatgpt-sign-in.cjs');
import type { Registration, Session } from './chatgpt-sign-in.cjs';

export type CodexMode = 'cli' | 'account';
export type CodexAccountStatus = {
  mode: CodexMode;
  phase: 'idle' | 'downloading' | 'installing' | 'signing-in' | 'signing-out';
  percent?: number;
  installed: boolean;
  signedIn: boolean;
  email?: string;
  hasRegistration?: boolean;
  remembered?: string;
  message?: string;
};

interface AccountData { hostId: string; registration?: Registration; session?: Session }
interface Runtime {
  executable(): Promise<string | null>;
  install(report: (progress: RuntimeProgress) => void, signal: AbortSignal): Promise<string>;
}
interface CredentialProtection {
  isEncryptionAvailable(): boolean;
  encryptString(value: string): Buffer;
  decryptString(value: Buffer): string;
}

async function replaceFile(file: string, content: string | Buffer): Promise<void> {
  const temp = file + '.' + randomUUID();
  try {
    await fs.writeFile(temp, content, { flag: 'wx', mode: 0o600 });
    await fs.rename(temp, file);
  } finally { await fs.rm(temp, { force: true }).catch(() => {}); }
}

class CodexAccount {
  mode: CodexMode = 'cli';
  private phase: CodexAccountStatus['phase'] = 'idle';
  private percent?: number;
  private message?: string;
  private data?: AccountData;
  private controller?: AbortController;
  private operation?: Promise<void>;
  private sessionRefresh?: Promise<Session>;
  private readonly credentialFile: string;
  private readonly modeFile: string;

  constructor(private readonly root: string, private readonly runtime: Runtime,
    private readonly encryption: CredentialProtection, private readonly openBrowser: (url: string) => Promise<void>) {
    this.credentialFile = path.join(root, 'connection.bin');
    this.modeFile = path.join(root, 'mode');
  }

  async load(): Promise<void> {
    if (await fs.readFile(this.modeFile, 'utf8').catch(() => '') === 'account') this.mode = 'account';
    if (!this.encryption.isEncryptionAvailable()) return;
    try {
      this.data = JSON.parse(this.encryption.decryptString(await fs.readFile(this.credentialFile)));
    } catch {}
  }

  async status(): Promise<CodexAccountStatus> {
    const installed = Boolean(await this.runtime.executable());
    const session = this.data?.session;
    return {
      mode: this.mode, phase: this.phase, percent: this.percent, installed,
      signedIn: Boolean(session), email: session?.email, hasRegistration: Boolean(this.data?.registration), remembered: this.data?.registration?.email,
      message: this.message,
    };
  }

  private run(operation: (signal: AbortSignal) => Promise<void>): Promise<void> {
    if (this.operation) throw new Error('Codex setup is already in progress.');
    this.message = undefined;
    const controller = this.controller = new AbortController();
    this.operation = operation(controller.signal).finally(() => {
      this.phase = 'idle';
      this.percent = undefined;
      this.controller = undefined;
      this.operation = undefined;
    });
    return this.operation;
  }

  async cancel(): Promise<void> {
    this.controller?.abort();
    await this.operation?.catch(() => {});
  }

  async select(mode: unknown): Promise<void> {
    if (mode !== 'cli' && mode !== 'account') throw new Error('Invalid Codex connection.');
    if (mode === 'cli') await this.cancel();
    return this.run(async signal => {
      await fs.mkdir(this.root, { recursive: true });
      await replaceFile(this.modeFile, mode);
      this.mode = mode;
      if (mode === 'account') {
        try {
          await this.runtime.install(progress => {
            this.phase = progress.phase;
            this.percent = progress.phase === 'downloading' ? progress.percent : undefined;
          }, signal);
        } catch (error) {
          if (!signal.aborted) {
            this.message = error instanceof Error && /^((ChatGPT account mode)|(Codex runtime checksum))/.test(error.message)
              ? error.message : 'Could not install the Codex runtime. Check your connection and try again.';
          }
        }
      }
    });
  }

  async signIn(newRegistration: boolean): Promise<void> {
    if (this.mode !== 'account' || !await this.runtime.executable()) throw new Error('Install the Codex runtime first.');
    return this.run(async signal => {
      this.phase = 'signing-in';
      try {
        if (this.data?.session) throw new Error('Sign out before connecting another account.');
        if (!this.encryption.isEncryptionAvailable()) throw new Error('Secure storage is unavailable for ChatGPT sign-in.');
        const data = this.data ?? { hostId: 'urn:uuid:' + randomUUID() };
        if (!this.data) await this.save(data);
        const session = await authorizeChatGPT(data.hostId, newRegistration ? undefined : data.registration, this.openBrowser, signal);
        signal.throwIfAborted();
        const { clientId, subject, email } = session;
        await this.save({ hostId: data.hostId, registration: { clientId, subject, email }, session });
      } catch (error) {
        if (!signal.aborted) this.message = error instanceof Error && /^(ChatGPT |OpenAI |Choose |Secure |Sign out |Could not verify)/.test(error.message)
          ? error.message : 'Could not sign in to ChatGPT. Try again.';
      }
    });
  }

  async signOut(): Promise<void> {
    return this.run(async () => {
      this.phase = 'signing-out';
      await this.sessionRefresh?.catch(() => {});
      const data = this.data;
      if (!data?.session) return;
      const revoked = await revokeChatGPT(data.session);
      await this.save({ hostId: data.hostId, registration: data.registration });
      if (!revoked) this.message = 'Signed out on this device. You can also disconnect Refract in ChatGPT Settings.';
    });
  }

  async connection() {
    const executable = await this.runtime.executable();
    if (!executable) throw new Error('Finish installing Codex in Settings.');
    // Logout and account changes must not race a new provider process.
    if (this.operation) throw new Error('Finish setting up ChatGPT account mode in Settings.');
    const session = await this.currentSession();
    const accountKey = createHash('sha256').update(session.clientId).digest('hex');
    const home = path.join(this.root, 'workspaces', accountKey);
    await fs.mkdir(home, { recursive: true });
    return { executable, home, accountKey, accessToken: session.accessToken, revision: session.expiresAt };
  }

  private async currentSession(): Promise<Session> {
    const data = this.data;
    if (!data?.session) throw new SignInRequired('Sign in with ChatGPT in Refract Settings.');
    if (data.session.expiresAt - Date.now() > 120_000) return data.session;
    this.sessionRefresh ??= (async () => {
      try {
        const session = await renewChatGPT(data.session);
        await this.save({ ...data, session });
        return session;
      } catch (error) {
        if (error instanceof SignInRequired) {
          await this.save({ hostId: data.hostId, registration: data.registration });
          throw error;
        }
        throw new Error('Could not renew your ChatGPT session. Check your connection and try again.');
      }
    })().finally(() => { this.sessionRefresh = undefined; });
    return this.sessionRefresh;
  }

  private async save(data: AccountData): Promise<void> {
    await fs.mkdir(this.root, { recursive: true });
    await replaceFile(this.credentialFile, this.encryption.encryptString(JSON.stringify(data)));
    this.data = data;
  }
}

export = { CodexAccount };
