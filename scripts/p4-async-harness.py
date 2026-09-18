"""Make the conformance harness async so a runner can authenticate before each case.

P4 work item 4 makes `handshake`/`auth` verifiable, which means a runner can now supply a transport
that requires a `hello` before any request. `harness()` has to perform that handshake, and performing
it is async — so `harness()` becomes async and its call sites await it.

The rewrite is mechanical and the counts are asserted, because a partial rewrite of the programme's
most valuable test asset is worse than no rewrite: a missed call site would leave a test asserting
against a Promise, and every one of those still "passes" as a truthy object.

`npm run typecheck` is the backstop for the `it(...)` callbacks: `await` inside a non-async arrow
function is a compile error, so anything this script misses the compiler names.
"""
import io
import re

PATH = "packages/engine/tests/conformance/protocol.conformance.ts"

src = io.open(PATH, encoding="utf-8").read()
lines = src.split("\n")

# ── 1. await every call site ─────────────────────────────────────────────────
call_sites = sum(1 for l in lines if "= harness(" in l)
if call_sites == 0:
    raise SystemExit("no call sites found — has the harness been renamed?")
lines = [l.replace("= harness(", "= await harness(") for l in lines]
print("awaited %d call sites" % call_sites)

# ── 2. mark the enclosing `it(...)` async where it now awaits ────────────────
# Brace-depth scan rather than a regex: an `it` body contains object literals, so the closing line is
# the one where depth returns to zero, not the next `});` in the file.
marked = 0
i = 0
while i < len(lines):
    line = lines[i]
    if re.match(r"^\s*it\(", line) and "async" not in line:
        depth = 0
        started = False
        j = i
        while j < len(lines):
            depth += lines[j].count("{") - lines[j].count("}")
            if "{" in lines[j]:
                started = True
            if started and depth <= 0:
                break
            j += 1
        block = "\n".join(lines[i : j + 1])
        if "await harness(" in block:
            lines[i] = line.replace("() =>", "async () =>", 1)
            if "async" not in lines[i]:
                raise SystemExit("could not mark async at line %d: %s" % (i + 1, line))
            marked += 1
        i = j + 1
    else:
        i += 1

print("marked %d it() callbacks async" % marked)
if marked != 5:
    raise SystemExit("expected 5 sync callbacks to need marking, got %d — review before writing" % marked)

io.open(PATH, "w", encoding="utf-8", newline="\n").write("\n".join(lines))
print("wrote %s" % PATH)
