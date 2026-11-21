# Real-agent change benchmark

GPT-6 Luna at medium reasoning, standard service tier. Each version ran six scenarios, including a resumed turn, over three repetitions. The execution order was before/after, after/before, and before/after. The first scenario rotated each repetition.

Before was `1b24029007564af02a23f1145efc61387fa067e1`. After was `83b5df3ea577f420dd94a2b56ffd3b62925d757a`. Both ran on Windows with Node 22.16.0, Codex CLI 0.171.0, and the same external TinyTeX. The targeted local regression tests overlapped the first baseline repetition.

| Metric | Before | After | Change |
| --- | ---: | ---: | ---: |
| Exact turn passes | 21/21 | 21/21 | 0.0 pp |
| Median backend completion | 13.60 s | 10.59 s | −22.2% |
| P95 backend completion | 22.20 s | 16.68 s | −24.9% |
| Median last edit tool completion | 10.37 s (21 observed) | 8.00 s (21 observed) | −22.8% |
| Median provider startup | 327.50 ms | 330.12 ms | +0.8% |
| Median tokens per turn | 110,024 | 82,071 | −25.4% |
| Total tokens | 2,497,575 | 1,897,294 | −24.0% |
| Cached input tokens | 2,282,752 | 1,703,168 | −25.4% |
| Uncached input tokens | 206,659 | 186,946 | −9.5% |
| Output tokens | 8,164 | 7,180 | −12.1% |
| Completed tool calls | 89 | 56 | −37.1% |
| Median Refract prompt characters | 2,038 | 928 | −54.5% |

Both builds passed 21/21 turns. The live suite had no remaining failures to distinguish their correctness at this sample size. The candidate’s Wilson 95% lower bound was 84.5%. The candidate finished faster on 16 of 21 matched turns and used fewer total tokens on 17 of 21. Median latency rose for Repeated CRLF text (13.60 s → 14.01 s).

| Scenario | Before passes | After passes | Median seconds before → after | Median tokens before → after | Median calls before → after |
| --- | ---: | ---: | ---: | ---: | ---: |
| Simple title edit | 3/3 | 3/3 | 9.99 → 9.36 | 91,113 → 72,793 | 3 → 2 |
| Multi-file and same-file edits | 3/3 | 3/3 | 19.54 → 16.33 | 130,661 → 112,074 | 5 → 3 |
| Repeated CRLF text | 3/3 | 3/3 | 13.60 → 14.01 | 110,024 → 72,981 | 4 → 2 |
| Target near end of long file | 3/3 | 3/3 | 15.30 → 10.13 | 158,126 → 72,828 | 5 → 2 |
| Compile, repair, recompile | 3/3 | 3/3 | 20.49 → 16.68 | 173,093 → 132,838 | 7 → 5 |
| First turn | 3/3 | 3/3 | 11.55 → 10.18 | 91,105 → 84,106 | 3 → 2 |
| Resumed turn | 3/3 | 3/3 | 9.12 → 7.85 | 94,836 → 82,071 | 3 → 2 |

The guarded edit regression test covers repeated matches, stale hashes, and a failed or overlapping additional replacement leaving the original file intact. It also verifies a bounded read beyond the normal excerpt, CRLF preservation, and exact unrelated bytes. The broader local regression run passed 118 tests and skipped three, with zero failures.

[Raw samples](agent-changes.json) include every turn, token breakdown, phase timing, compiler outcomes, and completed tool names. Calibration runs are excluded from the tables.

## Reproduce

Build both revisions with `npm ci` and use the same authenticated Codex CLI and external TeX distribution. Copy [the benchmark script](../../scripts/benchmark-agent-changes.cts) into `scripts/` on the baseline checkout before building it. Run the following on each checkout, using a new output filename and repetition index each time.

```powershell
npm run build
node build/scripts/benchmark-agent-changes.cjs changes-0.json 1 0
```

The arguments are output file, repeat count, starting repetition index, and an optional scenario ID. Starting indices rotate which scenario runs first. Run one build of Refract at a time. For a comparison, alternate the order of the baseline and candidate across repetitions. The report is updated after each scenario.

## Measurement

- Each scenario gets a fresh synthetic project and a new Codex app-server. The two-turn scenario resumes the saved thread on the same app-server, with a fresh MCP connection. Both versions use the same GPT-6 Luna model, medium reasoning, standard service tier, compiler, and prompts.
- A pass requires a completed assistant response and exact expected bytes in every visible project file, including unchanged files, with no unexpected files. The build repair also requires a failed Refract build, a successful Refract rebuild, and a PDF with the expected header. A provider error, timeout, or reported model reroute counts as a failure.
- Turn time covers the backend path from submission through the pre-edit snapshot, real provider and tool calls, chat persistence, and the recovery snapshot. It excludes fixture setup, verification, CLI teardown after the scenario, and renderer work. `lastEditMs` records when the final Refract edit tool returned, where observed. `modelCompletedMs` records the provider's turn-completed notification.
- Tokens come from Codex app-server usage events. Per-turn figures subtract the previous cumulative total for a resumed thread. Totals sum every input and output token reported across inference calls. Cached input is included in total input and is also shown separately. These counts are usage telemetry.
- The median averages the middle observations for an even sample count. P95 uses nearest rank. The Wilson interval describes the observed success proportion under an independent-trial assumption. Repetitions reuse the same fixtures, so use the interval for scale and the results for this task mix.
