# Feature Specification: Read-Only FUSE Client Proof of Concept

**Feature Branch**: `001-fuse-client-poc` (no git branch; tracked via `.specify/feature.json`)

**Created**: 2026-11-14

**Status**: Draft

**Input**: User description: "PoC - minimal set of functionality that shows that our FUSE client works. With using as much frontier Rust libraries as possible - minimal amount of hand-written code. Prefer popular stable libraries with millions of downloads at least."

## Clarifications

### Session 2026-11-14

- Q: Which operations should the PoC mount support? → A: Read-only. Mount, list
  directories, stat, read files and symlinks written by other clients, unmount cleanly.
  All mutating operations are refused.
- Q: What happens when a block service is unreachable or a block fails its CRC check? →
  A: The read fails with EIO. No parity reconstruction in the PoC; recorded as a
  time-boxed exception to Principle III.
- Q: Which constitution gates apply? → A: Correctness, robustness, security and
  test-first apply in full. Performance is measured against ternfuse and recorded but
  does not block. Fuzzing and fault injection cover the decoders and read path the PoC
  adds.
- Q: When part of a block is read, is checking each 4 KiB page against the CRC stored
  next to it on the block service enough, or must every byte be checked against a CRC
  from metadata? → A: Page CRCs are enough. Every fetched page is checked against its
  stored CRC; inline data and any read that covers a whole span are also checked
  against the metadata CRC. This is stricter than ternfuse, which never checks the
  span CRC on its read path and trusts kernel-cached pages during recovery.
- Q: If the mount is started with `-o allow_other`, should Rustern make the kernel
  enforce the displayed permissions for other users? → A: Yes. `allow_other` is
  accepted only together with `default_permissions`, which Rustern adds automatically,
  so the kernel enforces the presented uid, gid and mode. (ternfuse passes `-o`
  through unchanged, so with `allow_other` every local user can read every file.)
- Q: When two readers want overlapping but not identical parts of the same block at
  the same time, must they share one network fetch? → A: Sharing is left to the kernel
  page cache, as in ternfuse. Rustern keeps no block cache of its own, fetches exactly
  the pages each read covers, and keeps file pages in the kernel cache across opens;
  the kernel issues one read for pages that several readers are waiting on.
- Q: With no block cache, is a total memory limit plus a per-request limit enough, or
  must memory also be capped for each open file? → A: Also cap each open file: a
  per-file limit on in-flight block bytes (default 32 MiB, configurable) alongside the
  total limit (default 256 MiB, later raised to 1 GiB), so one busy file cannot take the whole budget.

## User Scenarios & Testing *(mandatory)*

The user throughout is an engineer evaluating Rustern against an existing TernFS cluster
(a local cluster started with the upstream tooling).

### User Story 1 - Mount and browse the namespace (Priority: P1)

The engineer starts Rustern pointing at a cluster's registry and a local directory. The
directory now shows the TernFS root. They list directories, descend into
subdirectories, and see names, types, sizes and modification times that match what the
reference client shows for the same tree. When they stop Rustern, the mount point goes
away cleanly.

**Why this priority**: Without a working mount and namespace there is nothing else to
demonstrate. This alone proves Rustern can find the cluster, talk to its metadata
services and serve the kernel.

**Independent Test**: Populate a cluster through the reference client with a known tree,
mount it with Rustern, and compare a recursive listing with sizes, types and mtimes
against the same listing taken through the reference client.

**Acceptance Scenarios**:

1. **Given** a running cluster and an empty mount point, **When** the engineer starts
   Rustern with the registry address and mount point, **Then** the mount point shows the
   cluster root within 5 seconds and the process reports that it is ready.
2. **Given** a mounted cluster, **When** the engineer lists a directory, **Then** every
   entry the reference client shows is present with the same name, type, size and mtime,
   and no extra entries appear.
3. **Given** a directory with more entries than fit in one metadata response, **When**
   it is listed, **Then** every entry appears exactly once.
4. **Given** a mounted cluster, **When** the engineer looks up a name that does not
   exist, **Then** the lookup fails with ENOENT.
5. **Given** a mounted cluster, **When** the engineer stops Rustern (SIGINT or SIGTERM),
   **Then** the mount is removed and the mount point no longer reports "transport
   endpoint is not connected".

---

### User Story 2 - Read file contents correctly (Priority: P1)

The engineer opens and reads files written by the reference client: small files, large
multi-span files, and files written with different redundancy policies. The bytes they
read through Rustern are identical to the bytes the reference client returns, whether
the file is read start to finish, at random offsets, or by several processes at once.

**Why this priority**: Returning the right bytes is the point of a filesystem client.
Together with Story 1 this is the minimum that "the client works" means.

**Independent Test**: Write a corpus of files with known checksums through the reference
client, read each one through Rustern sequentially and at random offsets, and compare
checksums.

**Acceptance Scenarios**:

1. **Given** a file written by the reference client, **When** it is read sequentially
   through Rustern, **Then** its checksum matches the checksum taken through the
   reference client.
2. **Given** a multi-span file, **When** reads are issued at random offsets and lengths,
   including ones that cross span and stripe boundaries, **Then** every read returns the
   same bytes as the reference client.
3. **Given** a read at or past end of file, **When** it is issued, **Then** it returns
   zero bytes or a short read ending exactly at end of file.
4. **Given** 16 processes reading different files concurrently, **When** all reads
   finish, **Then** every checksum matches and no read waits on an unrelated file.
5. **Given** a symbolic link created by the reference client, **When** the engineer
   reads the link, **Then** the target string matches and following the link reaches the
   target.

---

### User Story 3 - Fail safely, never wrongly (Priority: P2)

When something goes wrong (a block fails verification, a storage node is down, a
metadata service is slow, someone tries to write), Rustern returns a clear error for
that one request, explains it in its logs, and keeps serving everything else. It never
returns unverified bytes and never takes the mount down.

**Why this priority**: The project exists to be more correct and robust than the
reference client. A PoC that only works on the happy path does not demonstrate that.

**Independent Test**: With a mounted cluster, corrupt a stored block, stop a storage
service, and slow a metadata service; read affected and unaffected files and try
mutating operations. Check errors, logs and that the mount stays up.

**Acceptance Scenarios**:

1. **Given** a file one of whose stored blocks has been corrupted, **When** the affected
   range is read, **Then** the read fails with EIO, a log line names the file and block,
   and reads of other ranges and files still succeed.
2. **Given** a stopped storage service holding a block of a file, **When** the affected
   range is read, **Then** the read fails with EIO after the configured deadline rather
   than hanging, and the mount stays up.
3. **Given** a mounted cluster, **When** the engineer tries to create, write, delete,
   rename, or change attributes of anything, **Then** the operation fails with EROFS and
   nothing changes in the cluster.
4. **Given** a metadata service that stops answering, **When** a lookup is issued,
   **Then** it fails with an error after the configured deadline and succeeds again once
   the service returns, without restarting Rustern.
5. **Given** the Rustern process is killed with SIGKILL, **When** the engineer runs the
   documented clean-up command, **Then** the mount point is usable again.

### Edge Cases

- Malformed, truncated, or out-of-range responses from any service: the request fails
  with EIO and is logged; the process does not exit.
- Registry unreachable at start-up: Rustern exits with a non-zero status and a message
  naming the registry address, without leaving a mount behind.
- Registry unreachable after start-up: requests that need fresh addresses fail after
  the deadline; requests served from known addresses continue.
- Shard leader change during a request: the request is retried against the new leader
  within the overall deadline.
- Zero-length files, files exactly on span and stripe boundaries, and files whose data
  is stored inline in metadata.
- Directory removed or renamed by another client while being listed: the listing ends
  with ENOENT or a consistent partial result, never a crash.
- Very long names (255 bytes) and names with non-UTF-8 bytes.
- An open file that another client deletes: reads continue to return the original bytes
  or fail with ENOENT; they never return different bytes.
- Many concurrent readers of the same file: one fetch per page, shared through the
  kernel page cache (FR-011).
- A request interrupted by the caller (signal): the request is not cancelled (DEV-005);
  it runs until it completes or reaches its deadline, and then leaves no buffers or
  background work behind.

## Requirements *(mandatory)*

### Functional Requirements

**Mounting**

- **FR-001**: Rustern MUST mount a TernFS cluster read-only at a given mount point,
  given the cluster's registry address.
- **FR-002**: Rustern MUST accept the same flag names as the reference client for the
  options it supports (at minimum the registry address, mount options, log file and
  verbosity) so it can be swapped into the same command line.
- **FR-003**: Rustern MUST mount with `nosuid,nodev` and present the uid, gid and mode
  given in configuration, with documented defaults. When `allow_other` is requested,
  Rustern MUST also apply `default_permissions`, so other local users get only the
  access the presented uid, gid and mode allow.
- **FR-004**: Rustern MUST unmount before exiting on SIGINT, SIGTERM, or a fatal error,
  and MUST exit non-zero without mounting if the registry cannot be reached at start-up.

**Namespace**

- **FR-005**: Rustern MUST support lookup, attribute retrieval, and directory listing
  (including seeking within a listing and listing with attributes) for files,
  directories and symbolic links.
- **FR-006**: Attributes MUST report the size, mtime, atime and type recorded by the
  cluster. A failed attribute fetch MUST surface as an error, never as zero or
  placeholder values.
- **FR-007**: Rustern MUST support reading symbolic link targets.

**Reading**

- **FR-008**: Rustern MUST read file contents for every storage layout the reference
  client reads (inline data, mirrored blocks, and erasure-coded blocks with all data
  blocks available).
- **FR-009**: Every byte returned to a caller MUST be verified by checksum before it is
  returned: every fetched block page against the checksum the block service stores with
  it, inline data against the checksum in the file's metadata, and, whenever a read
  covers a whole span, the span's bytes against the span checksum in metadata. A
  mismatch is treated as an unreadable block (FR-010). Rustern's own caches hold only
  bytes that passed these checks.
- **FR-010**: When a block is unavailable or fails verification, a mirrored span MUST
  be read from another mirror; if no copy is readable, or the span is erasure-coded,
  the read MUST fail with EIO. Parity reconstruction is out of scope (see Deviations
  DEV-004).
- **FR-011**: Reads of independent blocks MUST proceed concurrently. Concurrent and
  repeated reads of the same file pages MUST be served from one fetch through the
  kernel page cache (kept across opens); Rustern keeps no block cache of its own and
  fetches only the pages each kernel read request covers.

**Refusing writes**

- **FR-012**: Every mutating operation (create, write, mkdir, rmdir, unlink, rename,
  symlink, link, attribute change) MUST fail with EROFS and send no mutating request to
  the cluster. Opening a file MUST NOT update its access time on the cluster.

**Robustness**

- **FR-013**: Every network request MUST have a timeout, retry with jittered
  exponential backoff, and an overall deadline; all three MUST be configurable. Defaults
  match the reference client, except that the overall deadline for any request a
  caller is waiting on MUST default to no more than 60 seconds, because without
  interrupt support the caller cannot be signalled while it waits (DEV-005, DEV-006).
- **FR-014**: No request, response, or kernel message MUST be able to terminate the
  process; malformed input fails only the request that carried it.
- **FR-015**: Memory used for reading MUST be bounded per file (default 32 MiB of
  in-flight block data, shared by all open handles of the same file) and in total
  (default 1 GiB), with configurable
  limits, independent of file size and of values received from the network. A read
  that would exceed a limit waits for memory to be released instead of allocating; a
  file at its own limit does not delay reads of other files while total memory
  remains.

**Observability**

- **FR-016**: Rustern MUST log every error returned to a caller with enough context to
  identify the operation, inode, and cause, and MUST keep per-operation counters and
  latency histograms that can be dumped on request. Conditions that need a human MUST
  be logged as alert-level events: a block that fails checksum verification, a mirrored
  span with no readable copy, and the registry unreachable for more than 60 s.
- **FR-017**: Rustern MUST NOT log cookies, certificates or block proofs.

**Build constraint (stakeholder requirement)**

- **FR-018**: The amount of TernFS-independent code written for the PoC MUST be
  minimised by using established, widely used, stable third-party components (on the
  order of millions of downloads), consistent with the project's Technology Baseline.
  Hand-written code is reserved for TernFS-specific logic and for parts where
  bit-exactness with upstream cannot be proven for an existing component.

### Key Entities

- **Cluster**: A TernFS deployment, reached through its registry address. Pinned to
  upstream commit `5550891`.
- **Inode**: A file, directory or symbolic link, identified by a cluster-wide id;
  carries type, size, mtime and atime.
- **Directory entry**: A name in a directory pointing at an inode.
- **Span**: A contiguous byte range of a file, stored inline or as blocks with a
  recorded checksum and redundancy layout.
- **Block**: A unit of span data held by a storage service, with its own checksum.
- **Mount session**: One Rustern process serving one mount point.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: For a test corpus of at least 1,000 files and 100 directories written by
  the reference client (including empty, inline, single-span, multi-span and 1 GiB
  files), 100% of listings and 100% of file checksums read through Rustern match the
  reference client.
- **SC-002**: In 100% of injected corruption and outage scenarios, the affected read
  fails with an error and no unverified byte is ever returned.
- **SC-003**: Across the full test suite and a 30-minute mixed-read soak with injected
  faults, the mount never goes down and never leaves a dead mount point behind.
- **SC-004**: An engineer can go from a running cluster to a browsable mount with one
  command in under 1 minute, following the quickstart.
- **SC-005**: Read throughput, latency, CPU and memory are measured against the
  reference client on the same machine and cluster and recorded with the PoC (see
  Performance Targets).
- **SC-006**: Every mutating operation attempted through the mount fails, and the
  cluster's contents are byte-identical before and after the test suite.

## Performance Targets *(mandatory)*

PoC exception (Principle V, time-boxed to the next feature that touches the read path):
targets are measured and recorded on every run but do not block merges. Baseline is
ternfuse at commit `5550891`, same machine, same local cluster.

| Metric | Workload | Target | ternfuse baseline | Benchmark |
|--------|----------|--------|-------------------|-----------|
| Throughput | Sequential read of 1 GiB files, 1 MiB requests, 1/4/16 concurrent readers | >= 50% of baseline at each reader count; scaling from 1 to 4 readers >= 2x | Measured in plan | `seq-read` |
| Throughput | Random 4 KiB and 128 KiB reads within a 1 GiB file, 16 readers | >= 50% of baseline | Measured in plan | `rand-read` |
| Latency p50 / p99 | Stat of an already-looked-up file; cold lookup of a name | p99 <= 2x baseline | Measured in plan | `meta-latency` |
| Latency p50 / p99 | Listing of a 10,000-entry directory | p99 <= 2x baseline | Measured in plan | `readdir-latency` |
| CPU per GiB transferred | Sequential read, 4 readers | <= 2x baseline | Measured in plan | `seq-read` |
| Memory ceiling | 16 readers on 1 GiB files | Read buffers stay within `--memory-limit`; peak RSS recorded (no fixed ceiling: a larger limit is fine if it is faster) | Measured in plan | `seq-read` |

## Deviations from ternfuse *(mandatory)*

Compared against `go/ternfuse/ternfuse.go` at commit `5550891`.

| ID | Behaviour | ternfuse | Rustern | Rationale | Test |
|----|-----------|----------|---------|-----------|------|
| DEV-001 | Mutating operations | Supported | Fail with EROFS | PoC scope is read-only | `refuses_mutations` |
| DEV-002 | Access time on open | Sends a set-time request on every open; the `flags\|O_NOATIME != 0` check is always true | Opening a file sends no mutating request | Read-only mount; the upstream check is a defect, not intended behaviour | `open_sends_no_mutation` |
| DEV-003 | Directory attribute caching | Directory attributes cached for `-attr-cache-time` (default 1 h) | Directory attributes cached no longer than directory entries (default 250 ms) | Principle I: directories are mutable, long caching is justified only for linked files | `dir_attr_validity` |
| DEV-004 | Missing or corrupt block | Reconstructs from parity | Mirrored spans fail over to another mirror; erasure-coded spans fail with EIO | PoC exception to Principle III, time-boxed to the feature that adds parity reconstruction | `corrupt_block_eio`, `dead_block_service_eio` |
| DEV-005 | Kernel interrupt of an in-flight request | The FUSE library cancels the request context, but the client ignores it; the request runs to its deadline | Not supported in the PoC: the FUSE layer declines interrupts, so the request runs to its deadline (same observable behaviour). Interrupt support, cancelling the request and replying EINTR, is added before v1, by an upstreamed patch to the FUSE layer or by switching FUSE layers | Time-boxed known limitation; deadlines are bounded by DEV-006 | `interrupt_bounded_by_deadline` |
| DEV-006 | Overall deadline for block-service requests | 5 minutes (shards 10 s, CDC 1 min) | Default no more than 60 s for any request a caller waits on, keeping the upstream backoff schedule (initial 1 s, x1.5 growth to 10 s, ±5% jitter, upstream's jitter 0.1); configurable | Without interrupt support a caller (even under SIGKILL) waits uninterruptibly until the reply | `block_deadline_default` |

## Caching and Validity *(mandatory)*

| Cached item | Cache | Validity rule | Invalidated by |
|-------------|-------|---------------|----------------|
| File attributes | Kernel attribute cache | `-attr-cache-time` (default 1 h): linked files are immutable apart from atime/mtime, which the reference client also caches | Expiry only (another client's time change is visible after expiry) |
| Directory attributes | Kernel attribute cache | Same as directory entries (default 250 ms), see DEV-003 | Expiry |
| Directory entries, positive and negative | Kernel entry cache | `-dir-entry-cache-time` (default 250 ms), as the reference client | Expiry |
| File contents | Kernel page cache (kept across opens) | Indefinitely while the inode exists: linked file contents are immutable | Inode eviction |
| Block service and shard addresses | In-process | Refreshed from the registry on failure and periodically | Request failure, refresh |
| File span maps | In-process, bounded by entry count | Indefinitely while cached: spans of linked files are immutable | Capacity eviction |
| In-flight stat requests | In-process | Shared only while the request is in flight; nothing is kept after the reply | Reply received |

## Testing Requirements *(mandatory)*

- Test-first development is REQUIRED for this feature: every implementation task
  is preceded by tests that are written, run, and seen to fail.
- [x] Unit tests: errno mapping, flag parsing, mount options, request deadline logic.
- [x] Property tests: span, stripe and cell offset arithmetic (any byte range maps to
  the right blocks and back); CRC verification of arbitrary splits of a buffer.
- [x] Fuzz targets for every decoder of network or kernel bytes the PoC adds (registry,
  shard and block-service responses), corpus replayed in CI.
- [x] Golden vectors from upstream Go code: encoded request and response messages, and
  CRC values, for every message type the PoC uses.
- [x] Differential tests against the Go client: recursive listing and file checksums of
  the same corpus through both clients (SC-001).
- [x] Integration tests against a local cluster started with upstream `ternrun`.
  Upstream `terntests` is excluded: it exercises writes, which the PoC refuses.
- [ ] ~~POSIX conformance~~: excluded for the PoC. Most conformance suites need write
  access; read-only POSIX behaviour is covered by the integration and differential tests.
- [x] Fault injection on the read path: corrupted block, stopped block service, slow and
  unresponsive shard, unreachable registry, malformed responses, packet loss and
  reordering, SIGKILL of Rustern followed by clean-up. Write-path fault injection does
  not apply (no write path).
- [x] Concurrency: stress test with concurrent readers of the same and different files;
  model tests (loom or shuttle) of the request-id demultiplexer are postponed (plan
  CT-6).
- [x] Interoperability: files written by the reference client read through Rustern. The
  reverse direction does not apply (Rustern does not write).

## Assumptions

- A local cluster is available by building upstream with `./build.sh alpine` (Docker is
  installed) and running `ternrun -leader-only`. Go, `fusermount3` and `/dev/fuse` are
  present on the development machine.
- The reference client used for differential tests is ternfuse built from the same
  checkout at `5550891`.
- The PoC runs unprivileged and does not use eBPF close tracking or direct mount; those
  only matter for writes.
- Extended attributes, file locking, `statfs` accuracy, NFS export and multi-region
  replication are out of scope.
- Rustern is licensed `MIT OR Apache-2.0` (constitution 1.0.1). Code may be derived
  only from the Apache-licensed upstream directories (`go/msgs`, `go/client`, `go/core`,
  `go/bincodegen`), with derived files labelled as the constitution requires; ternfuse
  is used as a behavioural reference only.
- Interrupt support (DEV-005) is a deferred follow-up task, scheduled before v1.
- Feature numbering: this PoC takes number 001. The workspace layout and the local gate
  tooling that the earlier Junie plan scoped as a separate feature 001 are set up as
  part of this feature's plan.
