# Feature Specification: [FEATURE NAME]

**Feature Branch**: `[###-feature-name]`

**Created**: [DATE]

**Status**: Draft

**Input**: User description: "$ARGUMENTS"

<!--
  Rustern project override of the spec-kit spec template.
  Adds the sections the constitution requires of every feature spec:
  Performance Targets (Principle V), Deviations from ternfuse (Principle II and
  Reference Implementation and Scope), Caching and Validity (Principle I) and
  Testing Requirements (Principle VII). These sections are mandatory. A section
  that does not apply is filled in with "Not applicable" and a one-line reason;
  it is never deleted.
-->

## User Scenarios & Testing *(mandatory)*

<!--
  IMPORTANT: User stories should be PRIORITIZED as user journeys ordered by importance.
  Each user story/journey must be INDEPENDENTLY TESTABLE - meaning if you implement just ONE of them,
  you should still have a viable MVP (Minimum Viable Product) that delivers value.

  Assign priorities (P1, P2, P3, etc.) to each story, where P1 is the most critical.
  Think of each story as a standalone slice of functionality that can be:
  - Developed independently
  - Tested independently
  - Deployed independently
  - Demonstrated to users independently
-->

### User Story 1 - [Brief Title] (Priority: P1)

[Describe this user journey in plain language]

**Why this priority**: [Explain the value and why it has this priority level]

**Independent Test**: [Describe how this can be tested independently - e.g., "Can be fully tested by [specific action] and delivers [specific value]"]

**Acceptance Scenarios**:

1. **Given** [initial state], **When** [action], **Then** [expected outcome]
2. **Given** [initial state], **When** [action], **Then** [expected outcome]

---

### User Story 2 - [Brief Title] (Priority: P2)

[Describe this user journey in plain language]

**Why this priority**: [Explain the value and why it has this priority level]

**Independent Test**: [Describe how this can be tested independently]

**Acceptance Scenarios**:

1. **Given** [initial state], **When** [action], **Then** [expected outcome]

---

### User Story 3 - [Brief Title] (Priority: P3)

[Describe this user journey in plain language]

**Why this priority**: [Explain the value and why it has this priority level]

**Independent Test**: [Describe how this can be tested independently]

**Acceptance Scenarios**:

1. **Given** [initial state], **When** [action], **Then** [expected outcome]

---

[Add more user stories as needed, each with an assigned priority]

### Edge Cases

<!--
  ACTION REQUIRED: The content in this section represents placeholders.
  Fill them out with the right edge cases.

  Principle III treats partial failure as the normal case. For any feature that
  touches the network or the kernel, cover at least: malformed or out-of-range
  input, timeouts, a dead or slow block service, a shard leader change, an
  unreachable registry, an interrupted request, and the daemon being killed
  mid-operation.
-->

- What happens when [boundary condition]?
- How does system handle [error scenario]?

## Requirements *(mandatory)*

<!--
  ACTION REQUIRED: The content in this section represents placeholders.
  Fill them out with the right functional requirements.

  Every failure a requirement can produce maps to a defined errno (Principle I).
  State the errno where the caller can observe it.
-->

### Functional Requirements

- **FR-001**: System MUST [specific capability, e.g., "return the file size recorded by the shard"]
- **FR-002**: System MUST [specific capability, e.g., "reject a read whose CRC32C does not match with EIO"]
- **FR-003**: Users MUST be able to [key interaction, e.g., "list a directory with readdirplus"]
- **FR-004**: System MUST [data requirement, e.g., "report a write as failed if linking fails"]
- **FR-005**: System MUST [behavior, e.g., "log every errno returned to a caller"]

*Example of marking unclear requirements:*

- **FR-006**: System MUST detect the final close of a file via [NEEDS CLARIFICATION: mechanism not specified - flush heuristic, release, eBPF close tracking?]
- **FR-007**: System MUST cache directory entries for [NEEDS CLARIFICATION: validity period not specified]

### Key Entities *(include if feature involves data)*

- **[Entity 1]**: [What it represents, key attributes without implementation]
- **[Entity 2]**: [What it represents, relationships to other entities]

## Success Criteria *(mandatory)*

<!--
  ACTION REQUIRED: Define measurable success criteria.
  These must be technology-agnostic and measurable.
-->

### Measurable Outcomes

- **SC-001**: [Measurable metric, e.g., "Files written by Rustern read back byte-identical through ternfuse"]
- **SC-002**: [Measurable metric, e.g., "A read succeeds with up to the parity count of blocks missing"]
- **SC-003**: [Robustness metric, e.g., "No malformed response in the fuzz corpus terminates the mount"]
- **SC-004**: [Performance metric, e.g., "Sequential read throughput at least matches ternfuse on the same cluster"]

## Performance Targets *(mandatory)*

<!--
  ACTION REQUIRED: Principle V requires numeric targets in every feature spec.
  The baseline is ternfuse at the pinned upstream commit, on the same hardware
  and cluster. Each target needs a benchmark that measures it (named here, built
  in the plan). Targets must meet or beat the baseline.

  Principle VI: state thread counts for throughput targets; benchmarks run at
  1, 2, 4 ... N threads and a scalability regression is a performance regression.

  If the feature has no runtime path (for example build or tooling work), write
  "Not applicable" and one line explaining why. Do not delete the section.
-->

| Metric | Workload | Target | ternfuse baseline | Benchmark |
|--------|----------|--------|-------------------|-----------|
| Throughput | [e.g., sequential read, 1 MiB requests, 1/4/16 threads] | [e.g., >= baseline] | [measured or TBD in plan] | [name] |
| Latency p50 / p99 | [e.g., getattr on a cached inode] | [value] | [value] | [name] |
| CPU per GiB transferred | [workload] | [value] | [value] | [name] |
| Memory ceiling | [e.g., per open handle; whole process] | [value] | [value] | [name] |

## Deviations from ternfuse *(mandatory)*

<!--
  ACTION REQUIRED: ternfuse at the pinned commit is the behavioural reference
  (Principle II). Nothing is inherited by default: where ternfuse behaviour is
  ambiguous, depends on privileged machinery, or is defective, decide here.
  Every entry is copied to docs/deviations.md during implementation and needs a
  test that demonstrates the Rustern behaviour.

  If the feature matches ternfuse exactly, write "None" and name the ternfuse
  code paths that were compared.
-->

| ID | Behaviour | ternfuse | Rustern | Rationale | Test |
|----|-----------|----------|---------|-----------|------|
| DEV-001 | [e.g., errno on CRC mismatch] | [what it does] | [what we do] | [why] | [test name] |

## Caching and Validity *(mandatory)*

<!--
  ACTION REQUIRED: Principle I requires a written validity rule for anything
  placed in a kernel cache (attributes, directory entries, pages) and for any
  in-process cache whose content is served to callers. Long-lived caching is
  justified only by the immutability of linked files; say where that applies.

  If the feature caches nothing, write "Not applicable" and one line explaining why.
-->

| Cached item | Cache | Validity rule | Invalidated by |
|-------------|-------|---------------|----------------|
| [e.g., attributes of a linked file] | [kernel attr cache] | [e.g., indefinitely: linked files are immutable] | [e.g., unlink, rename] |

## Testing Requirements *(mandatory)*

<!--
  Principle VII: tests are written and seen to fail before the implementation.
  This section is the explicit test request that /speckit-tasks reads; keep the
  first bullet as written. Tick each applicable layer and describe what it covers
  for this feature; strike out a layer that does not apply and give the reason.
-->

- Test-first development is REQUIRED for this feature: every implementation task
  is preceded by tests that are written, run, and seen to fail.
- [ ] Unit tests: [scope]
- [ ] Property tests (codecs, span layout arithmetic, Reed-Solomon, CRC algebra): [scope]
- [ ] Fuzz targets for every new decoder of network or kernel bytes, corpus replayed in CI: [targets]
- [ ] Golden vectors generated from upstream C/Go code: [vectors]
- [ ] Differential tests against the Go client: [scope]
- [ ] Integration tests against a local cluster (`ternrun`), including upstream `terntests` on a Rustern mount: [scope]
- [ ] POSIX conformance, with documented exclusions: [scope]
- [ ] Fault injection (mandatory for read and write paths: packet loss, duplication and reordering; block service death; corrupted blocks; slow shards; `SIGKILL` mid-write): [scenarios]
- [ ] Concurrency model tests (loom or shuttle) for new synchronisation primitives or state machines: [scope]
- [ ] Interoperability: files written by Rustern readable by ternfuse and the kernel module, and vice versa: [scope]

## Assumptions

<!--
  ACTION REQUIRED: The content in this section represents placeholders.
  Fill them out with the right assumptions based on reasonable defaults
  chosen when the feature description did not specify certain details.
-->

- [Assumption about the cluster, e.g., "The cluster runs the pinned upstream commit"]
- [Assumption about scope boundaries, e.g., "Extended attributes are out of scope for v1"]
- [Assumption about data/environment, e.g., "The daemon runs unprivileged"]
- [Dependency on existing feature, e.g., "Requires the metadata client from feature 004"]
