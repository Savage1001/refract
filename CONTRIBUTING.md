# Contributing

Refract is under development. Before starting a substantial change, open an issue to discuss its scope. Include reproduction steps, expected and actual behavior, and relevant versions when reporting a bug. Do not include private project contents, credentials, or chat history in public reports.

Report security vulnerabilities privately as described in [SECURITY.md](SECURITY.md).

## Development

Use stable Rust and the setup instructions in [native/README.md](native/README.md) for the GPUI desktop app. Node.js 22.12 or newer is required for packaging and the Electron reference tests. Run the checks relevant to your change and describe any tests you could not run. Installer and real Codex tests have additional requirements documented in [README.md](README.md).

Use Rust for the native app and its tests. Use TypeScript for packaging, other application code, and corresponding tests. Keep type checking enabled. Follow the task ID allocation rules in [TODO.md](TODO.md) when adding or editing roadmap tasks. Keep changes focused and explain the behavior and verification in your pull request.

## Continuous integration

Relevant pull requests run native tests and installer verification on GitHub-hosted runners. The Electron reference workflow builds TypeScript and runs its renderer tests for comparison. The `Windows CI` workflow also provides legacy release verification and can be started manually. The workflows use read-only repository permissions and do not publish releases.

Tests requiring a signed-in assistant, a packaged application, or the installer lifecycle remain separate opt-in checks. A green CI result does not establish that those release checks passed. See [README.md](README.md) for their requirements.

Maintainers: see [GitHub maintenance setup](docs/github-maintenance.md) for repository settings and the remaining activation steps when the repository becomes public.

Contributions are subject to the project's [MIT License](LICENSE). Third-party components have their own licenses; see [the distribution review](docs/third-party-review.md) and the licenses distributed with dependencies.
