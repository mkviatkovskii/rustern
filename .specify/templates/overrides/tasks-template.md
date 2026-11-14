---

description: "Task list template for feature implementation"
---

# Tasks: [FEATURE NAME]

**Input**: Design documents from `/specs/[###-feature-name]/`

**Prerequisites**: plan.md (required), spec.md (required for user stories), research.md, data-model.md, contracts/

**Tests**: REQUIRED. Constitution Principle VII: tests are written and seen to fail before the implementation. Every user story phase lists its test tasks first, and the layers come from the spec's Testing Requirements section. Every bug fix task is preceded by a test that reproduces the bug.

**Organization**: Tasks are grouped by user story to enable independent implementation and testing of each story.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependencies)
- **[Story]**: Which user story this task belongs to (e.g., US1, US2, US3)
- Include exact file paths in descriptions

## Path Conventions

- Cargo workspace: one directory per crate under `crates/`, using the crate names from plan.md
- Unit tests: `#[cfg(test)]` modules next to the code in `crates/[crate]/src/`
- Integration tests: `crates/[crate]/tests/[name].rs`
- Property tests: alongside unit or integration tests, using `proptest`
- Fuzz targets: `crates/[crate]/fuzz/fuzz_targets/[name].rs`
- Benchmarks: `crates/[crate]/benches/[name].rs`
- Deviations from ternfuse: `docs/deviations.md`

<!--
  ============================================================================
  IMPORTANT: The tasks below are SAMPLE TASKS for illustration purposes only.

  The /speckit-tasks command MUST replace these with actual tasks based on:
  - User stories from spec.md (with their priorities P1, P2, P3...)
  - Feature requirements from plan.md
  - Entities from data-model.md
  - Interfaces from contracts/
  - Test layers from the spec's Testing Requirements section
  - Benchmarks from the spec's Performance Targets section

  Tasks MUST be organized by user story so each story can be:
  - Implemented independently
  - Tested independently
  - Delivered as an MVP increment

  Every phase ends with a task that runs the merge gates from the
  constitution's Development Workflow and Quality Gates section.

  DO NOT keep these sample tasks in the generated tasks.md file.
  ============================================================================
-->

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Project initialization and basic structure

- [ ] T001 Create or update crates per implementation plan in `crates/`
- [ ] T002 Add dependencies listed in plan.md's Dependency Justification table to `crates/[crate]/Cargo.toml` with minimal features
- [ ] T003 [P] Confirm `cargo-deny` accepts the new dependencies (`deny.toml`)
- [ ] T004 Run the merge gates (fmt, clippy with warnings denied, tests, `cargo-deny`)

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Core infrastructure that MUST be complete before ANY user story can be implemented

**⚠️ CRITICAL**: No user story work can begin until this phase is complete

Examples of foundational tasks (adjust based on your project):

- [ ] T005 [P] Write failing tests for the typed error and its errno mapping in `crates/[crate]/src/error.rs`
- [ ] T006 Implement the typed error and errno mapping in `crates/[crate]/src/error.rs`
- [ ] T007 [P] Write failing property tests for the shared types in `crates/[crate]/tests/[name].rs`
- [ ] T008 Implement the shared types that all stories depend on in `crates/[crate]/src/[module].rs`
- [ ] T009 Add `tracing` spans and metrics for the new request kinds in `crates/[crate]/src/[module].rs`
- [ ] T010 Run the merge gates

**Checkpoint**: Foundation ready - user story implementation can now begin in parallel

---

## Phase 3: User Story 1 - [Title] (Priority: P1) 🎯 MVP

**Goal**: [Brief description of what this story delivers]

**Independent Test**: [How to verify this story works on its own]

### Tests for User Story 1 (REQUIRED, written first) ⚠️

> **NOTE: Write these tests FIRST, run them, and confirm they FAIL before implementation**

- [ ] T011 [P] [US1] Unit and property tests for [behaviour] in `crates/[crate]/src/[module].rs`
- [ ] T012 [P] [US1] Fuzz target for [decoder] in `crates/[crate]/fuzz/fuzz_targets/[name].rs`
- [ ] T013 [P] [US1] Integration test for [user journey] against a local cluster in `crates/[crate]/tests/[name].rs`
- [ ] T014 [P] [US1] Fault-injection test for [failure] in `crates/[crate]/tests/[name].rs`
- [ ] T015 [US1] Run the tests above and record that they fail

### Implementation for User Story 1

- [ ] T016 [P] [US1] Implement [type] in `crates/[crate]/src/[module].rs`
- [ ] T017 [US1] Implement [operation] in `crates/[crate]/src/[module].rs` (depends on T016)
- [ ] T018 [US1] Wire [operation] into the FUSE adapter in `crates/[fuse-crate]/src/[module].rs`
- [ ] T019 [US1] Record deviation DEV-[###] with its test in `docs/deviations.md`
- [ ] T020 [US1] Run the merge gates

**Checkpoint**: At this point, User Story 1 should be fully functional and testable independently

---

## Phase 4: User Story 2 - [Title] (Priority: P2)

**Goal**: [Brief description of what this story delivers]

**Independent Test**: [How to verify this story works on its own]

### Tests for User Story 2 (REQUIRED, written first) ⚠️

- [ ] T021 [P] [US2] Unit and property tests for [behaviour] in `crates/[crate]/src/[module].rs`
- [ ] T022 [P] [US2] Integration test for [user journey] in `crates/[crate]/tests/[name].rs`
- [ ] T023 [US2] Run the tests above and record that they fail

### Implementation for User Story 2

- [ ] T024 [P] [US2] Implement [type] in `crates/[crate]/src/[module].rs`
- [ ] T025 [US2] Implement [operation] in `crates/[crate]/src/[module].rs`
- [ ] T026 [US2] Integrate with User Story 1 components (if needed)
- [ ] T027 [US2] Run the merge gates

**Checkpoint**: At this point, User Stories 1 AND 2 should both work independently

---

## Phase 5: User Story 3 - [Title] (Priority: P3)

**Goal**: [Brief description of what this story delivers]

**Independent Test**: [How to verify this story works on its own]

### Tests for User Story 3 (REQUIRED, written first) ⚠️

- [ ] T028 [P] [US3] Unit and property tests for [behaviour] in `crates/[crate]/src/[module].rs`
- [ ] T029 [P] [US3] Integration test for [user journey] in `crates/[crate]/tests/[name].rs`
- [ ] T030 [US3] Run the tests above and record that they fail

### Implementation for User Story 3

- [ ] T031 [P] [US3] Implement [type] in `crates/[crate]/src/[module].rs`
- [ ] T032 [US3] Implement [operation] in `crates/[crate]/src/[module].rs`
- [ ] T033 [US3] Run the merge gates

**Checkpoint**: All user stories should now be independently functional

---

[Add more user story phases as needed, following the same pattern]

---

## Phase N: Performance, Interop & Polish

**Purpose**: Release-relevant checks that span multiple user stories

- [ ] TXXX [P] Benchmarks for every target in the spec's Performance Targets section in `crates/[crate]/benches/`, at 1, 2, 4 ... N threads
- [ ] TXXX Compare benchmark results against the ternfuse baseline and record them in `specs/[###-feature]/`
- [ ] TXXX [P] Interoperability tests: Rustern-written files read by ternfuse and the kernel module, and vice versa
- [ ] TXXX [P] Concurrency model tests (loom or shuttle) for new synchronisation primitives or state machines
- [ ] TXXX Confirm every deviation in the spec is in `docs/deviations.md` with a test
- [ ] TXXX Run quickstart.md validation
- [ ] TXXX Run the merge gates

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: No dependencies - can start immediately
- **Foundational (Phase 2)**: Depends on Setup completion - BLOCKS all user stories
- **User Stories (Phase 3+)**: All depend on Foundational phase completion
  - User stories can then proceed in parallel (if staffed)
  - Or sequentially in priority order (P1 → P2 → P3)
- **Performance, Interop & Polish (Final Phase)**: Depends on all desired user stories being complete

### User Story Dependencies

- **User Story 1 (P1)**: Can start after Foundational (Phase 2) - No dependencies on other stories
- **User Story 2 (P2)**: Can start after Foundational (Phase 2) - May integrate with US1 but should be independently testable
- **User Story 3 (P3)**: Can start after Foundational (Phase 2) - May integrate with US1/US2 but should be independently testable

### Within Each User Story

- Tests MUST be written and FAIL before implementation
- Types before operations
- Client library before FUSE adapter wiring
- Core implementation before integration
- Merge gates pass before the story is marked complete
- Story complete before moving to next priority

### Parallel Opportunities

- All Setup tasks marked [P] can run in parallel
- All Foundational tasks marked [P] can run in parallel (within Phase 2)
- Once Foundational phase completes, all user stories can start in parallel (if team capacity allows)
- All tests for a user story marked [P] can run in parallel
- Types within a story marked [P] can run in parallel
- Different user stories can be worked on in parallel by different team members

---

## Parallel Example: User Story 1

```bash
# Launch all tests for User Story 1 together:
Task: "Unit and property tests for [behaviour] in crates/[crate]/src/[module].rs"
Task: "Integration test for [user journey] in crates/[crate]/tests/[name].rs"

# Launch all types for User Story 1 together:
Task: "Implement [type] in crates/[crate]/src/[module].rs"
Task: "Implement [type] in crates/[crate]/src/[other].rs"
```

---

## Implementation Strategy

### MVP First (User Story 1 Only)

1. Complete Phase 1: Setup
2. Complete Phase 2: Foundational (CRITICAL - blocks all stories)
3. Complete Phase 3: User Story 1
4. **STOP and VALIDATE**: Test User Story 1 independently
5. Demo if ready

### Incremental Delivery

1. Complete Setup + Foundational → Foundation ready
2. Add User Story 1 → Test independently → Demo (MVP!)
3. Add User Story 2 → Test independently → Demo
4. Add User Story 3 → Test independently → Demo
5. Each story adds value without breaking previous stories

### Parallel Team Strategy

With multiple developers:

1. Team completes Setup + Foundational together
2. Once Foundational is done:
   - Developer A: User Story 1
   - Developer B: User Story 2
   - Developer C: User Story 3
3. Stories complete and integrate independently

---

## Notes

- [P] tasks = different files, no dependencies
- [Story] label maps task to specific user story for traceability
- Each user story should be independently completable and testable
- Verify tests fail before implementing
- Commit after each task or logical group
- Stop at any checkpoint to validate story independently
- Avoid: vague tasks, same file conflicts, cross-story dependencies that break independence
