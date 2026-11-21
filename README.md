<p align="center">
  <img src="resources/logo.png" width="128" height="128" alt="Refract app logo">
</p>

# Refract

Refract is a Windows Electron workspace for local LaTeX writing, inspired by OpenAI Prism. Projects, editing, compilation, and displayed chat history are local. For the assistant, connect a ChatGPT account or use a separately installed Codex or OpenCode CLI.

## Local execution

Projects can compile and use assistants immediately, including imported and duplicated projects. Review unfamiliar content before opening it: TeX and assistants run with your Windows account's access, including files outside the project. `-no-shell-escape` is not a filesystem sandbox. See [Security](SECURITY.md#local-execution) for the execution boundary and limitations.

## Current status

The application is under development and does not yet have full visual or feature parity with Prism. It supports local project creation, file editing, project ZIP import/export, PDF compilation and preview, and saved Codex chats. Project PDFs and SVGs can be previewed, while unsupported binary files are kept out of the text editor. Individual file previews are limited to 32 MiB. ZIP import preserves empty folders and lets you choose a root when an archive contains multiple LaTeX documents. See [TODO.md](TODO.md) for remaining Prism parity and verification work.

**ChatGPT account** is the default Codex connection. On the home screen, open **Settings** and choose **Download Codex** to download and verify the runtime. Once the download finishes, choose **Continue with ChatGPT** and approve Refract’s access to your ChatGPT plan in your browser. Refract reuses the installed runtime. ChatGPT manages [plan usage and limits](https://chatgpt.com/settings/usage).

To use a separately installed and authenticated CLI, set **Codex connection** to **Codex CLI** in Settings. OpenCode uses its own separately configured CLI, default model, and credentials. Switch providers or Codex connections to continue a conversation with its saved messages and attachment paths. Use **New chat** to start a separate conversation. Image support depends on the selected model.

Selected UTF-8 text attachments (`.txt`, `.md`, `.tex`, `.bib`, and `.csv`) include a quoted preview in the assistant request, up to 16 KiB per file and 32 KiB in total. This lets small text attachments work without starting a provider shell. Previews explicitly mark truncation; larger files, binary documents, and any remaining content require the provider's file tools. Full attachment copies and their paths remain available. Preview text is not added to Refract's displayed chat history; the provider receives it as part of the request.

Choose pdfLaTeX, XeLaTeX, or LuaLaTeX per project under Tools → Project Info. Existing projects default to pdfLaTeX. Enable Draft mode in the same panel to use graphicx image placeholders for faster compilation while retaining a PDF preview. Disable it for final output; documents that explicitly override graphicx draft settings may still render images.

In the editor, use **Commands** or Ctrl+Space to complete common LaTeX commands. Use the arrow keys and Enter or Tab to choose a suggestion.

After a successful compile, use **Show in PDF** to jump from the current source line to the preview. Double-click a position in the compiled PDF to open its source line. You can also focus a PDF page and press Enter: this uses its last synchronized position when available, or the page center otherwise. Navigation is unavailable while the preview is out of date.

Refract does not currently provide voice conversations. It uses Codex's app-server text-and-image interface; the interactive Codex CLI's [experimental voice mode](https://learn.chatgpt.com/docs/changelog?date=2026-09-10&index=2) is separate from that integration.

## Development on Windows

See [CONTRIBUTING.md](CONTRIBUTING.md) for contribution and verification guidance. Refract is available under the [MIT License](LICENSE); third-party components have their own licenses.

Install Node.js 22.12 or newer, then run:

```powershell
npm ci
$env:REFRACT_TEX_BIN = 'C:\path\to\TeXLive\bin\windows'
powershell -ExecutionPolicy Bypass -File scripts/setup-tex.ps1
npm start
```

Starting with 0.3.1-pre-release-2, the Windows installer automatically downloads checksum-verified TinyTeX and SyncTeX into `%LOCALAPPDATA%\Refract\dependencies` (internet access required). Refract discovers the complete toolchain automatically; no separate TeX installation or environment changes are required. Existing complete installations are reused, and uninstall preserves them. Setup failures return a nonzero installer exit code and write `setup-error.log` in that directory; rerun the installer to retry. For a custom, separately installed TeX Live or TinyTeX distribution, set `REFRACT_TEX_BIN` to its absolute Windows executable directory and restart Refract. This override takes priority over managed TeX. Additional document-specific packages can be installed separately. The Codex CLI and OpenCode CLI remain optional, separately installed and authenticated integrations.

Refract packages contain no TeX distribution or replacement SyncTeX binary. Compilation remains local and can work offline with already installed packages. Electron and application dependencies retain their own notices. See [the distribution boundary and historical cleanup requirements](docs/third-party-review.md).

To keep a runnable development build ready during longer work sessions, run `npm run build` after source changes. With dependencies, Electron, and the external TeX environment set up, `npx electron .` starts that build; `npm start` rebuilds and starts it. Generated files and dependencies are local to each checkout and are not committed.

TypeScript sources are compiled into `build/` before starting, testing, or packaging. TypeScript checking runs during the build; stronger types and strict checking remain work to do.

`npm test` ensures the Electron runtime is installed before starting the tests (the first run may download it), then exercises local storage, ZIP handling, compilation, chat persistence, and the Electron workflow. `npm run build:win` creates an unpacked Windows application under `dist/win-unpacked`, without a TeX distribution. Run `Refract.exe` from that directory. `npm run build:installer` builds a per-user Windows installer in `dist/`; it excludes TeX and preserves application data on uninstall. Close Refract before installing or upgrading. The installer is unsigned, so Windows may show a security warning. An isolated install, upgrade and uninstall lifecycle has passed on one Windows environment; other environments and recovery cases still need verification.

The installer checks for any process named `Refract.exe` before installation or uninstall, including an unrelated program with that name. It refuses to proceed while one is running and does not terminate it. If an upgrade fails, close Refract and retry; a silent failure returns a nonzero exit code. Investigate a repeated failure before removing or replacing installation files.

By default, projects and application data are under `%APPDATA%\Refract` in the current Windows user profile. Uninstall leaves this data in place. Back up that directory before troubleshooting or moving to another machine. If `REFRACT_DATA_DIR` is set when launching Refract, the app uses that directory instead. The Codex CLI is installed and authenticated separately; it is not included in the installer.

After building the installer, its opt-in test can be run with `$env:REFRACT_TEST_INSTALLER='1'; npm run build; node --test build/test/installer.test.cjs`. It checks for an existing installation, then performs a silent install, upgrade and uninstall using temporary project data. Clear the environment variable afterward.

Before publishing a stable Windows release, build its installer and run the full lifecycle suite against that release's installer. Prereleases require targeted verification of their changed functionality, including automatic dependency setup when it changes; the full lifecycle suite is not required. Record verification results and investigate failures before publishing.

After packaging, `node --test build/test/installer-dependencies.test.cjs` checks the real NSIS dependency hook with the packaged helper, including reuse and failure exit codes, without registering an installation. Set `REFRACT_TEST_DEPENDENCY_DOWNLOAD=1` for the same targeted check to download and verify fresh upstream dependencies in disposable data before checking reuse (internet access required).

To exercise the signed-in Codex CLI and hidden desktop app in disposable projects, run `$env:REFRACT_REAL_CODEX_TEST='1'; npm run build; node --test build/test/real-codex.test.cjs build/test/real-codex-ui.test.cjs`, then clear the environment variable. These tests send prompts to Codex and require an existing login and network access.

On the development machine, real attachment requests timed out while Codex was starting a shell tool; plain chat and cancellation passed. If tool calls stall, inspect the configured Codex environment and consult the [Windows sandbox documentation](https://developers.openai.com/codex/windows/). Refract does not change the user's Codex sandbox settings.

To test the build bridge with a real signed-in Codex in a disposable project, run `$env:REFRACT_REAL_BUILD_BRIDGE_TEST='1'; npm run build; node --test build/test/real-codex-build.test.cjs`, then clear the variable. The real Codex test uses the configured sandbox. The direct sandbox bridge test is enabled with `$env:REFRACT_SANDBOX_BRIDGE_TEST='1'`; set `$env:REFRACT_TEST_UNELEVATED_SANDBOX='1'` only to use the unelevated fallback for that probe, which has weaker network isolation than the elevated sandbox. The equivalent OpenCode test uses `$env:REFRACT_REAL_OPENCODE_BUILD_TEST='1'; npm run build; node --test build/test/real-opencode-build.test.cjs` and requires a configured provider with available usage.

To run the desktop Codex test against `dist/win-unpacked/Refract.exe`, build the Windows app first and also set `$env:REFRACT_TEST_PACKAGED='1'` before running `build/test/real-codex-ui.test.cjs`.

To test compiler selection with a signed-in Codex CLI and a configured TeX installation, run `$env:REFRACT_REAL_ENGINE_SELECTION_TEST='1'; npm run build; node --test build/test/real-codex-engine-selection.test.cjs`, then clear the variable. This test runs GPT-6 Luna at medium reasoning on four disposable projects.

The same setting runs `build/test/desktop.test.cjs` against the packaged executable. Select individual workflows with Node's `--test-name-pattern` option; tests that depend on development-only fixtures may not apply to a package. `build/test/packaged.test.cjs` also exercises packaged workflows when the build is present.

Projects and chat history are stored in the application's Electron user-data directory. Back up that directory before removing the app. The Codex CLI retains its own session records separately.

Refract requests Codex's workspace-write sandbox for both new and resumed conversations. It shows activity while the assistant works, saves recovery snapshots before and after an operation, and blocks conflicting changes to that project while it runs. Use Stop to request cancellation; partial changes remain in the project and are included in the recovery snapshot. If Codex has created a session before stopping or failing, Refract retains it for the next message. Closing the app waits for an active operation and its final snapshot. Recovery after forced termination still needs verification.

Deleting a project moves its complete directory into the local Deleted projects list. Restoring it keeps its original project ID, files, comments, version history, chats, and attachments. Deleted projects remain in the user-data directory and are included in backups of that directory. They are permanently deleted after 30 days when Refract is running (or on its next launch).

Duplicating a project copies its files, comments, replies, and version history into an independent project. Codex chats, attachments, and deleted-file trash are not copied. ZIP exports contain project files and folders only; they do not include Refract comments, version history, chats, or trash.

### Assistant LaTeX diagnosis

During an assistant response, Refract gives either provider a temporary, project-scoped build and package lookup interface. The build uses the same configured root, engine, working directory, flags, externally configured TeX tools, bibliography passes, and environment as the editor build. Each response includes a unique ID, exit status, recent compiler output, and the PDF path when a PDF was produced. The build replaces `.refract-build/main.log`; earlier logs and PDFs are not evidence of a new build. Compiler output is capped, so inspect the current log for details. Cancellation also stops the assistant's compiler.

The package lookup uses the configured `kpsewhich` with the project working directory and inherited TeX search environment. It answers whether a `.sty` file can be found, not whether the package is compatible or complete. Refract does not install packages automatically or update its external TeX installation. Builds remain offline. When a package is missing, use a properly licensed project-local copy where suitable, or report the dependency as a blocker; redistributing any TeX component with Refract requires a separate source, build, relinking, and licensing review. A successful build and PDF signature do not establish correct layout or references: inspect the PDF and warnings separately. Visual inspection depends on the assistant provider's available tools and must be reported accurately.

### Fast assistant tools

The assistant can use `project_set_engine` to select the fastest compiler that meets the document’s requirements. It checks the build after switching between pdfLaTeX, XeLaTeX and LuaLaTeX. The selection is saved in Project Info and used by subsequent editor and assistant builds.

Both providers receive a temporary authenticated MCP connection named `refract` for each response. `project_list`, `project_read`, and `project_edit` operate directly on the active project, avoiding a new shell process for each file operation. `project_read_many` and `project_edit_many` batch up to eight independent files into one call. `build` and `package_lookup` call the configured build interface directly. The connection expires when the response finishes, including cancellation; a resumed conversation receives a fresh connection. The token is supplied through the provider process environment or per-thread app-server configuration, and is not saved in Refract chat or project configuration.

`project_edit` requires exactly one matching passage, then atomically saves and reads back the result. When the request gives the path and exact old text, the assistant can edit directly. After a read, it supplies the returned SHA-256 to detect stale context. `additional` groups up to seven more disjoint replacements against the same file contents, all validated before any write. Stale hashes, ambiguous passages, invalid UTF-8, binary files, linked files, private paths, and files larger than 2 MiB return errors. Edits preserve UTF-8 BOMs and normalize pasted multiline text to CRLF in CRLF-only files. New files and other operations remain available through the provider's normal tools. Calls are serialized per response, including builds.

The Refract MCP server exposes only project operations and bounded conversation retrieval; it cannot execute arbitrary commands. Codex automatically approves this server's tools for the active response and still runs shell commands in `workspace-write`. Codex shells use `allow_login_shell=false` to avoid loading profiles for each command; tools that depend on a shell profile should initialize their environment explicitly. OpenCode retains its own command execution behavior. This requires providers with Streamable HTTP MCP support; Codex treats a failed Refract connection as a startup error rather than silently proceeding without its tools.

Run `npm run benchmark:tools` on Windows for alternating before/after samples and raw results in `docs/benchmarks/assistant-tools.json`. See [the latency report](docs/benchmarks/assistant-tools.md) for methodology, limits, and measured results.

### Response speed and diagnostics

Codex responses reuse a project-specific app-server process. Conversations still receive their own thread IDs and fresh tool connections. Cancellation interrupts the active turn; if it does not settle within five seconds, Refract terminates that project's process tree before taking the recovery snapshot. Idle processes expire after two minutes, at most three idle/reusable processes are retained, and shutdown closes them. OpenCode 2 responses with Refract tools use a private standalone server so each response receives its current temporary connection. Before resuming a conversation, it clears that session's prior execution state so a stale OpenCode claim cannot indefinitely queue the next turn. Global OpenCode service settings are unchanged.

File reads return at most 16,000 characters by default, with `nextOffset` for more context and the full-file hash for editing. With `find`, a read returns up to 600 characters around the next exact match, and `nextOffset` continues the search. Batch reads return at most 8,000 characters per file. Batch edits report each file's result independently. A fresh or cross-provider conversation includes at most 32,000 characters of prior context. Full history remains stored, and `conversation_read` retrieves earlier messages in bounded pages. User requests are never shortened. Build logs default to the last 12,000 characters with an explicit truncation flag and an adjustable `logLimit`.

Automatic builds may reuse a successful result for up to 60 seconds within the same clock minute, after checking project content/settings/environment, the compiler, recorded TeX dependencies, and build outputs. Failed/cancelled builds are not cached. Bibliography builds are conservatively rebuilt because TeX's recorder does not capture every BibTeX/Biber or remote input. The toolbar Compile button and `build(force=true)` request real compilation. This also lets you refresh documents that intentionally use randomness or other external state.

Snapshots cache unchanged source bytes and per-file hashes using file identity, size, nanosecond modification/change times, mode, and link count. Scans validate paths and run with bounded concurrency; source buffers use a 64 MiB LRU cache. ZIP creation is deferred until the project digest changes. For changed snapshots, a separate 32 MiB cache reuses unchanged compressed entries created by this process; it never trusts hashes in editable history files. Every saved version remains an independent, backward-compatible restore archive. The new digest format may create one extra snapshot after upgrading an older project.

The model picker's **Faster replies** shortcut explicitly selects a supported low/minimal reasoning effort. It does not change your saved choice until clicked; difficult requests can still use higher effort.

For diagnostics, `store.assistantPerformance(projectId)` returns the most recent response's numeric timings for queueing, preparation, snapshots, context, provider startup/turn execution, tools, and builds, plus counts and context length. It stores no prompts, output text, file paths, or credentials and is kept only in memory. Provider turn time includes startup and tool waiting; build time is part of tool time, so these spans overlap and must not be added. Codex native command/file/search events and direct MCP operations are timed; model inference and provider transport are not separately observable, and OpenCode-native tool timings are not inferred from its CLI stream. Codex native tool outputs have a 4,000-token history budget; bounded Refract reads have separate larger budgets so requested context pages can be retrieved.

Run `npm run benchmark:pipeline` for [additional pipeline comparisons](docs/benchmarks/agent-pipeline.md) covering snapshots, cold/warm provider lifecycle work, batching, bounded reads, and real build reuse.

Run `npm run benchmark:changes -- changes.json` for the [real-agent change benchmark](docs/benchmarks/agent-changes.md), which checks exact file bytes and records token usage and backend completion time with GPT-6 Luna at medium reasoning.
