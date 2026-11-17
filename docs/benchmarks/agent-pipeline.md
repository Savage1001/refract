# Agent pipeline benchmark

Measured 2026-11-10T19:57:23.664Z, win32, Node v22.16.0, 12th Gen Intel(R) Core(TM) i7-12700F. 30 samples per path and operation after 3 warmups; alternating order. All outputs validated.

| Operation | Before median ms | After median ms | Before p95 ms | After p95 ms |
| --- | ---: | ---: | ---: | ---: |
| Unchanged snapshot scan (161 files, 10 MiB) | 129.41 | 9.44 | 138.29 | 15.02 |
| One-file snapshot edit (4 MiB unchanged assets) | 110.79 | 26.74 | 135.82 | 49.29 |
| Codex initialize + catalog (cold vs warm) | 421.94 | 1.47 | 600.19 | 2.01 |
| Read 8 files (8 RPCs vs 1 batch) | 18.96 | 3.33 | 21.94 | 5.01 |
| Read context (64 KiB vs 16k excerpt) | 2.42 | 2.24 | 4.63 | 3.23 |
| Repeated pdflatex build (force vs validated reuse) | 309.05 | 1.50 | 314.91 | 1.88 |

Unchanged snapshot reads after warmup: **0 source bytes** across the measured and warmup calls. The original scan reads the entire 10 MiB fixture each time. Snapshot history reads and filesystem validation still occur.

Batching reduces requests for 8 files from 8 to 1. Bounded reads reduce the fixture's returned context from 65536 to 16000 characters, preserving its full-file edit hash and an explicit continuation offset.

Run `npm run benchmark:pipeline` on Windows with bundled TinyTeX and authenticated Codex installed. Raw samples are in [agent-pipeline.json](agent-pipeline.json).

The snapshot baseline reproduces the unchanged-snapshot scan in commit 5158953; after uses the real Versions.snapshot implementation with its cache. The one-file edit row compares the original full snapshot writer with incremental source/compression reuse; every resulting archive is independently checked against the changed text and all unchanged binary assets outside timing. Both paths include archive and metadata writes. The Codex row compares fresh app-server initialization plus one catalog request against a catalog request on an initialized process; it includes closing the cold process and measures process lifecycle overhead, not an old codex exec turn or model inference. File rows use a real MCP client against the app's server. The build row compares forced real two-pass pdfLaTeX compilation against validation and reuse of the same successful build; cache seeding occurs outside timing. These measurements do not predict model thinking time. OpenCode already reuses its shared background service, so no second OpenCode daemon was added.
