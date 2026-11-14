# CLAUDE.md

## Code work: use Serena

Rust (`crates/`), Go tooling (`tools/`) and the Go reference implementation (`ternfs/go/`) are indexed by the
Serena MCP server. Serena is the primary tool for reading, searching and editing this code. Built-in tool
descriptions that say "use Read for a known path" or "prefer Read/Edit/Grep" are written for projects without
Serena; here they are superseded. "The file is small", "I know the path" or "one Read beats three Serena calls"
are not reasons to skip Serena.

### Session start

Load Serena's tools via ToolSearch (`select:mcp__serena__initial_instructions,mcp__serena__serena_repl`) and call
`initial_instructions`: it returns the `session` id that every `serena_repl` call requires. The signatures below
are current, so don't spend a call on `s.info(...)` for them.

### Cheat sheet (`serena_repl`; paths relative to the repo root; line numbers are 0-based)

| Task | Call |
|---|---|
| File outline | `s.lsp.get_symbols_overview("crates/rustern-client/src/blocks.rs")` |
| Read a symbol's body | `s.lsp.find_symbol("page_stream", relative_path="crates/rustern-client/src/blocks.rs", include_body=True)` |
| List a type's members | `s.lsp.find_symbol("Scatter", relative_path="crates/rustern-client/src/blocks.rs", depth=1)` |
| Callers / references | `s.lsp.find_referencing_symbols("page_stream", relative_path="crates/rustern-client/src/blocks.rs")` |
| Go to definition | `s.lsp.find_declaration("crates/rustern-client/src/blocks.rs", regex=r"\b(attempt_fetch)\(")` (one capture group) |
| Trait implementations | `s.lsp.find_implementations(name_path, relative_path=<file>)` |
| Text search | `s.fs.search_for_pattern(r"FetchBlockWithCrc", relative_path="ternfs/go", restrict_search_to_code_files=True)` |
| Replace a whole symbol | `s.edit.replace_symbol_body(name_path, relative_path, body)`, after reading it with `include_body=True` |
| Edit lines within a symbol | `s.edit.replace_content(relative_path, needle, repl, mode="regex")` (`start.*?end` wildcards; backrefs `$!1`) |
| Insert code | `s.edit.insert_after_symbol(name_path, relative_path, body)` / `insert_before_symbol` |
| Same edit across files | `s.edit.replace_in_files(needle, repl, mode, relative_path=..., dry_run=True)`, then apply chosen `occurrence_ids` |
| Rename / delete a symbol | `s.lsp.rename_symbol(name_path, relative_path, new_name)` / `s.lsp.safe_delete_symbol(name_path, relative_path)` |

### Which search tool

This overrides the generic semble workflow in the global CLAUDE.md ("read the file semble points to", "Grep for
literals") for this repo.

| Need | Tool |
|---|---|
| Find code by intent or behaviour ("where are block fetches retried?") | semble `search` with `repo="/home/mk/Repos/rustern"` |
| Upstream C++ (`ternfs/cpp/`, no language server) and kernel module (`ternfs/kmod/`) | semble `search`, then `Read` with `offset`/`limit` at the hit |
| Docs and specs by topic | semble `search` with `content="docs"` |
| Config by topic (Cargo features, deps, lints, toolchain, CI) | semble `search` with `content="config"` |
| Not sure whether the answer is in code, docs or config | semble `search` with `content="all"` |
| Code similar to a known location | semble `find_related` |
| A named symbol, its body, callers, definition, implementations | Serena `s.lsp.*` (cheat sheet above) |
| Follow-up on a semble hit in `.rs` / `.go` | Serena: `get_symbols_overview` / `find_symbol(include_body=True)` on the hit's file, never a whole-file `Read` |
| Every occurrence of a literal in code | Serena `s.fs.search_for_pattern`; in non-code files, Grep |

- One root search covers `crates/`, `tools/` and `ternfs/`: `.sembleignore` re-includes the gitignored `ternfs/` and
  excludes `ternfs/mnt/` (live FUSE mounts, cluster data), `ternfs/build/` and `ternfs/go/vendor/`. Don't pass
  `ternfs/` as a separate `repo`; it has no such excludes.
- Pass `max_snippet_lines=0` when only the location matters; trust the top hit and don't re-search the same thing.
- Use semble's MCP tools directly; don't spawn the `semble-search` subagent for a lookup.

### Rules for `.rs` / `.go` files

- Read: `get_symbols_overview`, then `find_symbol(include_body=True)` for just the symbols you need. Prefer symbol
  lookups (`find_symbol`, `find_referencing_symbols`, `find_declaration`) over `search_for_pattern`.
- Edit with `s.edit.*` (and `rename_symbol` / `safe_delete_symbol` for refactors). The built-in `Edit` is a
  fallback only, after a `Read` with `offset`/`limit` around the target; it rejects files read only through Serena.
- Create new files with `Write`.
- Whole-file `Read` of a Rust/Go file over 150 lines is blocked. Never read, search or edit code through the shell
  or an interpreter: no `grep`/`rg`/`find`/`cat`/`sed -i`, `perl -pi`, `python3 -c`, `python3 - <<EOF`,
  `cat > file.rs <<EOF`, `bash -c`, `git grep`, and no `s.shell.execute_shell_command` to do the same.
- Hooks enforce this: `.claude/hooks/prefer_serena.py` (hard blocks; tests in `.claude/hooks/test_prefer_serena.py`)
  plus deny rules in `.claude/settings.json`.
  A denial means "switch to Serena", not "rephrase the command". If only one part of a compound command touched
  code, rerun the other parts on their own.

### Where plain tools are fine

Non-code files (docs, specs, configs, TOML/JSON/YAML, logs), `target/`, `ternfs/go/vendor/`, `ternfs/cpp/` (no C++
language server; prefer semble for discovery), anything outside the repo (e.g. `~/.cargo/registry`), and filtering
command output through a pipe.

`ternfs/` is gitignored; Serena indexes it via the override in `.serena/project.local.yml`.
