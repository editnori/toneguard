---
name: ToneGuard Logic Flow Guardrails
description: Enforce flow specs, invariants, and complexity justification before implementing code.
version: 0.1.46
triggers:
  - architecture
  - design
  - refactor
  - flow
  - invariants
  - code-review
  - debugging
---

# ToneGuard Logic Flow Guardrails

Before writing code, make logic explicit and justify every new abstraction.

## Required: Flow Spec
For any meaningful change, write a Flow Spec:

FLOW: <name>
Entrypoint: <user action / API / job / CLI>
Inputs: <key inputs + constraints>
Happy path steps (5–12):
  1) ...
  2) ...
Outputs: <what is returned/emitted>
Side effects: <DB write, network call, file write, queue publish>
Failure modes:
  - <condition> -> <behavior>
Observability: <logs/metrics/traces>

## Required: Invariants
List 3–7 invariants that must always be true, including at least 1 failure-mode invariant.

Examples:
- If <condition>, then <must be true>.
- It is impossible for <bad thing> to happen.
- If <error>, the system must <behavior>.

## Complexity Justification (Mandatory)
Any new module/service/interface/adapter/config key must declare exactly ONE primary reason:

- Variation (multiple implementations required)
- Isolation (separates side effects for testing/reliability)
- Reuse (eliminates real duplication)
- Policy (centralizes a business rule/invariant)
- Volatility shielding (quarantines unstable dependency)

If none apply, do NOT add the abstraction.

## Design Discipline
- Prefer single-locus edits over new layers.
- Keep business logic pure where possible (functional core, imperative shell).
- Keep the end-to-end flow readable in one pass.
- Collapse pass-through wrappers that only forward arguments.
- Avoid generic naming like Manager/Handler/Helper unless truly generic.

## Post-implementation Checks
- Re-read the flow as a narrative: does the code match the steps?
- Identify where invariants are enforced and how they are tested.
- Remove unused config keys and single-implementation abstractions unless justified.
