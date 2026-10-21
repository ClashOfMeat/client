# Provider parity fixtures

The five `fixtures/<provider>/input` trees are synthetic HOME contents for Claude Code, Claude Cowork, Codex, Gemini CLI, and Cursor. They exercise two or more sessions per provider, progressive usage for one response, multiple models including an unrecognized model, and the applicable tool, subagent, compaction, malformed-line, and completion paths. Codex includes a completed turn, an aborted turn, and an archived rollout. Gemini includes current JSONL, legacy JSON, and a subagent recording. Cursor input is the hook spool, because Cursor does not store token usage in a transcript.

Regenerate with Node 22 and a local checkout of the old JavaScript plugin (`ClashOfMeat/plugin`, formerly
`guildbyte-plugin`), without a network connection:

```sh
node scripts/parity/generate.mjs --plugin <path-to-plugin-checkout>
node scripts/parity/generate.mjs --plugin <path-to-plugin-checkout> --check
```

Add `--provider <id>` to run one provider. Each run copies its input into a fresh temporary HOME, invokes the old provider scan once, and removes the temporary files. The generator appends a line longer than the old reader's 4 MiB cap to the temporary copy of one JSONL file per provider; the large line is intentionally absent from committed inputs. It then uses the old task payload builder and a fixed 2026-10-03 idle-finalization time. `--check` compares regenerated JSON with the committed files and exits 1 on a difference.

`expected.json` and `expected-tasks.json` are produced by the old JavaScript code. Do not hand-edit them; change synthetic inputs and regenerate instead.
