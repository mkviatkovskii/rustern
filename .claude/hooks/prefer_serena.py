#!/usr/bin/env python3
"""PreToolUse hook: steer code search/reading on Serena-indexed code to Serena.

Blocks (with a reason shown to Claude):
  * Grep tool calls whose scope covers Rust/Go code.
  * Bash grep/rg/ag/ack/find/fd/git grep whose scope covers Rust/Go code.
  * Bash cat/head/tail/sed/awk/less/bat/nl on .rs/.go files.
  * Inline interpreter code (python -c, python - <<EOF, perl -e, node -e, ...)
    that references Rust/Go files or walks code directories.
  * bash -c / sh -c / eval wrapping any of the above.
  * Read tool calls that would load a whole .rs/.go file over READ_MAX_LINES
    (a Read with offset/limit up to READ_MAX_LINES is fine).

Allows everything else: searching docs/specs/configs, target/, vendor/,
ternfs/cpp (no C++ language server), piped grep over command output,
the Glob, Edit and Write tools, interpreters running script files, git, cargo, go, etc.

Fails open on internal errors, except that a command shlex cannot parse
(e.g. a heredoc with an unbalanced quote) is still checked with a raw scan.
"""

import json
import os
import re
import shlex
import sys

# Extensions served by Serena's language servers that we want searched via Serena.
CODE_EXT = (".rs", ".go")

# Directories (relative to project root) that contain Serena-indexed code.
CODE_ROOTS = ("crates", "tools", "src", "tests", "benches", "examples", "ternfs/go")
# Sub-trees of CODE_ROOTS that Serena does not index (grep is the right tool there).
NON_CODE_UNDER_ROOTS = ("ternfs/go/vendor",)

SEARCH_CMDS = {"grep", "egrep", "fgrep", "rg", "ag", "ack"}
FIND_CMDS = {"find", "fd", "fdfind"}
READ_CMDS = {"cat", "head", "tail", "less", "more", "bat", "batcat", "sed", "awk", "nl"}
WRAPPERS = {"sudo", "time", "nice", "command", "env", "xargs", "timeout"}
SEPARATORS = {"|", "||", "&&", ";", "&", "|&", "(", ")", "\n"}
GLOB_CHARS = "*?["

INTERPRETER_RE = re.compile(
    r"^(python[\d.]*|pypy[\d.]*|perl|ruby|node|nodejs|deno|bun|php|lua[\d.]*)$"
)
INLINE_FLAGS = {"-c", "-e", "-E", "-n", "-p", "-", "--eval", "--print", "-r"}
SHELLS = {"bash", "sh", "zsh", "dash"}
READ_MAX_LINES = 150

# A Rust/Go file reference anywhere in raw command text (paths, globs).
CODE_REF_RE = re.compile(r"[\w.~/*{}\[\]@+$-]*\.(?:rs|go)\b(?![\w(])")
# Directory walks in inline scripts: os.walk('crates'), glob('crates/**'), Path('tools').rglob(...).
LITERAL_RE = re.compile(r"""(['"])([^'"\n]+?)\1""")
# A literal is used as a path when it follows open( / Path( / an assignment / a list bracket or comma
# (p='crates/x.rs', open(f'crates/{m}.rs'), ['a.rs', 'b.rs']) - not when it's a replace()/`in` argument.
PATH_USE_RE = re.compile(r"""(?:\bopen\(|\bPath\(|(?<![=!<>])=|\[|\[[^\]()]*,)\s*[rbfRBF]{0,2}$""")
WALK_RE = re.compile(
    r"(?:walk|glob|rglob|listdir|scandir|iterdir|Path|readdirSync|opendir)\s*\(\s*[rbf]?['\"]([^'\"]+)['\"]"
)

SERENA_HINT = (
    "Switch to Serena (mcp__serena__serena_repl; load it via ToolSearch "
    "'select:mcp__serena__initial_instructions,mcp__serena__serena_repl' if not loaded): "
    "s.lsp.get_symbols_overview(path), s.lsp.find_symbol(name_path, relative_path=..., include_body=True), "
    "s.lsp.find_referencing_symbols(...), s.fs.search_for_pattern(regex, relative_path=...). "
    "To find code by what it does rather than by name, use semble (mcp__semble__search, repo = project root; "
    "load it via ToolSearch 'select:mcp__semble__search' if not loaded), then follow up with Serena. "
    "Edit code with Serena's s.edit.* functions (built-in Edit only as a fallback, after a Read with "
    "offset/limit); create new files with Write. "
    "Do NOT route around this block: python/perl/node inline scripts, heredocs, bash -c, git grep and "
    "whole-file Reads of Rust/Go code are blocked too. "
    "If only part of a compound command touched code, rerun the other parts on their own. "
    "Shell tools stay fine for non-code (docs, specs, configs, logs, target/, ternfs/go/vendor, ternfs/cpp) "
    "and for filtering command output through a pipe."
)

PROJECT_ROOT = os.path.realpath(
    os.environ.get("CLAUDE_PROJECT_DIR")
    or os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
)


def rel_to_root(path):
    """Project-relative path, or None if outside the project."""
    try:
        rel = os.path.relpath(path, PROJECT_ROOT)
    except ValueError:
        return None
    if rel == ".." or rel.startswith(".." + os.sep):
        return None
    return rel.replace(os.sep, "/")


def under(rel, prefix):
    return rel == prefix or rel.startswith(prefix + "/")


def is_code_ext(s):
    s = s.rstrip("'\"").lower()
    return s.endswith(CODE_EXT) or any(s.endswith(e + "}") for e in CODE_EXT)


def has_ext(s):
    base = s.rstrip("'\"").rsplit("/", 1)[-1].lstrip("*")
    return "." in base.strip(".") or (base.startswith(".") and len(base) > 1)


def resolve(arg, cwd):
    # Strip the glob part so "src/**/*.rs" classifies as "src".
    cut = min((arg.find(c) for c in GLOB_CHARS if c in arg), default=-1)
    base = arg if cut < 0 else os.path.dirname(arg[:cut]) or "."
    base = os.path.expanduser(base)
    return os.path.realpath(os.path.join(cwd, base))


def in_project(arg, cwd):
    """Is `arg` a resolvable path inside the project, outside non-indexed sub-trees?
    Used when filters (--include=*.rs, -name '*.go') already say "code": ~/.cargo/registry or $d stays allowed."""
    arg = os.path.expandvars(arg)
    if "$" in arg:
        return False
    rel = rel_to_root(resolve(arg, cwd))
    return rel is not None and not any(under(rel, p) for p in NON_CODE_UNDER_ROOTS)


def path_is_code(arg, cwd):
    """Does searching `arg` (file, dir or glob) cover Serena-indexed code?"""
    arg = os.path.expandvars(arg)
    if "$" in arg:
        return False  # shell variable / command substitution we can't resolve (e.g. $d -> ~/.cargo/registry)
    if is_code_ext(arg):
        rel = rel_to_root(resolve(arg, cwd))
        return rel is not None and not any(under(rel, p) for p in NON_CODE_UNDER_ROOTS)
    full = resolve(arg, cwd)
    rel = rel_to_root(full)
    if rel is None:
        return False  # outside the project (e.g. ~/.cargo/registry): Serena can't help
    is_glob = any(c in arg for c in GLOB_CHARS)
    if os.path.isfile(full) or (has_ext(arg) and (is_glob or not os.path.isdir(full))):
        return False  # a single non-code file, or a glob of them (crates/*/Cargo.toml)
    if any(under(rel, p) for p in NON_CODE_UNDER_ROOTS):
        return False
    for root in CODE_ROOTS:
        # Inside a code root, or an ancestor of one ("." or "ternfs").
        if under(rel, root) or rel == "." or under(root, rel):
            return True
    return False


def filters_verdict(globs, types):
    """'code' if filters target code, 'noncode' if they exclude it, None if no filters."""
    if any(t.lower() in ("rust", "rs", "go") for t in types):
        return "code"
    globs = [g for g in globs if not g.startswith("!")]
    if any(is_code_ext(g) for g in globs):
        return "code"
    if globs and all(has_ext(g) for g in globs):
        return "noncode"
    if types:
        return "noncode"
    return None


def deny(reason):
    print(
        json.dumps(
            {
                "hookSpecificOutput": {
                    "hookEventName": "PreToolUse",
                    "permissionDecision": "deny",
                    "permissionDecisionReason": reason + " " + SERENA_HINT,
                }
            }
        )
    )
    sys.exit(0)


# ---------------------------------------------------------------- Grep tool


def check_grep_tool(inp, cwd):
    path = inp.get("path") or cwd
    globs = [inp["glob"]] if inp.get("glob") else []
    types = [inp["type"]] if inp.get("type") else []
    verdict = filters_verdict(globs, types)
    if verdict == "noncode":
        return
    if (verdict == "code" and in_project(path, cwd)) or path_is_code(path, cwd):
        deny(f"Grep over Rust/Go code ({inp.get('path') or 'project root'}) is blocked.")


# ---------------------------------------------------------------- Bash

GREP_VALUE_OPTS = {
    "-e",
    "-f",
    "-A",
    "-B",
    "-C",
    "-m",
    "-d",
    "-D",
    "--regexp",
    "--file",
    "--max-count",
    "--context",
    "--after-context",
    "--before-context",
    "--exclude",
    "--exclude-dir",
    "--label",
    "--color",
    "--colour",
}
RG_VALUE_OPTS = GREP_VALUE_OPTS | {
    "-g",
    "--glob",
    "--iglob",
    "-t",
    "--type",
    "-T",
    "--type-not",
    "-j",
    "--threads",
    "-M",
    "--max-columns",
    "-E",
    "--encoding",
    "--sort",
    "--sortr",
    "-r",
    "--replace",
    "--pre",
    "--type-add",
    "--max-depth",
    "-d",
    "--ignore-file",
    "--path-separator",
}


def parse_search(cmd, args):
    """Return (paths, globs, types, recursive)."""
    is_grep = cmd in ("grep", "egrep", "fgrep")
    value_opts = GREP_VALUE_OPTS if is_grep else RG_VALUE_OPTS
    paths, globs, types = [], [], []
    recursive = not is_grep
    explicit_pattern = False
    i = 0
    while i < len(args):
        a = args[i]
        if a == "--":
            paths.extend(args[i + 1 :])
            break
        if a.startswith("--"):
            name, _, val = a.partition("=")
            if name in ("--recursive", "--dereference-recursive"):
                recursive = True
            if not val and name in value_opts:
                i += 1
                val = args[i] if i < len(args) else ""
            if name in ("--include", "--glob", "--iglob"):
                globs.append(val)
            elif name == "--type":
                types.append(val)
            elif name in ("--regexp", "--file"):
                explicit_pattern = True
        elif a.startswith("-") and len(a) > 1:
            flags = a[1:]
            for j, ch in enumerate(flags):
                opt = "-" + ch
                if is_grep and ch in "rR":
                    recursive = True
                if opt in value_opts:
                    val = flags[j + 1 :]
                    if not val:
                        i += 1
                        val = args[i] if i < len(args) else ""
                    if opt == "-g":
                        globs.append(val)
                    elif opt == "-t":
                        types.append(val)
                    elif opt in ("-e", "-f"):
                        explicit_pattern = True
                    break
        else:
            paths.append(a)
        i += 1
    if not explicit_pattern and paths:
        paths = paths[1:]  # first positional is the pattern
    return paths, globs, types, recursive


def parse_find(cmd, args):
    """Return (paths, name_patterns)."""
    paths, names = [], []
    if cmd == "find":
        i = 0
        while i < len(args) and not args[i].startswith(("-", "(", "!")):
            paths.append(args[i])
            i += 1
        while i < len(args):
            if args[i] in (
                "-name",
                "-iname",
                "-path",
                "-ipath",
                "-wholename",
                "-regex",
                "-iregex",
            ) and i + 1 < len(args):
                names.append(args[i + 1])
                i += 1
            i += 1
        return paths or ["."], names
    # fd [opts] [pattern] [path...]
    positional = []
    i = 0
    while i < len(args):
        a = args[i]
        if a in ("-e", "--extension") and i + 1 < len(args):
            names.append("*." + args[i + 1].lstrip("."))
            i += 1
        elif a.startswith("--extension="):
            names.append("*." + a.split("=", 1)[1].lstrip("."))
        elif a in (
            "-t",
            "--type",
            "-d",
            "--max-depth",
            "-E",
            "--exclude",
            "-j",
            "--threads",
            "-x",
            "--exec",
            "-X",
            "--exec-batch",
            "-c",
            "--color",
            "--base-directory",
            "-S",
            "--size",
            "--changed-within",
            "--changed-before",
            "-o",
            "--owner",
        ):
            i += 1
        elif not a.startswith("-"):
            positional.append(a)
        i += 1
    return (positional[1:] or ["."]), names


def split_segments(command):
    lex = shlex.shlex(command, posix=True, punctuation_chars=";&|()")
    lex.whitespace_split = True
    lex.commenters = ""
    segs, cur, piped = [], [], False
    for tok in lex:
        if tok in SEPARATORS or set(tok) <= set(";&|()"):
            if cur:
                segs.append((cur, piped))
            cur = []
            piped = tok in ("|", "|&")
            continue
        cur.append(tok)
    if cur:
        segs.append((cur, piped))
    return segs


def strip_wrappers(tokens):
    i = 0
    while i < len(tokens):
        t = tokens[i]
        if "=" in t and not t.startswith(("-", "=")) and t.split("=", 1)[0].isidentifier():
            i += 1
        elif os.path.basename(t) in WRAPPERS:
            i += 1
            while i < len(tokens) and tokens[i].startswith("-"):
                i += 1
            if os.path.basename(t) == "timeout" and i < len(tokens):
                i += 1  # duration
        else:
            break
    return tokens[i:]


def raw_code_refs(text, cwd):
    """Rust/Go files or code directories that raw script/command text points at."""
    hits = [m.group(0) for m in CODE_REF_RE.finditer(text) if path_is_code(m.group(0), cwd)]
    hits += [m.group(1) for m in WALK_RE.finditer(text) if path_is_code(m.group(1), cwd)]
    return hits


def is_inline_interpreter(cmd, args):
    """python -c / python - <<EOF / perl -e / node -e ... (not `python script.py`)."""
    if not INTERPRETER_RE.match(cmd):
        return False
    for a in args:
        if a in INLINE_FLAGS or (a.startswith("-") and not a.startswith("--") and a[-1] in "ceE"):
            return True
        if a.startswith("<"):
            return True  # stdin / heredoc with no script file
        if not a.startswith("-"):
            return False  # first positional is a script file
    return True  # bare interpreter: reads the program from stdin (pipe or heredoc)


def interpreter_code_refs(text, cwd):
    """Code paths an inline script actually opens: whole string literals ('crates/x.rs',
    f'crates/{m}.rs'), bare shell args (perl -pi -e ... crates/x.rs) and directory walks.
    Paths merely mentioned inside longer text (e.g. a tasks.md line being edited) don't count."""
    hits = [
        m.group(2)
        for m in LITERAL_RE.finditer(text)
        if CODE_REF_RE.fullmatch(m.group(2))
        and PATH_USE_RE.search(text[: m.start()])
        and path_is_code(m.group(2), cwd)
    ]
    shell_part = text.split("<<", 1)[0]
    hits += [
        w
        for w in (w.strip("'\";|&()") for w in shell_part.split())
        if is_code_ext(w) and path_is_code(w, cwd)
    ]
    hits += [m.group(1) for m in WALK_RE.finditer(text) if path_is_code(m.group(1), cwd)]
    return hits


def check_interpreter(cmd, command, cwd):
    hits = interpreter_code_refs(command, cwd)
    if hits:
        deny(
            f"Inline `{cmd}` code touching Rust/Go code ({', '.join(sorted(set(hits))[:3])}) is blocked."
        )


def raw_scan(command, cwd):
    """Fallback when shlex can't parse the command (unbalanced quotes in a heredoc, ...)."""
    words = set(re.findall(r"(?:^|[\s;&|(])([\w.\-/]+)", command))
    names = {os.path.basename(w) for w in words}
    risky = names & (SEARCH_CMDS | FIND_CMDS | READ_CMDS | SHELLS | {"eval"}) or any(
        INTERPRETER_RE.match(n) for n in names
    )
    if any(INTERPRETER_RE.match(n) for n in names):
        hits = interpreter_code_refs(command, cwd)
    else:
        hits = raw_code_refs(command, cwd) if risky else []
    if hits:
        deny("A command that reads, searches or edits Rust/Go code outside Serena is blocked.")


def check_git(args, cwd):
    i = 0
    while i < len(args) and args[i].startswith("-"):
        if args[i] == "-C" and i + 1 < len(args):
            cwd = os.path.realpath(os.path.join(cwd, os.path.expanduser(args[i + 1])))
            i += 1
        elif args[i] in ("-c", "--git-dir", "--work-tree", "--namespace") and i + 1 < len(args):
            i += 1
        i += 1
    if i >= len(args) or args[i] != "grep":
        return
    paths, globs, types, _ = parse_search("grep", args[i + 1 :])
    verdict = filters_verdict(globs + [p for p in paths if any(c in p for c in GLOB_CHARS)], types)
    if verdict == "noncode":
        return
    paths = paths or ["."]
    if (verdict == "code" and any(in_project(p, cwd) for p in paths)) or any(
        path_is_code(p, cwd) for p in paths
    ):
        deny("`git grep` over Rust/Go code is blocked.")


def check_bash(command, cwd, depth=0):
    try:
        segments = split_segments(command)
    except ValueError:
        raw_scan(command, cwd)
        return
    for tokens, piped in segments:
        tokens = strip_wrappers(tokens)
        if not tokens:
            continue
        cmd, args = os.path.basename(tokens[0]), tokens[1:]

        if cmd == "cd":
            target = args[0] if args else os.path.expanduser("~")
            cwd = os.path.realpath(os.path.join(cwd, os.path.expanduser(target)))
            continue

        if (cmd in SHELLS and "-c" in args) or cmd == "eval":
            inner = args[args.index("-c") + 1] if cmd in SHELLS else " ".join(args)
            if depth < 3 and inner:
                check_bash(inner, cwd, depth + 1)
            continue

        if is_inline_interpreter(cmd, args):
            # Scan the raw text from the interpreter on: shlex mangles heredoc bodies.
            start = command.find(tokens[0])
            check_interpreter(cmd, command[start:] if start >= 0 else command, cwd)
            continue

        if cmd == "git":
            check_git(args, cwd)
            continue

        if cmd in SEARCH_CMDS:
            paths, globs, types, recursive = parse_search(cmd, args)
            verdict = filters_verdict(globs, types)
            if verdict == "noncode":
                continue
            if not paths:
                if piped or not recursive:
                    continue  # searching stdin (command output)
                paths = ["."]
            if (verdict == "code" and any(in_project(p, cwd) for p in paths)) or any(
                path_is_code(p, cwd) for p in paths
            ):
                deny(f"`{cmd}` over Rust/Go code is blocked.")

        elif cmd in FIND_CMDS:
            paths, names = parse_find(cmd, args)
            if names and all(has_ext(n) and not is_code_ext(n) for n in names):
                continue
            if (
                any(is_code_ext(n) for n in names) and any(in_project(p, cwd) for p in paths)
            ) or any(path_is_code(p, cwd) for p in paths):
                deny(
                    f"`{cmd}` over Rust/Go code is blocked (the Glob tool or "
                    "s.fs.find_file / s.fs.list_dir are fine for locating files)."
                )

        elif cmd in READ_CMDS and any(
            is_code_ext(a) and path_is_code(a, cwd) for a in args if not a.startswith("-")
        ):
            deny(f"Reading/editing Rust/Go code with `{cmd}` is blocked.")


# ---------------------------------------------------------------- Read tool


def check_read_tool(inp, cwd):
    path = inp.get("file_path") or ""
    if not (is_code_ext(path) and path_is_code(path, cwd)):
        return
    limit = inp.get("limit")
    if isinstance(limit, int) and limit <= READ_MAX_LINES:
        return
    try:
        with open(path, "rb") as f:
            lines = sum(1 for _ in f)
    except OSError:
        return
    if lines - (inp.get("offset") or 1) + 1 <= READ_MAX_LINES:
        return
    deny(
        f"Reading {rel_to_root(os.path.realpath(path))} whole ({lines} lines) is blocked. "
        f"Find what you need with s.lsp.get_symbols_overview / s.lsp.find_symbol(include_body=True); "
        f"for surrounding lines use Read with offset and limit <= {READ_MAX_LINES} "
        f"(that also satisfies Edit's read-before-edit check)."
    )


def main():
    try:
        data = json.load(sys.stdin)
        tool = data.get("tool_name", "")
        inp = data.get("tool_input") or {}
        cwd = data.get("cwd") or os.getcwd()
        if tool == "Grep":
            check_grep_tool(inp, cwd)
        elif tool == "Bash":
            check_bash(inp.get("command", ""), cwd)
        elif tool == "Read":
            check_read_tool(inp, cwd)
    except SystemExit:
        raise
    except Exception:  # noqa: BLE001, S110 - a hook bug must not block the session: fail open
        pass
    sys.exit(0)


if __name__ == "__main__":
    main()
