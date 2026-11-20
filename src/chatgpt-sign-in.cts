const { createHash, createPublicKey, randomBytes, verify } = require('node:crypto') as typeof import('node:crypto');
const { createServer } = require('node:http') as typeof import('node:http');
const { once } = require('node:events') as typeof import('node:events');
import type { AddressInfo } from 'node:net';

const issuer = 'https://auth.openai.com';
const resource = 'https://api.openai.com/v1';
const tokenEndpoint = issuer + '/api/accounts/oauth/token';
const planScope = 'chatgpt.tokens.use.direct';

export interface Registration { clientId: string; subject: string; email?: string }
export interface Session extends Registration {
  accessToken: string;
  refreshToken: string;
  expiresAt: number;
}
interface TokenResponse {
  access_token?: string;
  refresh_token?: string;
  expires_in?: number;
  id_token?: string;
  token_type?: string;
  scope?: string;
  error?: string;
}
class SignInRequired extends Error {}

async function exchange(fields: Record<string, string>, signal?: AbortSignal): Promise<TokenResponse> {
  const timeout = AbortSignal.timeout(20_000);
  const response = await fetch(tokenEndpoint, {
    method: 'POST', body: new URLSearchParams({ ...fields, resource }), redirect: 'error',
    signal: signal ? AbortSignal.any([signal, timeout]) : timeout,
  });
  const data = await response.json() as TokenResponse;
  if (!response.ok) {
    if (fields.grant_type === 'refresh_token' && ['invalid_grant', 'invalid_client'].includes(data.error)) {
      throw new SignInRequired('Your ChatGPT session ended. Sign in again in Settings.');
    }
    throw new Error('OpenAI could not complete ChatGPT sign-in.');
  }
  return data;
}

function credentials(data: TokenResponse, previousRefresh?: string): Pick<Session, 'accessToken' | 'refreshToken' | 'expiresAt'> {
  const refreshToken = data.refresh_token ?? previousRefresh;
  if (typeof data.access_token !== 'string' || !data.access_token ||
      typeof refreshToken !== 'string' || !refreshToken ||
      data.token_type?.toLowerCase() !== 'bearer' || !Number.isFinite(data.expires_in) || data.expires_in <= 0) {
    throw new Error('OpenAI returned an incomplete ChatGPT session.');
  }
  if ((data.scope !== undefined || !previousRefresh) && !data.scope?.split(/\s+/).includes(planScope)) {
    throw new SignInRequired('ChatGPT plan usage was not granted. Sign in again and allow Refract to use your plan.');
  }
  return { accessToken: data.access_token, refreshToken, expiresAt: Date.now() + data.expires_in * 1000 };
}

async function verifyIdentity(idToken: string, clientId: string, nonce: string, signal: AbortSignal) {
  const parts = idToken?.split('.') ?? [];
  if (parts.length !== 3) throw new Error('Invalid ChatGPT identity.');
  const [header, payload, signature] = parts;
  const { alg, kid } = JSON.parse(Buffer.from(header, 'base64url').toString());
  if (alg !== 'RS256' || typeof kid !== 'string') throw new Error('Invalid ChatGPT identity.');
  const response = await fetch(issuer + '/.well-known/jwks.json', {
    redirect: 'error', signal: AbortSignal.any([signal, AbortSignal.timeout(20_000)]),
  });
  if (!response.ok) throw new Error('Could not verify ChatGPT identity.');
  const { keys } = await response.json() as { keys: (JsonWebKey & { kid: string })[] };
  const jwk = keys.find(key => key.kty === 'RSA' && key.kid === kid);
  if (!jwk || !verify('RSA-SHA256', Buffer.from(header + '.' + payload), createPublicKey({ key: jwk, format: 'jwk' }), Buffer.from(signature, 'base64url'))) {
    throw new Error('Invalid ChatGPT identity signature.');
  }
  const claims = JSON.parse(Buffer.from(payload, 'base64url').toString());
  const audience = Array.isArray(claims.aud) ? claims.aud : [claims.aud];
  if (claims.iss !== issuer || !audience.includes(clientId) ||
      ((audience.length > 1 || claims.azp !== undefined) && claims.azp !== clientId) ||
      claims.nonce !== nonce || !Number.isFinite(claims.exp) || claims.exp <= Date.now() / 1000 ||
      typeof claims.sub !== 'string' || !claims.sub) throw new Error('Invalid ChatGPT identity claims.');
  return { subject: claims.sub as string, email: typeof claims.email === 'string' ? claims.email as string : undefined };
}

async function authorizeChatGPT(hostId: string, registration: Registration | undefined,
  openBrowser: (url: string) => Promise<void>, cancel: AbortSignal): Promise<Session> {
  const state = randomBytes(32).toString('base64url');
  const nonce = randomBytes(32).toString('base64url');
  const verifier = randomBytes(32).toString('base64url');
  const signal = AbortSignal.any([cancel, AbortSignal.timeout(5 * 60_000)]);
  const callbackPath = '/auth/callback';
  let origin: string;
  const server = createServer();
  server.requestTimeout = 10_000;
  server.headersTimeout = 10_000;
  try {
    signal.throwIfAborted();
    server.listen(0, '127.0.0.1');
    await once(server, 'listening', { signal });
    origin = 'http://127.0.0.1:' + (server.address() as AddressInfo).port;
    const redirect = origin + callbackPath;
    const callback = new Promise<URLSearchParams>((resolve, reject) => {
      const abort = () => reject(signal.reason);
      signal.addEventListener('abort', abort, { once: true });
      server.once('close', () => signal.removeEventListener('abort', abort));
      server.on('request', (request, response) => {
        const url = URL.parse(request.url || '/', origin);
        response.setHeader('Cache-Control', 'no-store');
        response.setHeader('Content-Type', 'text/plain; charset=utf-8');
        response.setHeader('Referrer-Policy', 'no-referrer');
        if (request.method !== 'GET' || !url || url.origin !== origin || request.headers.host !== new URL(origin).host ||
            url.pathname !== callbackPath || url.searchParams.get('state') !== state) {
          response.writeHead(400).end('Invalid Refract sign-in callback.');
          return;
        }
        response.end('Return to Refract to finish connecting your ChatGPT account.');
        server.close();
        resolve(url.searchParams);
      });
    });
    void callback.catch(() => {});
    const url = new URL(issuer + '/api/accounts/authorize');
    url.search = new URLSearchParams({
      client_id: registration?.clientId ?? 'dynamic_agent_client',
      ...(!registration ? { agent_name_hint: 'Refract' } : registration.email ? { login_hint: registration.email } : {}),
      ext_agent_host_id: hostId, response_type: 'code', redirect_uri: redirect, resource,
      scope: 'openid profile email offline_access resource.invoke ' + planScope,
      state, nonce, code_challenge_method: 'S256',
      code_challenge: createHash('sha256').update(verifier).digest('base64url'),
    }).toString();
    await openBrowser(url.href);
    const result = await callback;
    if (result.has('error')) throw new Error(result.get('error') === 'access_denied' ? 'ChatGPT sign-in was cancelled.' : 'ChatGPT sign-in failed.');
    const returnedClient = result.get('client_id');
    const clientId = registration?.clientId ?? returnedClient;
    const code = result.get('code');
    if (!code || !clientId || clientId === 'dynamic_agent_client' || (returnedClient && returnedClient !== clientId)) {
      throw new Error('OpenAI did not finish registering Refract.');
    }
    const tokens = await exchange({ grant_type: 'authorization_code', client_id: clientId, code, code_verifier: verifier, redirect_uri: redirect }, signal);
    const identity = await verifyIdentity(tokens.id_token, clientId, nonce, signal);
    if (registration && registration.subject !== identity.subject) throw new Error('Choose the same ChatGPT account to reconnect.');
    return { clientId, ...identity, ...credentials(tokens) };
  } finally {
    server.close();
    server.closeAllConnections();
  }
}

async function renewChatGPT(session: Session, signal?: AbortSignal): Promise<Session> {
  const tokens = await exchange({ grant_type: 'refresh_token', client_id: session.clientId, refresh_token: session.refreshToken }, signal);
  return { ...session, ...credentials(tokens, session.refreshToken) };
}

async function revokeChatGPT(session: Session): Promise<boolean> {
  const body = new URLSearchParams({ token: session.refreshToken, token_type_hint: 'refresh_token', client_id: session.clientId });
  for (const delay of [0, 300, 900]) {
    if (delay) await new Promise(resolve => setTimeout(resolve, delay));
    try {
      const response = await fetch(issuer + '/api/accounts/oauth/revoke', {
        method: 'POST', body, redirect: 'error', signal: AbortSignal.timeout(3000),
      });
      if (response.status < 500) return response.ok;
    } catch {}
  }
  return false;
}

export = { authorizeChatGPT, renewChatGPT, revokeChatGPT, SignInRequired };
