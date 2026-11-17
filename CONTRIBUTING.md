# Contributing

Refract is under development. Before starting a substantial change, open an issue to discuss its scope. Include reproduction steps, expected and actual behavior, and relevant versions when reporting a bug. Do not include private project contents, credentials, or chat history in public reports.

Report security vulnerabilities privately as described in [SECURITY.md](SECURITY.md).

## Development

Use Windows, Node.js 22.12 or newer, and the setup instructions in [README.md](README.md). Run `npm ci`, configure a separately installed TeX toolchain, and use `npm start` for development. Run `npm test` before submitting a change; describe any tests you could not run. Installer and real Codex tests are opt-in and have additional requirements documented in the README.

Application and test code must be TypeScript. Keep type checking enabled. Follow the task ID allocation rules in [TODO.md](TODO.md) when adding or editing roadmap tasks. Keep changes focused and explain the behavior and verification in your pull request.

## Continuous integration

To limit CI costs, the `Windows CI` workflow runs automatically only when a GitHub release is published. Maintainers can also start it manually to verify a release tag or commit before publication. Ordinary pushes and pull requests do not run CI; contributors should run the relevant checks locally. Its `Windows build and tests` check installs the locked npm dependencies, downloads and verifies an external TinyTeX toolchain for the runner, builds TypeScript, and runs `npm test` on a GitHub-hosted Windows runner. The workflow has read-only repository permissions and does not retain checkout credentials or publish application artifacts.

Tests requiring a signed-in assistant, a packaged application, or the installer lifecycle remain separate opt-in checks. A green CI result does not establish that those release checks passed. See [README.md](README.md) for their requirements.

Maintainers: see [GitHub maintenance setup](docs/github-maintenance.md) for repository settings and the remaining activation steps when the repository becomes public.

Contributions are subject to the project's [MIT License](LICENSE). Third-party components have their own licenses; see [the distribution review](docs/third-party-review.md) and the licenses distributed with dependencies.
