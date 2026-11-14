# Implementation Plan: [FEATURE]

**Branch**: `[###-feature-name]` | **Date**: [DATE] | **Spec**: [link]

**Input**: Feature specification from `/specs/[###-feature-name]/spec.md`

**Note**: This template is filled in by the `/speckit-plan` command; its definition describes the execution workflow.

<!--
  Rustern project override of the spec-kit plan template. Technical Context
  defaults come from the constitution's Technology Baseline; the Constitution
  Check is one row per principle, as Development Workflow and Quality Gates
  requires.
-->

## Summary

[Extract from feature spec: primary requirement + technical approach from research]

## Technical Context

<!--
  ACTION REQUIRED: Replace the content in this section with the technical details
  for this feature. Defaults below come from the constitution's Technology
  Baseline; replacing a baseline entry needs a decision record in research.md
  with benchmark or risk evidence.
-->

**Language/Version**: Rust stable, edition 2024, pinned via `rust-toolchain.toml`

**Primary Dependencies**: [Baseline crates this feature uses, e.g., `fuser`, `tokio`, `bytes`. Each NEW dependency is listed in the Dependency Justification table below]

**Storage**: TernFS cluster at upstream pin `[commit]` (protocol reference: `go/msgs`, `go/client`, `go/core`)

**Testing**: [e.g., `cargo-nextest`, `proptest`, `cargo-fuzz`, `loom`/`shuttle`, `turmoil`, local cluster via `ternrun`, or NEEDS CLARIFICATION]

**Target Platform**: Linux, x86_64 primary

**Project Type**: Library-first Cargo workspace with a FUSE daemon binary

**Crates Touched**: [Which workspace crates change, and confirmation that the FUSE adapter gains no protocol logic and the client gains no FUSE types]

**Performance Goals**: [Copy targets from the spec's Performance Targets section and name the benchmark for each]

**Constraints**: [e.g., memory per handle, queue depths, retry deadlines, no steady-state allocation on the hot path]

**Scale/Scope**: [e.g., file sizes, directory sizes, concurrent handles, thread counts benchmarked]

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

<!--
  ACTION REQUIRED: Address every principle individually. Status is PASS, N/A
  (with reason) or VIOLATION. A VIOLATION is either removed by changing the
  design or justified in Complexity Tracking with the simpler alternative that
  was rejected.
-->

| Principle | Status | Evidence in this plan | Complexity ref |
|-----------|--------|-----------------------|----------------|
| I. Correctness Above All: CRC-verified reads, writes certified before success, every failure an errno, cache validity rules, write lifecycle as an exhaustive type | [PASS/N/A/VIOLATION] | [where in the design] | [row or -] |
| II. Protocol Fidelity: generated wire types, bit-identical layout/RS/CRC proven by golden vectors, interop, deviations recorded in `docs/deviations.md` | | | |
| III. The Mount Must Not Die: no panics on request paths, all input validated, everything bounded, timeouts/backoff/idempotency, cancel-safety, unmount before exit | | | |
| IV. Secure by Default: no `unsafe` outside designated crates, least privilege, `nosuid,nodev`, no secrets in logs, justified dependencies, fuzz target per decoder | | | |
| V. Performance Is a Measured Requirement: numeric targets with benchmarks, hot-path rules, optimisation motivated by a profile | | | |
| VI. Parallel by Design: no global lock, bounded concurrent fan-out, coalescing, CPU work off reactor threads, model and stress tests, scalability tracked | | | |
| VII. Test-First, Verified Against Reality: test layers from the spec's Testing Requirements, fault injection on read/write paths | | | |
| VIII. Observable and Operable: structured logs, counters and histograms, errno traceable to a log line or metric, `ternfuse` flag compatibility | | | |
| Technology Baseline and Architecture constraints | | | |
| Reference Implementation and Scope: within v1 parity, licence constraint on derived code | | | |

### Dependency Justification

<!-- Principle IV: every new dependency needs a written justification and minimal feature flags. Write "None" if no dependency is added. -->

| Crate | Purpose | Why not in-tree or an existing dependency | Features enabled | `cargo-deny` clean |
|-------|---------|-------------------------------------------|------------------|--------------------|
| [crate] | [purpose] | [reason] | [features] | [yes/no] |

## Project Structure

### Documentation (this feature)

```text
specs/[###-feature]/
├── plan.md              # This file (/speckit-plan command output)
├── research.md          # Phase 0 output (/speckit-plan command)
├── data-model.md        # Phase 1 output (/speckit-plan command)
├── quickstart.md        # Phase 1 output (/speckit-plan command)
├── contracts/           # Phase 1 output (/speckit-plan command)
└── tasks.md             # Phase 2 output (/speckit-tasks command - NOT created by /speckit-plan)
```

### Source Code (repository root)
<!--
  ACTION REQUIRED: Replace the tree below with the concrete layout for this
  feature, using the real crate names in the workspace. The constitution
  requires separate crates for protocol types, core algorithms (CRC,
  Reed-Solomon, parity), the async TernFS client, and the FUSE adapter.
  Show only the crates and files this feature touches.
-->

```text
Cargo.toml                 # workspace manifest, shared [workspace.lints]
crates/
├── [proto-crate]/         # generated protocol types and codecs
│   ├── src/
│   ├── tests/
│   └── fuzz/
├── [core-crate]/          # CRC32C, Reed-Solomon, parity
│   ├── src/
│   ├── tests/
│   └── benches/
├── [client-crate]/        # async TernFS client, no FUSE types
│   ├── src/
│   └── tests/
├── [fuse-crate]/          # FUSE adapter, no protocol logic
│   ├── src/
│   └── tests/
└── [binary-crate]/        # thin daemon binary
    └── src/main.rs
docs/
└── deviations.md          # deliberate differences from ternfuse
```

**Structure Decision**: [Document the crates touched and reference the real
directories captured above]

## Complexity Tracking

> **Fill ONLY if Constitution Check has violations that must be justified**

Exceptions are time-boxed and reviewed at the next release (Governance).

| Violation | Why Needed | Simpler Alternative Rejected Because | Expires |
|-----------|------------|-------------------------------------|---------|
| [e.g., `unsafe` in a SIMD kernel] | [current need] | [why the scalar path is insufficient] | [release] |
| [e.g., >5% benchmark regression] | [specific problem] | [why it cannot be avoided] | [release] |
