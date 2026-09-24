---
name: orchestrate-rustycore
description: "Route RustyCore development work between Sol and its worker (DeepSeek, with Luna as fallback) when a bounded independent task benefits from delegation. Use for implementation coordination or adapting this workflow, not ordinary factual answers."
---

# RustyCore orchestration

AGENTS.md owns scope, authority, validation cadence and completion. This skill owns
task routing, not another architecture plan. Delegation is explicitly requested for
useful independent work, not for every task. Keep one macrodeliverable and one parent
integrator; no per-worker issues, PRs or mandatory continuation requests.

## Routing

MiMo is outside the active workflow: do not call it or use its credentials. Its
local experimental profile must not replace the main project configuration.

- Parent Sol: `medium` default; own architecture, decomposition, coordination,
  integration and final acceptance. Delegate routine bounded implementation to the worker;
  implement directly for trivial, inseparable or critical work. Keep ambiguous
  architecture, ownership, concurrency, persistence and protocol decisions in the
  Sol parent, and record the evidence that supports them. Reasoning effort is a real
  runtime setting, not a promise in prose: do not claim a higher effort or a model
  switch unless the effective session configuration confirms it.
- `deepseek_worker`: `openrouter/deepseek-v4.1-flash` via codex-router, `max`; the
  primary worker. Implement a complete bounded responsibility with decided contracts
  and assigned files, including its tests and consumers. It also handles focused
  read-only exploration or the parent's final-check sequence when explicitly assigned
  that mode. Keep architectural decisions with Sol.
- `luna_worker`: `gpt-6-luna`, `max`; the only fallback, with the same modes and limits.
  When a `deepseek_worker` spawn or turn fails at the API level (spawn/model error,
  provider outage, quota/balance, rate limit or timeout), state it once, then give the
  same task to `luna_worker`, including the base, current diff and any partial work to
  preserve. A poor-quality result is not an API failure: review and correct it as usual.
  Sol, DeepSeek and Luna are the primary models; Claude Opus 5.5 is only the fallback below.

### Claude fallback

- Parent: if the Codex/Sol session cannot run (no Codex quota, API or client failure),
  the operator starts Claude Code in this checkout. `.claude/settings.json` selects
  `claude-opus-5-5` with `medium` effort, and it takes the Sol parent role unchanged.
  This switch is manual; Codex cannot hand a live session to Claude.
- Workers keep one order: DeepSeek → Luna → `rustycore-worker`
  (`.claude/agents/rustycore-worker.md`, inherits the Opus 5.5 parent, `low`). Use the
  Claude worker only when both DeepSeek and Luna fail at the API level, with the same
  handoff (base, current diff, partial work).
- Plain `claude`: reach DeepSeek and Luna through `codex exec` from this checkout, e.g.
  `codex exec -m openrouter/deepseek-v4.1-flash -c model_reasoning_effort=max "<task>"`
  and then `-m gpt-6-luna`, passing the role's developer instructions in the task.
- `claude-router` (codex-router launcher; parent
  `codex_router/anthropic/claude-subscription/claude-opus-5.5`): DeepSeek is the native
  `.claude/agents/deepseek-worker.md` subagent. Luna is not published there, so its
  fallback still uses `codex exec -m gpt-6-luna`. Plain Anthropic ids such as
  `claude-opus-5-5` are not served inside `claude-router`; keep agents on `inherit` or a
  `codex_router/...` id.
- Claude Opus 5.5 joins Sol, DeepSeek and Luna as an allowed model only in these
  fallback positions. State each fallback once and record the model that actually ran.

Use the actual available model/effort and record it in the handoff. Custom TOML roles
apply only on clients that load them; otherwise pass explicit supported model/effort
and the role instructions to the available delegation tool. Keep context bounded;
avoid full-history forks when they force inherited models or expose irrelevant data.
If a role/model/effort is unavailable, disclose that once and keep safe work moving
in the parent. The DeepSeek-to-Luna and Claude fallbacks above are the only allowed substitutions;
do not silently substitute other models or add another paid API fallback. A quota
or unsupported optional role is not a reason to abandon the delivery.

## One useful collaborator

Start with at most one child at a time. No worker spawns children. The parent must
have useful independent work; execute directly if delegation costs more than it saves.
Once a nontrivial implementation unit has a clear contract, independent file ownership
and useful concurrent parent work, assign it to `deepseek_worker` with a real spawn call rather than
merely describing delegation and doing it all in Sol. The parent can settle other
consumers, prepare integration or inspect a separate boundary, but must not duplicate
the child's implementation. File count alone does not require delegation. Keep tiny,
tightly coupled or unavailable-model tasks local; briefly identify the reason when a
substantial delivery stays root-only. This is not a user approval checkpoint.
Give the child the checkout/base, owned paths, objective, relevant Rust/C++ anchors,
contract/non-goals, acceptance criteria and whether checks are deferred or assigned.
Do not send secrets or full session transcripts.

Within a shared checkout, parent and child must not edit overlapping files, perform
Git mutations, or change shared generated/policy files concurrently. The parent owns
Git and integration; serialize overlapping work. Worktrees are optional when useful,
not a compulsory handoff step; they do not isolate DBs, processes, caches or network.
Permissions may inherit live session overrides. These role instructions are not a
security sandbox; keep unneeded credentials and live resources out of worker tasks.

Workers return the actual model/effort, base and final diff identity, exact paths/symbols,
changes, unexecuted tests and concrete blockers. Claims of model use require a successful
spawn trace; a configured role or default is not evidence that it ran.
The parent inspects the actual diff and consumers, not just the summary. Resolve
routine uncertainty locally; escalate on evidence, not a fixed attempt counter or
a compulsory escalation chain. Preserve useful work when changing owners.

## Acceptance and resumption

Follow AGENTS.md's complete-implementation-first cadence and exclusive validation
owner. The worker (DeepSeek, or Luna on fallback) can execute the agreed final sequence as the exclusive validation executor;
that assignment does not include another QA campaign or autonomous repairs. The parent interprets findings and assigns
corrections, then reruns affected evidence on the combined candidate as required.
Delegate only non-live checks to these roles; authorized live DB/runtime QA stays with
the parent under AGENTS.md. Do not send a worker a task its role explicitly forbids.
Freeze inputs while that sequence runs. Reuse valid evidence for unchanged inputs;
do not rerun successful commands merely because ownership passed between agents.
Do not delegate a single shell command merely to consume a collaborator's quota.

The parent's diff inspection is required; an additional reviewer agent or automated
review request is not. Preserve any explicit external contribution/review requirements.
No automatic review loop, new approval gate, or permission to publish/runtime-write.

Use the existing task/checkpoint for material decisions and final evidence, with a
short handoff in the conversation for active child/process IDs and remaining work.
Do not create a competing orchestration ledger for every helper. On resume reconcile
Git and running processes before continuing; do not replay completed operations.
Report actual time/usage/rework when available, otherwise unknown. This initial
profile is not benchmark-proven and does not enforce a hard CPU/RAM/token budget.

Configuration shape checked against
[Codex subagents](https://learn.chatgpt.com/docs/agent-configuration/subagents).
The Sol-orchestrator/Luna-executor topology is also used by
[donvito's template](https://github.com/donvito/codex-astra-luna-orchestrator);
this project deliberately omits its broad mandatory-delegation triggers and staged
tester/reviewer pipeline.
New project defaults require a fresh session and a trusted project configuration;
inspect effective settings rather than assuming files changed an existing session.
