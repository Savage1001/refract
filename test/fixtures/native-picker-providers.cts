import http = require('node:http');
import readline = require('node:readline');

// Match the fake catalogs in headless-ui.cts using the protocols the native desktop reads.
const codexModel = {
  id: 'model-a',
  model: 'model-a',
  displayName: 'Research model',
  description: 'Sample model for UI verification',
  supportedReasoningEfforts: ['low', 'medium', 'high'].map(effort => ({ effort, description: '' })),
  defaultReasoningEffort: 'medium',
  isDefault: true,
};
const openCodeModel = {
  providerID: 'opencode',
  id: 'research',
  name: 'Research model',
  variants: [{ id: 'low' }, { id: 'high' }],
};

function serveCodex(): void {
  readline.createInterface({ input: process.stdin }).on('line', line => {
    const { id, method } = JSON.parse(line) as { id?: number; method?: string };
    if (typeof id !== 'number') return;
    if (method === 'initialize') {
      console.log(JSON.stringify({ id, result: {} }));
    } else if (method === 'model/list') {
      console.log(JSON.stringify({ id, result: { data: [codexModel], nextCursor: null } }));
    }
  });
}

function serveOpenCode(): void {
  const { OPENCODE_SERVER_USERNAME: username, OPENCODE_SERVER_PASSWORD: password } = process.env;
  if (username !== 'opencode' || !password) throw new Error('Missing synthetic OpenCode server configuration');
  const authorization = 'Basic ' + Buffer.from(username + ':' + password).toString('base64');
  const server = http.createServer((request, response) => {
    if (request.method !== 'GET' || request.url !== '/api/model') {
      response.writeHead(404).end();
    } else if (request.headers.authorization !== authorization) {
      response.writeHead(401).end();
    } else {
      response.writeHead(200, { 'Content-Type': 'application/json' });
      response.end(JSON.stringify({ data: [openCodeModel] }));
    }
  });
  server.listen(0, '127.0.0.1', () => {
    const address = server.address();
    if (address === null || typeof address === 'string') throw new Error('Missing synthetic OpenCode listen address');
    console.log(`server listening on http://127.0.0.1:${address.port}`);
  });
}

if (process.env.GITHUB_ACTIONS !== 'true' || process.env.RUNNER_ENVIRONMENT !== 'github-hosted') {
  throw new Error('The native model catalog fixture runs only on a GitHub-hosted runner');
}
const [, , provider, command] = process.argv;
if (provider === 'codex' && command === 'app-server') {
  serveCodex();
} else if (provider === 'opencode' && command === 'serve') {
  serveOpenCode();
} else {
  throw new Error('Unsupported synthetic provider command');
}
