# GitHub maintenance setup

The repository must remain private until the separate source and binary distribution reviews are resolved. Adding CI does not authorize publication.

## CI

To limit CI costs, `.github/workflows/windows-ci.yml` runs the `Windows build and tests` job automatically only on the `release: published` event, including published prereleases. It checks out the release tag, rather than the current default branch. There are no push, pull request, or scheduled triggers.

After the workflow is on the default branch, maintainers can also start it manually for release verification. Supply the optional `ref` input with the exact release tag or commit; otherwise it checks the selected workflow ref. For example: `gh workflow run windows-ci.yml --ref main -f ref=refs/tags/v0.2.3`. A published-release run is verification after publication and does not gate publishing. To verify before publication, start a manual run against the candidate tag or commit and inspect its result before publishing the release. Do not routinely dispatch it for ordinary changes.

GitHub-hosted runners perform the desktop verification; no maintainer workstation is used. The workflow uses SHA-pinned checkout and Node setup actions, a read-only token, and no persisted checkout credentials. It installs the checksum-pinned upstream TinyTeX archive only in the runner temporary directory. TinyTeX omits SyncTeX, so the workflow separately downloads the upstream Windows SyncTeX archive and verifies its pinned SHA-256 before extracting only the test executable. A changed upstream archive fails verification instead of running unchecked bytes. It exports REFRACT_TEX_BIN for tests. The application setup script validates that external directory without downloading or modifying TeX. These test archives and the toolchain are never packaged or uploaded. The workflow does not publish binaries, upload project data, use repository secrets, or run signed-in assistant probes.

Do not require **Windows build and tests** as a pull request status check: it intentionally does not run on pull requests, so that requirement would block normal merges. Local verification remains the contribution requirement. Fork pull requests should require approval from a maintainer before any future PR workflows run; approve only after reviewing the workflow and commands they execute.

## Repository settings

These settings require repository administrator permissions. Some controls are unavailable for a private repository on GitHub Free. Do not change repository visibility solely to enable them.

1. Enable Dependabot alerts under **Settings > Security and quality > Advanced Security**. Alerts identify vulnerable dependencies; they do not establish that a dependency update is compatible.
2. Keep **Settings > Actions > General > Workflow permissions** at **Read repository contents and packages permissions**, with **Allow GitHub Actions to create and approve pull requests** disabled. The workflow also restricts its token to `contents: read`.
3. When the repository is made public, enable **Private vulnerability reporting** under **Advanced Security** and verify the **Report a vulnerability** link on **Security > Advisories** works. `SECURITY.md` also provides the maintainer's approved public email fallback.
4. Under **Settings > Actions > General**, require approval for **all outside collaborators** before fork pull request workflows run.
5. Protect `main` under **Settings > Branches**. Require a pull request before merging, require conversation resolution, and apply the rules to administrators. Keep force pushes and branch deletions disabled. Leave required CI status checks unset because CI runs only for releases. Review approval counts are a maintainer policy choice; do not require an unavailable second maintainer merely to protect the branch.

Private reporting can be enabled without a browser using `gh api --method PUT repos/Savage1001/refract/private-vulnerability-reporting`. Dependabot alerts can be enabled with `gh api --method PUT repos/Savage1001/refract/vulnerability-alerts`. Both commands should be followed by a GET of the same endpoint to verify the resulting state. A private-repository or plan restriction is a blocker to report, not a reason to silently skip verification or make the repository public.
