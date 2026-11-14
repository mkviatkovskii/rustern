# Deviations from ternfuse

Behaviour where Rustern intentionally differs from `go/ternfuse/ternfuse.go` at upstream
commit `5550891`. Source of truth: `specs/001-fuse-client-poc/spec.md`, "Deviations from
ternfuse". Each row names the test that proves the Rustern behaviour.

| ID | Behaviour | ternfuse | Rustern | Rationale | Test |
|----|-----------|----------|---------|-----------|------|
| DEV-001 | Mutating operations | Supported | Fail with EROFS | PoC scope is read-only | `crates/rustern/tests/refuses_mutations.rs::mutations_fail_and_cluster_is_unchanged` |
| DEV-002 | Access time on open | Sends a set-time request on every open; the `flags\|O_NOATIME != 0` check is always true | Opening a file sends no mutating request | Read-only mount; the upstream check is a defect, not intended behaviour | `crates/rustern/tests/refuses_mutations.rs::open_sends_no_mutation` |
| DEV-003 | Directory attribute caching | Directory attributes cached for `-attr-cache-time` (default 1 h) | Directory attributes cached no longer than directory entries (default 250 ms) | Principle I: directories are mutable, long caching is justified only for linked files | `crates/rustern/tests/browse.rs::dir_attr_validity` |
| DEV-004 | Missing or corrupt block | Reconstructs from parity | Mirrored spans fail over to another mirror; erasure-coded spans fail with EIO | PoC exception to Principle III, time-boxed to the feature that adds parity reconstruction | `crates/rustern/tests/faults.rs::cluster_serial::{corrupt_block_eio, dead_block_service_eio, mirrored_failover, mirrored_all_copies_alert}` |
| DEV-005 | Kernel interrupt of an in-flight request | The FUSE library cancels the request context, but the client ignores it; the request runs to its deadline | Not supported in the PoC: the FUSE layer declines interrupts, so the request runs to its deadline (same observable behaviour). Interrupt support is added before v1 | Time-boxed known limitation; deadlines are bounded by DEV-006 | `crates/rustern/tests/faults.rs::cluster_serial::interrupt_bounded_by_deadline` |
| DEV-006 | Overall deadline for block-service requests | 5 minutes (shards 10 s, CDC 1 min) | Default no more than 60 s for any request a caller waits on, keeping the upstream backoff schedule (initial 1 s, x1.5 growth to 10 s, ±5% jitter); configurable | Without interrupt support a caller (even under SIGKILL) waits uninterruptibly until the reply | `crates/rustern-client/src/config.rs::block_deadline_default`, `crates/rustern/tests/faults.rs::cluster_serial::dead_block_service_eio` |
