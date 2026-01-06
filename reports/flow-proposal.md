# ToneGuard Flow Proposal

A concrete review artifact: flow spec checks + static entropy findings.

## Flow checks

- Files: 7
- Errors: 0
- Warnings: 0

## Audit summary

- Files scanned: 8
- Findings: 4
- By category:
  - passthrough: 4
- By language:
  - rust: 4

## Findings

### PassThrough (4)

- [Info] ./core/src/lib.rs:4015: Pass-through wrapper chain length 2: sentence_has_citation -> is_match
  - Evidence:
    - Forward-only functions: sentence_has_citation -> is_match
- [Info] ./core/src/lib.rs:4011: Pass-through wrapper chain length 2: sentence_has_specifics -> is_match
  - Evidence:
    - Forward-only functions: sentence_has_specifics -> is_match
- [Info] ./core/src/lib.rs:3791: Pass-through wrapper chain length 2: is_emoji_hint -> contains
  - Evidence:
    - Forward-only functions: is_emoji_hint -> contains
- [Info] ./core/src/lib.rs:4019: Pass-through wrapper chain length 2: percent_claim_is_contextual -> is_match
  - Evidence:
    - Forward-only functions: percent_claim_is_contextual -> is_match

## Next steps

- Either reduce the finding, or justify it in a flow spec under `justifications` (reason: variation/isolation/reuse/policy/volatility).
- Keep logic in a readable, end-to-end flow: fewer hops, fewer knobs, clearer invariants.
