# ToneGuard Flow Proposal

A concrete review artifact: flow spec checks + static entropy findings.

> **For AI Agents**: Each finding includes machine-readable fix instructions in JSON format.

## Flow checks

- Files: 6
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

#### [Info] `core/src/lib.rs:3791`

**What**: Pass-through wrapper chain length 2: is_emoji_hint -> contains

**Fix options**:
1. **Inline**: Inline `is_emoji_hint` by replacing calls with direct calls to `contains`
2. **Justify**: Add justification to flow spec with reason: isolation

**For AI agents**:

```json
{
  "action": "inline",
  "alternative": "Add justification to flow spec with reason: isolation",
  "file": "core/src/lib.rs",
  "find": "is_emoji_hint\\\\s*\\\\(",
  "line": 3791,
  "replace": "contains("
}
```

**Evidence**:
- Forward-only functions: is_emoji_hint -> contains

---

#### [Info] `core/src/lib.rs:4015`

**What**: Pass-through wrapper chain length 2: sentence_has_citation -> is_match

**Fix options**:
1. **Inline**: Inline `sentence_has_citation` by replacing calls with direct calls to `is_match`
2. **Justify**: Add justification to flow spec with reason: isolation

**For AI agents**:

```json
{
  "action": "inline",
  "alternative": "Add justification to flow spec with reason: isolation",
  "file": "core/src/lib.rs",
  "find": "sentence_has_citation\\\\s*\\\\(",
  "line": 4015,
  "replace": "is_match("
}
```

**Evidence**:
- Forward-only functions: sentence_has_citation -> is_match

---

#### [Info] `core/src/lib.rs:4011`

**What**: Pass-through wrapper chain length 2: sentence_has_specifics -> is_match

**Fix options**:
1. **Inline**: Inline `sentence_has_specifics` by replacing calls with direct calls to `is_match`
2. **Justify**: Add justification to flow spec with reason: isolation

**For AI agents**:

```json
{
  "action": "inline",
  "alternative": "Add justification to flow spec with reason: isolation",
  "file": "core/src/lib.rs",
  "find": "sentence_has_specifics\\\\s*\\\\(",
  "line": 4011,
  "replace": "is_match("
}
```

**Evidence**:
- Forward-only functions: sentence_has_specifics -> is_match

---

#### [Info] `core/src/lib.rs:4019`

**What**: Pass-through wrapper chain length 2: percent_claim_is_contextual -> is_match

**Fix options**:
1. **Inline**: Inline `percent_claim_is_contextual` by replacing calls with direct calls to `is_match`
2. **Justify**: Add justification to flow spec with reason: isolation

**For AI agents**:

```json
{
  "action": "inline",
  "alternative": "Add justification to flow spec with reason: isolation",
  "file": "core/src/lib.rs",
  "find": "percent_claim_is_contextual\\\\s*\\\\(",
  "line": 4019,
  "replace": "is_match("
}
```

**Evidence**:
- Forward-only functions: percent_claim_is_contextual -> is_match

---

## How to use this document

### For humans
1. Review each finding above
2. Either fix the issue OR justify it in a flow spec
3. Re-run `dwg flow propose` to verify fixes

### For AI agents (Claude/Codex)
1. Parse the JSON blocks for each finding
2. Apply the suggested `find`/`replace` patterns
3. If justification is more appropriate, add to `flows/*.md` under `justifications:`

### Justification reasons
- `variation`: The abstraction exists because implementations will differ
- `isolation`: The wrapper isolates callers from implementation changes
- `reuse`: The duplicated code is intentionally repeated (not DRY by design)
- `policy`: Business/security policy requires this structure
- `volatility`: This code changes frequently; abstraction reduces blast radius
