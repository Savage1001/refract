# Assistant tool latency

Measured 2026-11-10T19:20:53.862Z on Windows 10.0.26300, 12th Gen Intel(R) Core(TM) i7-12700F, Node v22.16.0. Base implementation: d5004ac (v0.2.1).

| Operation | Before median ms | After median ms | Before p95 ms | After p95 ms | Median speedup |
| --- | ---: | ---: | ---: | ---: | ---: |
| List project files (1 file) | 229.14 | 1.84 | 235.01 | 2.70 | 124.6× |
| Read text (40.0 KiB UTF-8) | 246.26 | 3.45 | 329.88 | 4.43 | 71.5× |
| Verified exact edit (40.0 KiB UTF-8) | 255.51 | 6.01 | 261.78 | 8.34 | 42.5× |
| PowerShell no-op (login vs no profile) | 221.99 | 176.58 | 229.68 | 182.41 | 1.3× |
| Build dispatch (stub compiler) | 300.47 | 1.37 | 306.12 | 1.54 | 219.4× |

One-time MCP server start and SDK client initialization: 45.57 ms. This cost occurs once per assistant response, not once per tool. It excludes module loading at app startup and provider startup.

## Method

Run `npm run benchmark:tools` on Windows. Each path gets 3 warmups and 30 measured samples per operation. Execution order alternates within each pair. The fixture is one 40.0 KiB UTF-8 paper containing Unicode and CRLF line endings; it is reset outside the timer before every call. Before and after results are validated. Median is the mean of the two central sorted samples; p95 uses nearest rank. [Raw samples](assistant-tools.json) include all measurements, min/max, hardware, and setup time.

Before: fresh Windows PowerShell with its default profile behavior. The build row executes the original bridge's documented Invoke-RestMethod command against its real HTTP server. After: the real MCP SDK client makes authenticated HTTP calls to the same implementation used by the app. The shell row compares the same no-op command with and without -NoProfile. Both build implementations use an immediate stub compiler, so that row measures dispatch overhead only.

These are local execution and transport measurements, not total agent response times. They exclude model inference, remote model latency, CLI startup, provider sandbox setup, and real compilation. The file-operation baseline is a shell-based workflow; it does not measure Codex apply_patch or OpenCode's native file tools, nor establish that every prior agent call took this long. The measured shell command still takes roughly hundreds of milliseconds. Compilers and other long-running commands retain their actual execution cost.

## Reliability and integration checks

The regression tests exercise exact-match and SHA-256 preconditions, concurrent edits, read-back verification, UTF-8 BOM/CRLF preservation, private/traversal/link rejection, binary/invalid UTF-8/oversized files, authentication, origin rejection, connection expiry, and waiting for active tools at shutdown. A hidden desktop test checks both provider paths, snapshots, and that the capability is not saved in chat.

Opt-in real-provider checks (`REFRACT_REAL_TOOLS_TEST=1`, optionally `REFRACT_REAL_TOOLS_PROVIDER=opencode`) each perform two turns: read, edit, stub build, then resume the same session through a fresh capability. Both installed providers passed these checks during development. They verify wiring and session reuse; their model-dependent wall times are not benchmark samples.
