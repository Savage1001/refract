import test = require('node:test');
import assert = require('node:assert/strict');
import fs = require('node:fs/promises');
import os = require('node:os');
import path = require('node:path');
import crypto = require('node:crypto');
import account = require('../src/codex-account.cjs');
import fixture = require('./fixtures/headless-ui.cjs');
import type { CodexAccountStatus } from '../src/codex-account.cjs';

test.test('account setup downloads once and persists a protected browser sign-in', async t => {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), 'refract-account-test-'));
  t.after(() => fs.rm(root, { recursive: true, force: true }));
  const keys = crypto.generateKeyPairSync('rsa', { modulusLength: 2048 });
  const clientId = 'oaiapp_fixture';
  const issuer = 'https://auth.openai.com';
  const loopback = globalThis.fetch;
  let authorizeUrl: URL;
  let exchanges = 0;
  let refreshes = 0;
  const fetchMock = t.mock.method(globalThis, 'fetch', async (input: string, init?: RequestInit) => {
    const body = init?.body as URLSearchParams;
    if (input === issuer + '/.well-known/jwks.json') {
      return Response.json({ keys: [{ ...keys.publicKey.export({ format: 'jwk' }), kid: 'fixture' }] });
    }
    if (input.endsWith('/oauth/revoke')) {
      assert.equal(body.get('token'), 'rotated-refresh');
      return new Response();
    }
    assert.equal(input, issuer + '/api/accounts/oauth/token');
    assert.equal(body.get('resource'), 'https://api.openai.com/v1');
    assert.equal(body.get('client_id'), clientId);
    if (body.get('grant_type') === 'refresh_token') {
      refreshes++;
      assert.equal(body.get('refresh_token'), 'fixture-refresh');
      return Response.json({ access_token: 'refreshed-token', refresh_token: 'rotated-refresh', token_type: 'Bearer', expires_in: 3600 });
    }
    exchanges++;
    assert.equal(body.get('code'), 'fixture-code');
    const challenge = crypto.createHash('sha256').update(body.get('code_verifier')).digest('base64url');
    assert.equal(challenge, authorizeUrl.searchParams.get('code_challenge'));
    assert.equal(body.get('redirect_uri'), authorizeUrl.searchParams.get('redirect_uri'));
    const encode = (value: object) => Buffer.from(JSON.stringify(value)).toString('base64url');
    const header = encode({ alg: 'RS256', kid: 'fixture' });
    const payload = encode({ iss: issuer, aud: clientId, sub: 'fixture-subject', exp: Date.now() / 1000 + 3600,
      nonce: authorizeUrl.searchParams.get('nonce'), email: 'account@example.com' });
    const signature = crypto.sign('RSA-SHA256', Buffer.from(header + '.' + payload), keys.privateKey).toString('base64url');
    return Response.json({ access_token: 'fixture-token', refresh_token: 'fixture-refresh', token_type: 'Bearer',
      expires_in: 60, scope: 'openid resource.invoke chatgpt.tokens.use.direct', id_token: [header, payload, signature].join('.') });
  });
  const openBrowser = async (url: string) => {
    authorizeUrl = new URL(url);
    assert.equal(authorizeUrl.origin, issuer);
    assert.equal(authorizeUrl.searchParams.get('client_id'), exchanges ? clientId : 'dynamic_agent_client');
    const callback = new URL(authorizeUrl.searchParams.get('redirect_uri'));
    callback.search = new URLSearchParams({ code: 'fixture-code', client_id: clientId, state: 'wrong' }).toString();
    assert.equal((await loopback(callback)).status, 400);
    callback.searchParams.set('state', authorizeUrl.searchParams.get('state'));
    assert.equal((await loopback(callback)).status, 200);
  };
  let installed = false;
  let downloads = 0;
  const executable = path.join(root, 'codex-app-server.exe');
  const runtime = {
    executable: async () => installed ? executable : null,
    install: async (report: (progress: {phase: 'downloading'; percent: number}) => void) => {
      if (!installed) {
        downloads++;
        report({ phase: 'downloading', percent: 37 });
        await new Promise(resolve => setImmediate(resolve));
        installed = true;
      }
      return executable;
    },
  };
  const protection = {
    isEncryptionAvailable: () => true,
    encryptString: (text: string) => Buffer.from(Buffer.from(text).toString('base64')),
    decryptString: (buffer: Buffer) => Buffer.from(buffer.toString(), 'base64').toString(),
  };
  const service = new account.CodexAccount(root, runtime, protection, openBrowser);
  await service.load();
  assert.equal((await service.status()).mode, 'cli');
  assert.equal(downloads, 0);
  await service.select('account');
  await service.signIn(false);
  assert.equal((await service.status()).email, 'account@example.com');
  assert.equal(JSON.stringify(await service.status()).includes('fixture-token'), false);
  assert.equal((await fs.readFile(path.join(root, 'connection.bin'), 'utf8')).includes('fixture-refresh'), false);

  const restarted = new account.CodexAccount(root, runtime, protection, openBrowser);
  await restarted.load();
  assert.equal((await restarted.status()).mode, 'account');
  const connections = await Promise.all([restarted.connection(), restarted.connection()]);
  assert.equal(connections[0].accessToken, 'refreshed-token');
  assert.equal(connections[0].accountKey, connections[1].accountKey);
  assert.equal(refreshes, 1);
  await restarted.signOut();
  assert.equal((await restarted.status()).signedIn, false);
  await restarted.select('cli');
  await restarted.select('account');
  assert.equal(downloads, 1);
  await restarted.signIn(false);
  assert.equal(exchanges, 2);
  assert.equal(fetchMock.mock.callCount(), 6);
});

test.test('Codex setup uses the update progress bar style and keeps the downloads separate', { timeout: 30_000 }, async () => {
  const { browser, page, errors } = await fixture.openHeadlessUi('light', { pdf: '', source: '% Account settings' });
  try {
    await page.evaluate(() => {
      const store = (window as unknown as { store: Record<string, unknown> }).store;
      let status: CodexAccountStatus = { mode: 'cli', phase: 'idle', installed: false, signedIn: false };
      let install: () => void;
      store.codexAccountStatus = async () => status;
      store.codexAccountSelect = async (mode: 'cli' | 'account') => {
        status = { ...status, mode };
        if (mode === 'cli') return;
        status = { ...status, phase: 'downloading', percent: 42 };
        await new Promise<void>(resolve => { install = resolve; });
        status = { ...status, phase: 'idle', percent: undefined, installed: true };
      };
      store.codexAccountSignIn = async () => { status = { ...status, signedIn: true, email: 'account@example.com' }; };
      store.updateStatus = async () => ({ track: 'stable', phase: 'downloading', message: 'Downloading update', downloadPercent: 17 });
      window.addEventListener('finish-test-install', () => install());
    });
    await page.getByRole('button', { name: 'Settings', exact: true }).click();
    const connection = page.getByLabel('Codex connection');
    assert.equal(await connection.inputValue(), 'cli');
    await connection.selectOption('account');
    const codex = page.getByRole('progressbar', { name: 'Codex runtime download progress' });
    const update = page.getByRole('progressbar', { name: 'Update download progress' });
    await codex.waitFor();
    assert.equal(await codex.getAttribute('value'), '42');
    assert.equal(await update.getAttribute('value'), '17');
    const style = (el: Element) => [getComputedStyle(el).height, getComputedStyle(el).appearance, el.parentElement.className];
    assert.deepEqual(await codex.evaluate(style), await update.evaluate(style));
    await page.evaluate(() => window.dispatchEvent(new Event('finish-test-install')));
    const signIn = page.getByRole('button', { name: 'Continue with ChatGPT' });
    await signIn.click();
    await page.getByText('Signed in as account@example.com').waitFor();
    assert.equal(await codex.isVisible(), false);
    await connection.selectOption('cli');
    await signIn.waitFor({ state: 'hidden' });
    assert.deepEqual(errors, []);
  } finally {
    await browser.close();
  }
});
