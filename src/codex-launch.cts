import path = require('node:path');

export interface CodexLaunch {
  command: string;
  args: string[];
  env?: NodeJS.ProcessEnv;
  accountKey?: string;
  revision?: number;
}

function accountCodexLaunch(connection: {
  executable: string; home: string; accountKey: string; accessToken: string; revision: number;
}, cwd: string): CodexLaunch {
  const { executable, home, accountKey, accessToken, revision } = connection;
  const config = [
    'allow_login_shell=false', 'thread_unload_delay_secs=0',
    'model_provider="openai_chatgpt_plan"',
    'model_providers.openai_chatgpt_plan.name="ChatGPT plan"',
    'model_providers.openai_chatgpt_plan.base_url="https://api.openai.com/v1"',
    'model_providers.openai_chatgpt_plan.env_key="REFRACT_CHATGPT_ACCESS_TOKEN"',
    'model_providers.openai_chatgpt_plan.wire_api="responses"',
    'model_providers.openai_chatgpt_plan.requires_openai_auth=false',
    'model_providers.openai_chatgpt_plan.supports_websockets=false',
    'shell_environment_policy.exclude=["REFRACT_CHATGPT_ACCESS_TOKEN"]',
    // Project-level app-server configuration cannot override this connection or start stdio MCP servers.
    `projects.${JSON.stringify(cwd)}.trust_level="untrusted"`,
    'features.tool_search=false',
  ];
  const env = { ...process.env, CODEX_HOME: home, REFRACT_CHATGPT_ACCESS_TOKEN: accessToken };
  // The native app-server distribution carries rg and sandbox helpers alongside bin/.
  const pathKey = Object.keys(env).find(key => key.toUpperCase() === 'PATH') ?? 'PATH';
  env[pathKey] = [path.resolve(path.dirname(executable), '../codex-path'), env[pathKey]].filter(Boolean).join(path.delimiter);
  for (const key of Object.keys(env)) {
    if (/^(OPENAI_(API_KEY|BASE_URL)|CODEX_(ACCESS_TOKEN|API_KEY))$/i.test(key)) delete env[key];
  }
  return {
    command: executable, args: ['--listen', 'stdio://', ...config.flatMap(value => ['-c', value])],
    env, accountKey, revision,
  };
}

export = { accountCodexLaunch };
