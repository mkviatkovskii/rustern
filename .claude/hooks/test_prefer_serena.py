#!/usr/bin/env python3
"""Allow/deny regression cases for prefer_serena.py. Run: python3 -I .claude/hooks/test_prefer_serena.py"""

import json
import os
import subprocess
import sys

ROOT = os.path.realpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", ".."))
HOOK = os.path.join(ROOT, ".claude", "hooks", "prefer_serena.py")
# The checkout as a `$HOME/...` path (the hook expands $HOME), wherever the checkout is.
HOME_ROOT = "$HOME/" + os.path.relpath(ROOT, os.path.expanduser("~"))


def run(tool, inp, cwd=ROOT):
    p = subprocess.run(
        [sys.executable, HOOK],
        check=False,
        capture_output=True,
        text=True,
        input=json.dumps({"tool_name": tool, "tool_input": inp, "cwd": cwd}),
        env={**os.environ, "CLAUDE_PROJECT_DIR": ROOT},
    )
    return "D" if '"deny"' in p.stdout else "A"


def B(cmd, cwd=ROOT):
    return ("Bash", {"command": cmd}, cwd)


def rust_files():
    out = []
    for d, dirs, fs in os.walk(os.path.join(ROOT, "crates")):
        dirs[:] = [x for x in dirs if x != "target"]
        out += [os.path.join(d, f) for f in fs if f.endswith(".rs")]
    return out


def line_count(path):
    with open(path, "rb") as f:
        return sum(1 for _ in f)


def main():
    size = {f: line_count(f) for f in rust_files()}
    big, small = max(size, key=size.get), min(size, key=size.get)
    cases = [
        # interpreters / heredocs on code
        (
            "D",
            *B(
                "python3 - <<'EOF'\np='crates/rustern-client/src/blocks.rs'; t=open(p).read()\nprint(t[:10])\nEOF"
            ),
        ),
        ("D", *B("python3 -c \"print(open('crates/rustern/src/main.rs').read())\"")),
        ("D", *B("python3 -c \"import os; [print(r) for r,_,_ in os.walk('crates')]\"")),
        ("D", *B("python3 -c \"import glob; print(glob.glob('crates/**/*.rs', recursive=True))\"")),
        ("D", *B("python3 -c \"m='cli'; print(open(f'crates/rustern/src/{m}.rs').read())\"")),
        (
            "D",
            *B(
                "python3 - <<'EOF'\n# it's a code edit\np='crates/rustern/src/cli.rs'; t=open(p).read()\nEOF"
            ),
        ),
        (
            "D",
            *B(
                "python3 - <<'EOF'\nfor p in ['README.md', 'crates/rustern-client/src/config.rs']:\n    s=open(p).read()\nEOF"
            ),
        ),
        ("D", *B("cd ternfs/go && python3 -c \"print(open('client/client.go').read())\"")),
        ("D", *B("cd tools && python3 - <<'EOF'\np='goldens/main.go'; s=open(p).read()\nEOF")),
        ("D", *B("perl -pi -e 's/a/b/' crates/rustern/src/main.rs")),
        ("A", *B("python3 -c \"print(open('specs/001-fuse-client-poc/tasks.md').read()[:100])\"")),
        (
            "A",
            *B(
                "python3 - <<'EOF'\np='specs/001-fuse-client-poc/tasks.md'; t=open(p).read()\n"
                "t=t.replace('- [ ] T097 in crates/rustern/tests/faults.rs','- [X] T097 in crates/rustern/tests/faults.rs')\nEOF"
            ),
        ),
        (
            "A",
            *B(
                "python3 - <<'EOF'\n# it's a doc edit\np='docs/deviations.md'; t=open(p).read()\n"
                "t=t.replace('(path: TBD)','(see crates/rustern/tests/browse.rs)')\nEOF"
            ),
        ),
        (
            "A",
            *B(
                "python3 - <<'EOF'\np='NOTICE'; s=open(p).read()\nif 'tools/rsgen/lists.go' not in s:\n"
                "    s=s.replace('tools/rsgen/main.go','tools/rsgen/main.go\\n- tools/rsgen/lists.go',1)\nEOF"
            ),
        ),
        ("A", *B('cargo test 2>&1 | python3 -c "import sys; print(sys.stdin.read()[-100:])"')),
        ("A", *B("python3 -I /tmp/x/stats.py ~/.claude/projects/a.jsonl")),
        ("A", *B("python3 -c \"s='abc'; print(s.rstrip('c'))\"")),
        ("A", *B("d=~/.cargo/registry/src/x; python3 -c \"print(open('$d/lib.rs').read())\"")),
        # shells, git
        ("D", *B('bash -c "grep -rn foo crates"')),
        ("A", *B("bash -c 'ls specs'")),
        ("D", *B("git grep -n foo")),
        ("D", *B("git -C ternfs/go grep FetchBlock")),
        ("A", *B("git grep -n foo -- '*.md'")),
        ("A", *B("git diff crates/ && git status")),
        # text tools
        ("D", *B("cat crates/rustern/src/main.rs")),
        ("D", *B(f"grep -n foo {HOME_ROOT}/crates/rustern/src/main.rs")),
        ("D", *B("grep -rn foo crates/*/src")),
        ("D", *B("grep -rn foo . --include=*.rs")),
        ("D", *B("grep -rn foo --include=*.go")),
        ("D", *B("find . -name '*.rs'")),
        ("A", *B("grep -n x specs/001-fuse-client-poc/spec.md")),
        ("A", *B("cat Cargo.toml && cargo build 2>&1 | grep -c warning")),
        ("A", *B("grep -n crc32c crates/*/Cargo.toml Cargo.toml")),
        ("A", *B("go doc ./msgs IpPort", os.path.join(ROOT, "ternfs", "go"))),
        # outside the repo (~/.cargo/registry, unresolved $vars)
        (
            "A",
            *B(
                "d=$(ls -d ~/.cargo/registry/src/*/fuser-0.18*/ | tail -1); grep -n MountOption -A40 $d/src/mnt/mount_options.rs | head"
            ),
        ),
        ("A", *B("d=~/.cargo/registry/src/x/fuser-0.18.0/src; sed -n 1,80p $d/open_flags.rs")),
        (
            "A",
            *B(
                "d=$(ls -d ~/.cargo/registry/src/*/fuser-0.18* | tail -1); grep -rn max_pages $d/src --include=*.rs | head"
            ),
        ),
        ("A", *B("grep -rn max_pages ~/.cargo/registry/src/x/fuser-0.18.0/src --include=*.rs")),
        ("A", *B("find ~/.cargo/registry/src -name '*.rs' -path '*fuser*'")),
        # Grep tool
        ("D", "Grep", {"pattern": "foo", "glob": "*.rs"}, ROOT),
        (
            "A",
            "Grep",
            {"pattern": "foo", "glob": "*.rs", "path": os.path.expanduser("~/.cargo/registry/src")},
            ROOT,
        ),
        ("A", "Grep", {"pattern": "foo", "path": os.path.join(ROOT, "specs")}, ROOT),
        # Read tool
        ("D", "Read", {"file_path": big}, ROOT),
        ("D", "Read", {"file_path": big, "limit": 500}, ROOT),
        ("A", "Read", {"file_path": big, "offset": 20, "limit": 120}, ROOT),
        ("A", "Read", {"file_path": small}, ROOT),
        ("A", "Read", {"file_path": os.path.join(ROOT, "specs/001-fuse-client-poc/spec.md")}, ROOT),
        ("A", "Read", {"file_path": os.path.join(ROOT, "ternfs/go/vendor/x.go")}, ROOT),
    ]
    bad = 0
    for exp, tool, inp, cwd in cases:
        got = run(tool, inp, cwd)
        if got != exp:
            bad += 1
            what = (inp.get("command") or inp.get("file_path") or inp.get("pattern", "")).replace(
                "\n", "⏎"
            )[:110]
            print(f"FAIL expected {exp} got {got}: {tool} {what} {inp.get('limit', '')}")
    print(f"{len(cases) - bad}/{len(cases)} passed")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
