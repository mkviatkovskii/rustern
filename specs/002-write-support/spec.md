# Feature Specification: Write Support

**Feature Branch**: `002-write-support` (no git branch; tracked via `.specify/feature.json`)

**Created**: 2026-11-15

**Status**: Draft

**Input**: User description: "Write support for Rustern, the Rust FUSE client for TernFS (feature 001 delivered the read-only PoC). Users must be able to create files and write them sequentially, create directories and symlinks, rename, unlink and rmdir, and set times, through the same mount, with behaviour at least matching the reference client ternfuse (ternfs/go/ternfuse/ternfuse.go): new files are transient until closed and become visible atomically when linked; writes are sequential (append at the current size, gaps zero-filled, writes behind the current size refused); spans are buffered up to the directory's span policy and written with its block and stripe policies (mirrored or Reed-Solomon) to the block services; O_TMPFILE plus linkat is supported; ftruncate can only extend a file being written; linked files are immutable except for times. Errors are explicit errnos, never silent data loss; a failed write leaves no visible partial file. Same principles as feature 001: frontier, popular Rust libraries, minimal hand-written code, no root required (root-only tunings opt-in and degrading gracefully), about 2 GiB RAM per client is acceptable and speed is preferred over memory, allow_other stays mandatory-and-explicit. Write throughput and CPU must be benchmarked against ternfuse on the local cluster like the read path, and correctness proven by differential tests (files written by Rustern read back identically through ternfuse and vice versa) and fault tests (block service and shard stalls during writes)."

## Clarifications

### Session 2026-11-15

- Q: When does a file being written become visible? → A: `close` (flush) stores and
  certifies everything written so far, so storage errors reach `close`; the last close
  (release) publishes. Lookups through the same mount wait for a pending publish. Only a
  failure of the publish step itself is logged and counted instead of returned.
- Q: Can a published file be overwritten? → A: By default no: opening it for writing fails
  with EPERM. An opt-in mount option makes `O_TRUNC` (and so `cp` and `>` onto an existing
  name) write a new file that replaces the old one when it is published.
- Q: How are mode and owner changes handled? → A: By default EOPNOTSUPP, as the reference
  client does. An opt-in mount option makes requests that match the presented uid, gid and
  mode succeed without effect, and others fail with EPERM.

## User Scenarios & Testing *(mandatory)*

The user is an engineer or a batch job using a Rustern mount of a TernFS cluster as an
ordinary filesystem: producing output files, organising them in directories, and
removing or renaming them. Terms used below:

- **Being written**: a file that has been created but not yet published. It has no
  name in its directory, so other processes and other clients cannot see it.
- **Published** (linked): the file has its name and its final content, which never
  changes again. Publishing is atomic: readers see either no file or the whole file.
- **Failed**: a file whose writing hit an error. It is never published; the cluster
  removes its stored data later.

### User Story 1 - Write a new file and read it back anywhere (Priority: P1)

A job creates a file, writes its content sequentially (any size, from empty to many
GiB), and closes it. When `close` returns success, all its data is stored; once the
last descriptor is closed the file is published under its name with its full content,
through this mount (at once), through other Rustern mounts and through the reference
client (within milliseconds). If storing fails, the job gets an error from a write, from
`fsync` or from `close`, and no partial file ever appears under the name.

**Why this priority**: Writing new files is the reason for the feature; every other
story builds on the file lifecycle defined here.

**Independent Test**: Write files of varied sizes (empty, smaller than a page, inline
size, exactly one span, many spans, several GiB) in directories with mirrored and
erasure-coded policies through Rustern, then compare their checksums read through the
reference client and through a fresh Rustern mount.

**Acceptance Scenarios**:

1. **Given** a mounted cluster, **When** a process creates a file, writes N bytes
   sequentially and closes it successfully, **Then** the file is listed with size N and
   reads back byte-identical through Rustern and through the reference client.
2. **Given** a file being written, **When** another process (or another client) lists
   the directory, **Then** the name does not appear until the writer's last descriptor
   has been closed and the file is published; a lookup through the writer's mount made
   after that `close` returns finds the file.
3. **Given** `cmd > newfile` in a shell (the shell opens the file, duplicates the
   descriptor and closes the original before `cmd` writes), **Then** the published file
   holds everything `cmd` wrote, not an empty file.
4. **Given** a file being written, **When** the writer writes at an offset beyond the
   current end, **Then** the gap reads back as zeros; **When** it writes at an offset
   before the current end, **Then** that write fails with EINVAL and the file stays
   writable.
5. **Given** a file being written, **When** `ftruncate` extends it, **Then** the new
   bytes read back as zeros; **When** `ftruncate` would shorten it, **Then** it fails
   with EOPNOTSUPP.
6. **Given** a file being written, **When** storing its data fails (no block service
   accepts it within the deadline), **Then** the failing `write`, `fsync` or `close`
   returns EIO, later writes on that file fail with the same error, and the name never
   appears.
7. **Given** a published file and the default mount options, **When** a process opens
   it for writing, **Then** the open fails with EPERM and the file is unchanged.
8. **Given** a published file and the overwrite option, **When** a process opens it
   with `O_TRUNC`, writes and closes, **Then** the name shows the new content once
   published, a reader that opened the old file before keeps reading the old content,
   and if writing fails the old file stays under the name.
9. **Given** a directory whose policy stores small files inline, **When** a file
   smaller than the inline limit is written, **Then** it is stored as the policy
   requires and reads back identically.

---

### User Story 2 - Organise the namespace (Priority: P2)

The engineer creates and removes directories, renames files and directories within and
across directories, removes files, and creates symlinks, with each change visible to
other clients as soon as the call returns.

**Why this priority**: Writing files is of limited use if their names cannot be
managed; these are also the operations shells and build tools use constantly.

**Independent Test**: Run a scripted sequence of mkdir, rename (same directory,
cross-directory file, cross-directory directory), unlink, rmdir and symlink through
Rustern, then compare the resulting tree, with types, sizes and symlink targets,
against what the reference client shows.

**Acceptance Scenarios**:

1. **Given** a directory, **When** `mkdir` creates a child, **Then** it appears empty
   with the parent's policies inherited; **When** the name exists, **Then** EEXIST.
2. **Given** a file, **When** it is renamed within its directory or into another
   directory, **Then** the old name is gone and the new name shows the same file;
   **When** a directory is renamed into its own subtree, **Then** EINVAL.
3. **Given** a rename onto a name that already holds a file, **Then** the target is
   replaced atomically (the old file stays reachable only as a snapshot, per the
   directory's policy).
4. **Given** a file, **When** it is unlinked, **Then** the name disappears; where the
   directory's snapshot policy allows immediate deletion, its storage is released
   too, as the reference client does.
5. **Given** an empty directory, **When** `rmdir` runs, **Then** it disappears;
   **When** the directory is not empty, **Then** ENOTEMPTY.
6. **Given** a directory, **When** `symlink` creates a link, **Then** `readlink`
   returns the exact target through Rustern and the reference client.
7. **Given** a request whose response was lost and was retried, **When** the first
   attempt had in fact succeeded, **Then** the caller sees success, not EEXIST or
   ENOENT (idempotent reconciliation, as the reference client does).

---

### User Story 3 - Work with ordinary tools (Priority: P3)

Common tools work on the mount without special flags: `cp` and `cp -r` into the
mount, shell redirection to a new file, `tar -x`, `rsync` of a tree into an empty
directory, `touch -d` and `mv`. Tools that write through a temporary file and rename
it over the target work. Tools that need behaviour TernFS cannot offer (editing a file
in place, random writes) fail with a clear errno rather than producing a wrong file.

**Why this priority**: Parity in the operation surface matters only if real tools run;
this story turns the first two into a usable filesystem and pins down the remaining
gaps.

**Independent Test**: Run a fixed script of tool invocations against Rustern and the
reference client; every tool either produces identical results on both or fails on
Rustern with the documented errno, and the resulting trees match.

**Acceptance Scenarios**:

1. **Given** an empty target directory, **When** `cp -r`, `tar -x` and `rsync -r` copy
   a tree with nested directories, symlinks, empty files and multi-GiB files into it,
   **Then** the result matches the source byte for byte.
2. **Given** a file, **When** `touch -d` or `utimensat` sets its access and
   modification times, **Then** the new times are visible through both clients.
3. **Given** a tool that changes mode or ownership after writing (`cp -p`,
   `rsync -a`, `tar -x` as a user), **Then** by default the change fails with
   EOPNOTSUPP (the data is written correctly and the tool reports the error); with the
   matching-attributes option, a change to the values already shown succeeds and any
   other fails with EPERM.
4. **Given** an `O_TMPFILE` file written in a directory, **When** `linkat` gives it a
   name in the same directory, **Then** it is published under that name; **When**
   the target directory differs, **Then** EXDEV; **When** it is closed without a
   name, **Then** nothing is published.

---

### User Story 4 - Writes keep up with reads (Priority: P4)

A job writing large files at full speed gets throughput at least equal to the
reference client, with less CPU, and the mount stays responsive for other readers and
writers while it runs.

**Why this priority**: Correctness comes first, but a write path slower than the
reference client would block adoption in the production use case.

**Independent Test**: Run the write benchmarks (sequential write at 1, 4 and 16
writers, small-file creation rate) on both clients on the same cluster and compare.

**Acceptance Scenarios**:

1. **Given** the write benchmarks, **When** they run on both clients, **Then** each
   meets the targets in Performance Targets.
2. **Given** 16 writers and 16 readers on the same mount, **When** they run together,
   **Then** neither group's p99 latency exceeds the targets.

### Edge Cases

- **Dead or stalled block service mid-write**: the span is written to other block
  services (the cluster is asked for replacements, excluding the failed ones) within
  the overall block deadline; only when that fails does the write fail with EIO.
- **Slow shard or shard leader change** while creating, adding spans or publishing:
  requests are retried with backoff until the overall deadline, then fail with EIO,
  matching the read path's mapping of exhausted deadlines.
- **Lost response to a publish (link)**: the retry is reconciled (the file is found
  published under the name with the expected identity) and reported as success.
- **Name taken while the file was being written** (another client created the same
  name): publishing follows the cluster's rule for an existing name and the outcome is
  reported to `close`; no data of either file is lost.
- **Two processes create the same new name in the same mount**: each gets its own
  file being written; the later publish wins or fails per the cluster's rule, and
  both outcomes are reported.
- **Writer killed (SIGKILL) or daemon killed mid-write**: nothing is published; the
  unpublished file's storage is reclaimed by the cluster's garbage collection after
  its deadline. A restarted daemon serves the mount normally.
- **Interrupted request** (signal to the writer while a span is being stored): the
  file either stays consistent for further writes or moves to Failed; it never
  publishes data the writer did not get a success for.
- **Write larger than the per-file buffer**: backpressure (the write waits), never an
  allocation driven by the request size; sparse writes and extending truncates stream
  zeros.
- **Disk-full or out-of-space reply from the cluster**: ENOSPC to the caller.
- **Name too long, invalid name bytes** (`/`, NUL): ENAMETOOLONG or EINVAL before any
  request is sent.
- **Publish fails at release** (every `close` succeeded, then the link request fails,
  for example because the cluster refuses the name): the file is not published, the
  failure is logged with the name and counted, and its storage is reclaimed by the
  cluster.
- **Replacement under the overwrite option while another client replaces the same
  name**: the later publish wins; neither publishes a mix of the two files.
- **Unmount (SIGTERM) with files being written**: in-flight writes are given the
  shutdown grace period; files not closed by then are not published, and the
  shutdown is logged with the count.
- **fsync on a file being written**: all data written so far is durably stored before
  `fsync` returns; the file is still not visible until it is published.
- **Reading a file that is being written** through the writer's own handle or another
  handle: as decided in FR-010.
- **Unreachable registry** while writing: cached addresses are used; a block service
  list that cannot be refreshed only fails new spans after their deadline.

## Requirements *(mandatory)*

### Functional Requirements

File lifecycle

- **FR-001**: Creating a regular file MUST create it in the Being-written state with
  no name visible to anyone; it MUST be published only after every byte written has
  been stored and certified by the cluster.
- **FR-002**: Writes MUST be accepted only at or after the current end of a file being
  written. A gap MUST be filled with zeros. A write before the current end MUST fail
  with EINVAL and leave the file writable.
- **FR-003**: Data MUST be stored according to the directory's span, block and stripe
  policies (inline, mirrored or Reed-Solomon), bit-compatible with the reference
  client so that either client and the kernel module read it back identically.
- **FR-004**: Every `close` of a descriptor of a file being written (each flush) MUST
  store and certify all data written so far before returning. The file MUST be
  published once, when its last descriptor is closed (release). Until then it stays
  writable through its remaining descriptors. A lookup or listing through the same
  mount that races a pending publish MUST wait for it, so a process that saw `close`
  return finds the file.
- **FR-005**: Any error discovered while storing or certifying MUST reach the caller as
  an errno from `write`, `fsync`, `close` or `linkat`, MUST move the file to Failed, and
  every later operation on it MUST fail with the same errno. A Failed file MUST NOT be
  published. A failure of the publish step at release (after every `close` succeeded)
  cannot reach a caller; it MUST be logged with the file and name and counted in the
  metrics.
- **FR-006**: `ftruncate` and size-changing `setattr` on a file being written MUST
  extend it with zeros; shortening MUST fail with EOPNOTSUPP.
- **FR-007**: `fsync` and `fdatasync` on a file being written MUST store and certify
  all data written so far before returning; they MUST NOT publish the file.
- **FR-008**: `O_TMPFILE` MUST create a file being written with no name; `linkat` from
  it into the directory it was created in MUST publish it; into any other directory
  MUST fail with EXDEV. Closing it without `linkat` MUST NOT publish it.
- **FR-009**: Symlinks MUST be created as small published files holding the target,
  readable by `readlink` through every client.
- **FR-010**: Reading a file that is being written MUST fail with EOPNOTSUPP, as in the
  reference client; once published it reads normally.
- **FR-011**: By default, opening a published file for writing (with or without
  `O_TRUNC` or `O_APPEND`) MUST fail with EPERM at `open`. With the opt-in overwrite
  mount option, `open` with `O_TRUNC` (including `creat` and `O_CREAT|O_TRUNC` on an
  existing name) MUST create a new file being written that, when published, replaces
  the old one under the same name atomically; descriptors already open on the old
  file keep reading the old content, the old file is kept as the directory's snapshot
  policy defines, and a Failed replacement leaves the old file in place. Without
  `O_TRUNC`, opening a published file for writing MUST still fail with EPERM.
- **FR-012**: `stat` of a file being written MUST report the bytes written so far
  through the writer's mount.

Namespace operations

- **FR-013**: `mkdir`, `rmdir`, `unlink`, `rename` (same directory, cross-directory
  file, cross-directory directory) and `symlink` MUST behave as in the reference
  client, including EEXIST, ENOENT, ENOTEMPTY, ENOTDIR, EISDIR and EINVAL (rename of
  a directory into its own subtree), and rename onto an existing file replacing it.
- **FR-014**: `unlink` MUST honour the directory's snapshot policy for immediate
  deletion as the reference client does.
- **FR-015**: Every mutation retried after a lost response MUST be reconciled so a
  first attempt that succeeded is reported as success (create, link, rename, unlink,
  mkdir, rmdir).
- **FR-016**: `utimensat` (atime and mtime) on published files MUST be stored in the
  cluster. Requests to change mode, owner or group MUST fail with EOPNOTSUPP by
  default. With the opt-in matching-attributes mount option, a request whose values
  equal the uid, gid and mode the mount presents for that inode MUST succeed without
  effect, and any other MUST fail with EPERM.
- **FR-017**: Names MUST be validated (length, forbidden bytes) before a request is
  sent; invalid names fail with ENAMETOOLONG or EINVAL.

Mount and limits

- **FR-018**: The mount MUST be read-write by default; `-o ro` MUST give the feature
  001 read-only behaviour (every mutation EROFS). `nosuid,nodev` stay mandatory;
  `allow_other` keeps adding `default_permissions` and fails explicitly when the
  system refuses it. Two opt-in options, both off by default, change write semantics:
  overwrite (FR-011) and matching-attributes (FR-016); their flag names are fixed in
  the CLI contract.
- **FR-019**: Memory used for buffered write data MUST be bounded per file and in total
  by configurable limits; a writer that reaches a limit MUST wait, not fail and not
  allocate beyond it.
- **FR-020**: Spans MUST be stored concurrently across files and, within a file, the
  next span MUST be filled while the previous one is being stored, within the limits
  of FR-019.
- **FR-021**: On SIGINT, SIGTERM or SIGHUP the daemon MUST stop accepting new
  mutations, give in-flight ones the shutdown grace period, and unmount; files not
  closed are not published.
- **FR-022**: Every errno returned by a mutation MUST be traceable to a log line or a
  counter, and write throughput, span store latency and publish latency MUST appear in
  the metrics dump.
- **FR-023**: The command line MUST stay compatible with the reference client's flags
  that affect writes where the semantics match; flags for privileged machinery
  (close tracking) are accepted and ignored with a warning unless this spec decides
  otherwise.

### Key Entities

- **File being written**: created, not yet visible; has a creation cookie, the
  directory it was created in, its intended name (none for `O_TMPFILE`), the policies
  resolved from that directory, the bytes stored so far and the span currently being
  filled. Moves to Published or Failed, never back.
- **Span**: a contiguous range of a file stored as one unit (inline, mirrored or
  Reed-Solomon striped across block services), with its CRC; the unit of storage and
  of certification.
- **Directory policies**: span sizes, block placement (storage class, location) and
  stripe layout, inherited from the nearest ancestor that sets them.
- **Published file**: immutable content plus mutable times; the only kind other
  processes can see.
- **Snapshot**: a previous file under a name, kept after unlink or replacement
  according to the directory's snapshot policy.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: Every file in the write corpus (sizes from 0 bytes to 4 GiB, mirrored
  and erasure-coded directories, inline-sized files) written by Rustern reads back
  byte-identical through the reference client and the kernel module, and every file
  written by the reference client reads back identically through Rustern.
- **SC-002**: Across the fault suite (block service death and stalls, shard stalls,
  packet loss, duplication and reordering, daemon and writer SIGKILL mid-write), no
  run publishes a file whose content differs from what the writer got success for,
  and every failed file leaves no visible name.
- **SC-003**: The tool script of User Story 3 produces identical trees on both
  clients, except for operations listed as deviations, which fail with their
  documented errno.
- **SC-004**: Write throughput and CPU meet the Performance Targets on the local
  cluster.
- **SC-005**: A 30-minute mixed read and write soak under periodic faults finishes
  with the mount alive, no wrong byte, and resident memory within the configured
  limits plus a fixed margin.
- **SC-006**: The upstream filesystem test suite (`terntests`), run against a Rustern
  mount, passes except for documented exclusions.

## Performance Targets *(mandatory)*

Baseline: ternfuse at commit `5550891`, same machine, same local cluster, same
directory policies. Unlike feature 001, these targets block merges once the benchmark
exists (Principle V).

| Metric | Workload | Target | ternfuse baseline | Benchmark |
|--------|----------|--------|-------------------|-----------|
| Throughput | Sequential write of new 1 GiB and 4 GiB files, 1 MiB writes, 1/2/4/8/16 concurrent writers, erasure-coded directory | >= baseline at every writer count; 1 -> 4 writers >= 2x | Measured in plan | `seq-write` |
| Throughput | Same, mirrored directory | >= baseline | Measured in plan | `seq-write-mirror` |
| Rate | Create, write 4 KiB, close; 16 writers | >= baseline files per second | Measured in plan | `small-files` |
| Latency p50 / p99 | `close` (publish) of a 1 MiB file; `mkdir`; `rename`; `unlink` | p99 <= baseline | Measured in plan | `meta-write-latency` |
| Latency p99 | 16 readers while 16 writers run | Read p99 <= 1.5x the read-only p99 | Measured in plan | `mixed-rw` |
| CPU per GiB written | Sequential write, 4 writers | <= baseline | Measured in plan | `seq-write` |
| Memory ceiling | 16 writers | Write buffers within the configured limits; peak RSS recorded (a larger limit is fine if faster, up to about 2 GiB) | Measured in plan | `seq-write` |

## Deviations from ternfuse *(mandatory)*

Compared against `go/ternfuse/ternfuse.go` at commit `5550891` (`Create`, `Write`,
`Flush`, `Setattr`, `Link`, `Rename`, `Unlink`, `Rmdir`, `Symlink`, `Mkdir`). IDs
continue `docs/deviations.md`.

| ID | Behaviour | ternfuse | Rustern | Rationale | Test |
|----|-----------|----------|---------|-----------|------|
| DEV-007 | Publish point for a file being written | Publishes on the first flush, or on the real close with the root-only eBPF close tracker | Every flush stores and certifies; release publishes; same-mount lookups wait for a pending publish | The first flush comes too early for `cmd > file` and inherited descriptors; close tracking needs root (Principle IV) | `write_publish_point` |
| DEV-008 | Write after a span store error | Later writes return the stored error; `Read` on a failed file returns it too | Same, and `fsync` stores pending data (ternfuse has no `Fsync`) | Principle I: errors reach the caller; `fsync` must mean stored | `failed_file_sticks` |
| DEV-009 | Open of a published file for writing | Succeeds, then every `write` (and `O_TRUNC`'s truncate) fails with EOPNOTSUPP | EPERM at `open` | Fail where the caller can act, not after partial work | `open_published_for_write` |
| DEV-010 | Overwrite of an existing name with `O_TRUNC` | Not supported | Opt-in: a replacement file is published over the old one (as upstream nfsd does on close) | Lets `cp` and `>` overwrite; off by default for parity | `overwrite_option` |
| DEV-011 | Mode and owner changes with the matching-attributes option | EOPNOTSUPP | Opt-in: no-op success when values match what the mount presents, EPERM otherwise (default unchanged: EOPNOTSUPP) | `cp -p`, `rsync -a` and `tar -x` succeed when nothing would change | `chmod_chown_policy` |
| DEV-012 | `unlink` or `rename` of a name whose file is still being written | ENOENT (the name is not linked yet); the file is still published at its flush | `unlink` succeeds and the file is never published; `rename` waits for a pending publish, otherwise EBUSY | The name is visible through the writer's mount; publishing after a successful unlink would resurrect it | `unlink_while_writing` |
| DEV-013 | errno mapping differences | `LOOP_IN_DIRECTORY_RENAME` → ELOOP; no-space → EIO; `BAD_COOKIE` → EACCES | EINVAL (POSIX); ENOSPC when no block service can be picked on the first attempt; EIO (a cookie error is a fault, not a permission) | Callers act on the POSIX meaning | `errno_mapping` |

## Caching and Validity *(mandatory)*

| Cached item | Cache | Validity rule | Invalidated by |
|-------------|-------|---------------|----------------|
| Attributes of a published file | Kernel attr cache | As feature 001 (`-attr-cache-time`, content immutable); times may change | Rustern's own `utimensat`, rename and unlink invalidate the kernel entries at once; other clients' time changes within the attr cache time |
| Attributes of a file being written | Kernel attr cache | Not cached (zero validity): the size changes with every write | — |
| Directory entries | Kernel dentry cache | As feature 001 (`-dir-entry-cache-time`); a lookup of a name with a pending publish waits for it instead of caching a negative entry | Rustern's own create, publish (including a replacement under the overwrite option), mkdir, rename, unlink, rmdir and symlink invalidate the affected entries at once |
| Directory listings | Kernel readdir cache | As feature 001 | Any mutation of the directory through this mount |
| Pages of a file being written | Kernel page cache | Not used: reads of a file being written are refused (FR-010) | — |
| Directory policies | In-process | Resolved when a file is created; held for that file's lifetime (a policy change applies to files created later) | Expiry as feature 001's directory info cache |
| Block service list for new spans | In-process | Refreshed from the registry periodically; a block service that fails a write is excluded for that span | Registry refresh |

## Testing Requirements *(mandatory)*

- Test-first development is REQUIRED for this feature: every implementation task
  is preceded by tests that are written, run, and seen to fail.
- [ ] Unit tests: the file lifecycle state machine (every transition and every
  rejected one), offset rules, zero fill, span cutting at policy sizes, name
  validation, errno mapping for every mutation.
- [ ] Property tests: span splitting of arbitrary write sequences reproduces the
  written bytes; stripe and parity layout for writes; CRC combination over written
  spans matches the CRC of the data.
- [ ] Fuzz targets: every new decoder (add-span, certify, link, rename, block write
  responses) with the corpus replayed in CI.
- [ ] Golden vectors: Reed-Solomon parity and span CRCs for writes generated from
  upstream Go code, compared bit for bit.
- [ ] Differential tests: the write corpus written through Rustern and read through
  ternfuse, and the reverse; namespace operation scripts compared on both clients.
- [ ] Integration tests against a local cluster, including upstream `terntests` on a
  Rustern mount with documented exclusions.
- [ ] POSIX conformance: a pjdfstest-style subset for create, write, rename, unlink,
  mkdir, rmdir, symlink and utimens, with documented exclusions for TernFS semantics.
- [ ] Fault injection: packet loss, duplication and reordering on metadata and block
  traffic; block service death and SIGSTOP during span stores; slow shards during
  publish; writer and daemon SIGKILL mid-write; lost responses to every mutation.
- [ ] Concurrency model tests: the per-file write state machine with concurrent write,
  fsync, flush and release.
- [ ] Interoperability: Rustern-written files read by ternfuse and the kernel module,
  and files written by both read by Rustern, including degraded reads of
  Rustern-written erasure-coded files with blocks missing.

## Assumptions

- The cluster runs the pinned upstream commit; directory policies on the test cluster
  cover inline, mirrored and Reed-Solomon storage.
- Writes are sequential by TernFS design; random writes and editing published files in
  place are out of scope (they fail with an errno).
- Renaming onto an existing file name replaces it as the cluster defines (the old file
  becomes a snapshot); the plan confirms this against the upstream shard and CDC.
- Writing does not require parity reconstruction on the read path, but the
  interoperability tests read Rustern-written erasure-coded files with blocks missing
  through clients that reconstruct (ternfuse, kernel module).
- The daemon runs unprivileged; eBPF close tracking is not used by default.
- Extended attributes, hard links to published files, `fallocate`, `copy_file_range`
  offload and mmap writes are out of scope (they fail with EOPNOTSUPP or EPERM as the
  kernel and FUSE define).
- About 2 GiB of memory per client is acceptable for write buffers when it buys
  throughput.
- Builds on feature 001's client library, connection pools, metrics and FUSE bridge.
