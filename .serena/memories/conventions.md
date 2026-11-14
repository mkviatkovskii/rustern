# Conventions

## Licensing (easy to get wrong)
- Project code: `MIT OR Apache-2.0`.
- Upstream `go/ternfuse` is GPL: behavioural reference only, NEVER translate or copy its code.
- Upstream `go/msgs`, `go/client`, `go/core`, `go/bincodegen` (Apache-2.0 WITH LLVM-exception) may be adapted. Derived files carry upstream copyright line + `SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception`, are listed in `NOTICE`, and their crate declares `(MIT OR Apache-2.0) AND Apache-2.0 WITH LLVM-exception`.
- Protocol facts and golden vectors from running upstream code are not derived code.

## Architecture rules
- FUSE dependency only in `rustern-fuse` and `rustern`; gate checks the other crates have none.
- Typed errors map to errno in exactly one place (rustern-fuse `errno`).
- Hand-written code limited to TernFS logic; prefer established crates for FUSE, retries, caching, CLI, logging.
- Behaviour where ternfuse is ambiguous/privileged/defective: explicit decision recorded in `docs/deviations.md`; nothing inherited by default.
- Debug impls redact certificates/cookies.
- ternfuse CLI flag names accepted, including single-dash form.

## Workflow
- spec-kit sequence: constitution -> specify -> clarify -> plan -> tasks -> implement (`speckit-*` skills). Feature dirs `specs/NNN-name/`.
- Every plan has a Constitution Check per principle I-VIII; violations justified in Complexity Tracking with expiry.
- Test-first: each story's tasks start with failing tests.
- Review: two reviewers for unsafe, wire-format code, write path.
