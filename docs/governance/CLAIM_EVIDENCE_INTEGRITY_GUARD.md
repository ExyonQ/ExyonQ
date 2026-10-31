# Claim/Evidence Integrity Guard (CEI)

```text
STATUS = DEVELOPMENT_GOVERNANCE
NOT_CURRENT_PRODUCT_CAPABILITY = YES
RULE_ID = CLAIM-EVIDENCE-INTEGRITY
FAIL_MODE = FAIL_CLOSED
APPLIES_TO = development, audit, verification, qualification and documentation claims
```

This guard governs **how work is claimed**, not how the product behaves. The
ExyonQ runtime does not enforce any of this. Nothing here may be cited as a
product capability, a runtime feature, or release evidence.

It is a precursor and proving ground for the future
`CLAIM_EVIDENCE_INTEGRITY_ENGINE` already queued architecturally for V1.x. The
engine does not exist yet; this is the development-side rehearsal of its rules.

## Why it exists

Software built with AI assistance fails in a specific way: the code is
plausible, the tests are green, and the *claim* about what was proven is wider
than what was actually executed. The failure is epistemic, not syntactic, so
compilers, linters, coverage and CI do not catch it.

The guard targets sixteen recurring failure classes:

| # | Failure class |
| --- | --- |
| 1 | claim inflation |
| 2 | evidence-scope laundering |
| 3 | execution-class inflation |
| 4 | platform scope inflation |
| 5 | client scope inflation |
| 6 | partial evidence promoted to global PASS |
| 7 | hypothesis promoted to root cause |
| 8 | absence of observed failure promoted to absence of defect |
| 9 | green test promoted to contract proof |
| 10 | self-confirming implementation/test loops |
| 11 | mirrored implementation oracles |
| 12 | wrong causal-path evidence |
| 13 | stale source evidence reused for current state |
| 14 | concurrency/TOCTOU claims supported only by linear tests |
| 15 | build/cross-build evidence promoted to runtime evidence |
| 16 | unsupported or unexecuted coverage promoted to PASS |

## Core invariant

```text
CLAIM_STRENGTH <= EVIDENCE_STRENGTH
```

with the companion invariants on scope, execution class, platform, client and
source state, and the non-equivalences `HYPOTHESIS != ROOT_CAUSE`,
`NO_FAILURE_OBSERVED != ABSENCE_OF_DEFECT`, `TEST_PASS != CONTRACT_PROVEN`,
`SAME_OUTPUT != SAME_CAUSAL_PATH`, `BUILD_VERIFIED != RUNTIME_VERIFIED`,
`LIBRARY_TEST != REAL_E2E`, `PARTIAL != GLOBAL`,
`SELF_CONFIRMING_ORACLE != INDEPENDENT_PROOF`.

## Components

| Component | Location | Role |
| --- | --- | --- |
| Always-active rule | Cursor rule `claim-evidence-integrity` (local IDE config, not versioned) | Requires a claim/evidence block before any material status |
| Readonly auditor | Cursor agent `epistemic-contract-auditor` (local IDE config, not versioned) | Independently attempts to falsify material claims |
| Deterministic checker | `scripts/integrity/claim-evidence-check.sh` | Mechanical verification of structured blocks |
| Engine | `scripts/integrity/lib/claim_evidence_check.py` | Python stdlib, read-only, no product dependency |
| Adversarial cases | `scripts/integrity/fixtures/claim-evidence/` | Owner-specified cases plus negative controls |
| Independent verifier | `scripts/integrity/tests/verify-claim-evidence-cases.sh` | Black-box case verification with counterfactuals |

The rule and the agent live under the Cursor configuration tree, which this
repository does not version — see the Cursor governance policy. The checker,
the cases and the verifier are versioned here.

## The claim/evidence block

Any material status (`PASS`, `VERIFIED`, `CLOSED`, `FIXED`, `ROOT_CAUSE`,
`PRODUCTION`, `GLOBAL`, and the project synonyms below) must be preceded by a
declaration block delimited by `BEGIN_CLAIM_EVIDENCE_BLOCK` /
`END_CLAIM_EVIDENCE_BLOCK`, containing the claim side, the evidence side, the
epistemic status, and the resulting `CLAIM_EVIDENCE_COMPATIBILITY`. Working
examples live in the fixtures directory; `pass-01`, `pass-02` and `pass-03` are
the well-formed ones.

The delimiters are recognized only when alone on their line, so a document may
describe the format in prose — as this one does — without becoming a claim.

Under a material status every field of the block is required. A deleted field
is answered with `REVIEW`, so omission is never cheaper than an honest
declaration; the `omit-*` fixtures pin that behavior.

Nor is disclaiming cheaper. `NOT_APPLICABLE` says a dimension does not apply;
it does not say the dimension is satisfied. A material claim that disclaims its
platform while the evidence ran on one, disclaims the platform on *both* sides
while saying the work was executed, disclaims its client while the sentence
names one, disclaims its source binding, disclaims the effect it required, or
disclaims the causal path while offering `SAME_OUTPUT` as proof, is answered
with `REVIEW` or `FAIL` rather than PASS. Static analysis is the one honest
platform exception: reading source is the same on every machine. An `E2E` or
`REAL_RUNTIME` claim about the server's own behavior may disclaim the client,
because there is none; `REAL_CLIENT` may not.

A well-formed commit id is also not a real one. Where the checker can read the
repository it resolves every `HEAD=` and `TREE=`, so an invented id is
`UNRESOLVABLE_SOURCE_BINDING` rather than a satisfied binding. Blocks that
demonstrate the format instead of claiming anything say `SOURCE_BINDING =
ILLUSTRATIVE`, which is a declaration the reader and the auditor can see.

Together these are the largest family of smuggling paths found when the guard
was attacked. Two adversarial audits drove them, the second finding nine more
after the first ten were closed.

### Vocabulary

```text
CLAIM_STATUS (material)  PASS VERIFIED CLOSED FIXED ROOT_CAUSE PRODUCTION GLOBAL
                         VERIFIED_REAL_PRODUCTION ACCEPT APPROVED COMPLETE DONE
                         MERGE_READY RELEASE_APPROVED PROVEN
CLAIM_STATUS (other)     FAIL PARTIAL BLOCKED NOT_PROVEN REVIEW IN_PROGRESS OPEN
                         HYPOTHESIS DISPROVEN REJECT
EPISTEMIC_STATUS         PROVEN STRONGLY_SUPPORTED BOUNDED PARTIAL HYPOTHESIS
                         NOT_PROVEN DISPROVEN
EXECUTION_CLASS          STATIC_ANALYSIS BUILD UNIT INTEGRATION E2E REAL_RUNTIME
                         REAL_CLIENT FAILURE_PATH SECURITY BENCHMARK ADVERSARIAL
                         FORMAL_OR_PROPERTY
SCOPE                    FUNCTION SINGLE_CASE FIXTURE MODULE FILE PACKAGE CRATE
                         WORKSPACE REPOSITORY GLOBAL
PLATFORM                 LINUX_AMD64 LINUX_ARM64 DARWIN_ARM64 DARWIN_AMD64
                         WINDOWS_AMD64 ALL_PLATFORMS NOT_APPLICABLE
CLIENT                   MCP_STDIO CURSOR CLAUDE_DESKTOP VSCODE HTTP_CLIENT CURL
                         BROWSER ALL_CLIENTS NOT_APPLICABLE
CONCURRENCY_EVIDENCE     DETERMINISTIC_INTERLEAVING BARRIER_CONTROLLED_RACE
                         PROPERTY_TEST MODEL_CHECK STRESS_WITH_INDEPENDENT_ORACLE
                         SYSTEMATIC_STATE_SEQUENCE REAL_CONCURRENT_PROCESS_REPRO
CONTRACT_SOURCE          PROJECT_DOC EXTERNAL_SPEC RFC PUBLIC_API SCHEMA
                         PROTOCOL_SPEC OWNER_INVARIANT ARCHITECTURE_DOC
                         IMPLEMENTATION NONE UNKNOWN
ORACLE_ORIGIN            CONTRACT_DOC PROJECT_DOC ARCHITECTURE_DOC EXTERNAL_SPEC
                         RFC SCHEMA PROTOCOL_SPEC PUBLIC_API OWNER_INVARIANT
                         INDEPENDENT_IMPLEMENTATION EXTERNAL_TOOL DIFFERENTIAL
                         PROPERTY IMPLEMENTATION_DERIVED UNKNOWN
SOURCE_BINDING           REAL ILLUSTRATIVE                    (default REAL)
EFFECT (leading token)   ABSENCE_OF_DEFECT NO_DEFECT DEFECT_ABSENT
                         NO_FAILURE_OBSERVED NOT_OBSERVED NONE NOT_EXECUTED
                         POSITIVE_OBSERVATION STATE_CHANGE_OBSERVED OUTPUT_MATCHED
                         NOT_APPLICABLE
CAUSAL (leading token)   DIRECT_OBSERVATION TRACE LOG_CORRELATION INSTRUMENTATION
                         DIFFERENTIAL STATUS_CODE_ONLY SAME_OUTPUT NOT_OBSERVED
                         NONE NOT_APPLICABLE
```

`CLAIM_REQUIRED_EFFECT`, `EVIDENCE_OBSERVED_EFFECT` and `PATH_EXECUTION_EVIDENCE`
are free text prefixed by one of those leading tokens. The token is what the
checker reads, and it is what makes the difference between "no failure was
observed" and "the defect is absent" mechanical rather than a matter of
phrasing. An unrecognized leading token is `REVIEW`.

`CLAIM_EVIDENCE_COMPATIBILITY` is itself a claim. The checker compares the
declared verdict with the one it computes and reports
`DECLARED_VERDICT_INFLATED` when the declaration is the more favourable of the
two. A declaration stricter than the mechanical verdict stands: the author may
know something the checker cannot see, and the reported verdict and exit code
follow the declaration rather than overruling it.

Any status word outside the second list is treated as material, so a local
synonym cannot switch the gates off.

Execution classes and scopes are both compared with explicit supports-tables,
not numeric scores. `INTEGRATION` supports `UNIT`; `BUILD` supports only
`BUILD`; nothing on the ladder supports `FAILURE_PATH`, `SECURITY`, `BENCHMARK`
or `FORMAL_OR_PROPERTY`. `FIXTURE` and `SINGLE_CASE` support nothing but
themselves, and `MODULE` and `FILE` are incomparable — an incomparable pair is
`REVIEW`, because a gap in the model is not a proven mismatch. The tables live
in the CEI rule and in the engine.

For a workspace-wide, repository-wide or global claim the checker reads
`EVIDENCE_COMMAND` for **cargo scope flags** and nothing else. `--exclude`
narrows even alongside `--workspace`, and so does a *named* target
(`--test routing`, `--bin exyonq`): one target is one target however wide the
package set around it. The bare plural forms (`--lib`, `--tests`,
`--all-targets`) and the package flags narrow only when no `--workspace` or
`--all` is present, and package selection is also cancelled by the
`cargo run -p <runner> -- <work>` form, where the package names the runner
rather than the work. A command in any other shape leaves the breadth of the
run unchecked, which is `REVIEW` — this project's own
`cargo run -p xtask -- ci` included, and every non-cargo command too.

## Using the checker

```bash
bash scripts/integrity/claim-evidence-check.sh --selftest
bash scripts/integrity/claim-evidence-check.sh --check <file-or-directory>
```

| Exit | Meaning |
| --- | --- |
| 0 | `CEI_CHECK=PASS` — no mechanically provable mismatch |
| 1 | `CEI_CHECK=FAIL` — proven claim/evidence mismatch |
| 2 | `CHECKER_ERROR` — unparseable input or engine failure |
| 3 | `CEI_CHECK=REVIEW` — not mechanically decidable |

The checker parses structured blocks only. It does not read natural language,
and it never resolves an unrecognized token, a missing required field, or a
malformed block in favour of PASS.

**A checker PASS is not proof of the claim.** It means only that the declared
block contains no mismatch the checker can prove. Dimensions such as
self-confirming loops, contract gaps and negative space are the auditor's job,
and ultimately the owner's.

## Validation of the guard itself

The guard was not accepted because it was generated. It was attacked by an
independent adversarial auditor, which found ten blocks encoding real inflation
that the checker answered `PASS`, and eight single-gate deletions of the engine
that the selftest failed to notice. Those findings drove the disclaimed-
dimension gates above and the probe and mutant layers below.

Six levels of evidence are produced by the selftest itself, so they are
reproducible rather than historical. Run:

```bash
bash scripts/integrity/claim-evidence-check.sh --selftest
```

1. **Owner-specified cases.** The twelve cases in the owner authorization are
   fixtures, with expected outcomes recorded in `expectations.tsv` from the
   specification text rather than from checker output. Each row also carries
   the owner's own outcome word, and the verifier enforces the documented
   translation from that word to the expected verdict, so the expectations
   cannot quietly drift toward whatever the checker does. Three well-formed
   negative controls and eight fail-closed cases are added — three omission
   cases, two command-scope cases, one prose-divergence case and two
   unparseable-field cases — twenty-three fixtures in total. The probes in
   level 3 are counted alongside them in the selftest total.

2. **Counterfactual per case.** Each non-PASS row declares the minimal honest
   repair for the defects the case exhibits. The verifier applies it and
   requires the block to become PASS. A checker that rejected everything would
   fail every counterfactual. Where a case overclaims in its sentence as well
   as in its tokens, the repair narrows the sentence too; relabelling a field
   while the prose still overclaims is not a repair.

3. **One probe per finding code.** Sixty-odd probes derive from a single
   known-good template, each changing one thing. The verifier extracts the list
   of codes from the engine source and fails if any is never asserted on.
   Note what that does and does not buy: coverage is by *code*, so two gates
   that emit the same code are covered by one probe, and the second can be
   deleted without the coverage phase noticing. The mutant sweep in level 6 is
   what closes that, gate by named gate. Three probes pin the exit-code
   contract — PASS 0, FAIL 1, REVIEW 3 — because consumers gate on the status,
   not the text. Four are false-positive controls: the project's own
   `cargo run -p xtask -- ci` and `cargo test --workspace --tests` must not be
   rejected as narrowed, an incomparable scope pair must not be reported as
   inflation, and a static-analysis claim must be allowed to have no platform.

4. **Structural regressions.** A malformed block must produce a `FAIL` finding
   rather than abort the run (an abort would suppress the verdicts of every
   other block), prose mentioning the delimiters must not be parsed as a claim,
   an explicit file with no block is `REVIEW`, and this document must survive a
   scan.

5. **Live source binding.** The fixtures declare their commit ids illustrative,
   so they say nothing about the checkout they sit in, which leaves the
   git-backed path untested by them. Three probes are written inside the work
   tree instead: one claims the current source while pointing at a commit that
   is not checked out and must come back `FAIL` with `STALE_EVIDENCE`; one
   offers a fabricated commit id and must come back `UNRESOLVABLE_SOURCE_BINDING`;
   one claims a clean source and is checked against whether the tree is
   actually clean, so it asserts something either way rather than passing
   silently on a clean checkout. Skipped where git cannot answer. The real
   repository root is handed to mutant copies explicitly — deriving it from the
   script's own location would make this phase evaporate exactly where it is
   needed.

6. **Mutation test of the checker.** Two whole-verdict mutants — one always
   returning `FAIL`, one always returning `PASS` — are run against the full
   verifier, and twenty-two single-gate mutants, each switching off exactly one
   named check, are run against the fast layer. All must be caught. If a
   mutant's anchor no longer matches the engine after a refactor, the harness
   fails loudly rather than quietly testing nothing. This is what keeps a green
   selftest from being self-confirming: a checker that has stopped
   discriminating cannot pass it.

Observed on Darwin arm64 at the time of writing: `ADVERSARIAL_CASES=89`,
`ADVERSARIAL_PASS=89`, `ADVERSARIAL_FAIL=0`, `CEI_SELFTEST=PASS`, all
twenty-four mutants rejected. That line is a development observation, not
evidence; the command above is the evidence.

### What this validation does not establish

```text
EVIDENCE_PLATFORM = DARWIN_ARM64
EVIDENCE_EXECUTION_CLASS = UNIT, INTEGRATION
NOT_LINUX_EVIDENCE = YES
NOT_A_PRODUCT_CAPABILITY = YES
```

It does not show that the cases cover every real-world instance of the sixteen
failure classes, or that agents will always emit a block. The checker reads
structured fields; it only flags prose when a sweeping sentence, a causal
assertion, a platform or client mention, or concurrency vocabulary sits above
narrower fields in the same block, and it cannot see inflation in a report that
declares no block at all. Word lists are a backstop, not a language model:
wording that avoids them passes. The owner-word translation table in
`expectations.tsv` is this project's reading of the authorization, not the
owner's own mapping. A block with no populated field is treated as a
documentation template and skipped, so an empty block proves nothing and is
counted as nothing.

The mutant sweep shows that twenty-two named gates are load-bearing, not that
every gate is: a catalogue is a sample of the ways an engine can be broken.
An independent auditor built its own mutants outside the catalogue and killed
eight gates the selftest did not notice; those eight now have probes and
entries, but the same exercise repeated tomorrow may find more, and that is the
point of repeating it.

The two audits are also not independent of each other in the way a reader might
assume: the second audited the fixes made in response to the first, on the same
codebase, using probes derived from the same rule text. Neither establishes
that the sixteen failure classes are fully covered.

Those limits remain owner and auditor responsibilities.

## Layer model

```text
LEVEL 0  Integrity constitution — zero-fake, no-smoke-as-proof, no-mock-as-reality,
         no-execution-no-evidence, Linux evidence policy
LEVEL 1  Claim/evidence compatibility — this guard
LEVEL 2  Contract and oracle independence — contract verification, post-code logic audit
LEVEL 3  Language and engineering rules — Rust corpus, native verification
LEVEL 4  Task-specific verification
```

No lower level may weaken a higher one. This guard adds a layer; it replaces
nothing. Where it and an existing integrity policy overlap, the stricter
requirement applies. In particular it does not relax the project integrity
policy (`docs/governance/project-integrity.md`) or the benchmark integrity
policy (`docs/governance/benchmark-integrity.md`).

## Scope boundary

```text
DOES_NOT_CHANGE_H2_SCOPE = YES
DOES_NOT_START_H3 = YES
DOES_NOT_MODIFY_PRODUCT = YES
```

`V1X_H2_CLIENT_CONFIG_CONCURRENCY` remains the current product tranche. This
guard may govern and audit that work. It is not a reason to redesign it.
