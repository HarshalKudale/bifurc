#!/usr/bin/env python3
"""Collects the license notices of every third-party crate Bifurc ships and writes the file the app embeds.

    python packaging/licenses/generate.py

It reads `cargo metadata` for both workspaces (the engine here, the desktop app in `packages/ui-rust`), follows normal
dependencies only (build tools and test crates are not shipped), and for each crate takes its LICENSE/COPYING/
NOTICE files from the cargo registry. A crate that publishes no license file gets the standard text of its
license, with its authors as the copyright holder. Texts are grouped by license and de-duplicated, so a notice
shared by several crates is stored once with all their names.

Output: packages/ui-rust/crates/app/licenses/third-party.json (embedded by the Settings > Licenses page).
Re-run after changing dependencies. Cargo needs the registry sources, so run `cargo fetch` first on a fresh checkout.
"""
import hashlib
import json
import os
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / "packages/ui-rust/crates/app/licenses/third-party.json"
THEMES = ROOT / "packages/ui-rust/crates/app/themes"

NAMES = {
    "MIT": "MIT License", "Apache-2.0": "Apache License 2.0", "BSD-3-Clause": "BSD 3-Clause License",
    "BSD-2-Clause": "BSD 2-Clause License", "ISC": "ISC License", "Zlib": "zlib License",
    "Unicode-3.0": "Unicode License v3", "MPL-2.0": "Mozilla Public License 2.0", "CC0-1.0": "CC0 1.0 Universal",
    "0BSD": "BSD Zero Clause License", "Other": "Other licenses",
}
ORDER = ["MIT", "Apache-2.0", "BSD-3-Clause", "BSD-2-Clause", "ISC", "Zlib", "Unicode-3.0", "0BSD", "CC0-1.0", "MPL-2.0", "Other"]

MIT_BODY = (
    "Permission is hereby granted, free of charge, to any person obtaining a copy of this software and "
    "associated documentation files (the \"Software\"), to deal in the Software without restriction, including "
    "without limitation the rights to use, copy, modify, merge, publish, distribute, sublicense, and/or sell "
    "copies of the Software, and to permit persons to whom the Software is furnished to do so, subject to the "
    "following conditions:\n\nThe above copyright notice and this permission notice shall be included in all "
    "copies or substantial portions of the Software.\n\nTHE SOFTWARE IS PROVIDED \"AS IS\", WITHOUT WARRANTY OF "
    "ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR "
    "A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR "
    "ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, "
    "OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE."
)
BSD3_BODY = (
    "Redistribution and use in source and binary forms, with or without modification, are permitted provided "
    "that the following conditions are met:\n\n1. Redistributions of source code must retain the above copyright "
    "notice, this list of conditions and the following disclaimer.\n\n2. Redistributions in binary form must "
    "reproduce the above copyright notice, this list of conditions and the following disclaimer in the "
    "documentation and/or other materials provided with the distribution.\n\n3. Neither the name of the copyright "
    "holder nor the names of its contributors may be used to endorse or promote products derived from this "
    "software without specific prior written permission.\n\nTHIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND "
    "CONTRIBUTORS \"AS IS\" AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED "
    "WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE "
    "COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR "
    "CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF "
    "USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN "
    "CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF "
    "THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE."
)

NOTES = {
    "MPL-2.0": "These libraries are used unmodified. Their source code is available from crates.io and the "
               "repositories each one names.",
    "Other": "Libraries whose license is a combination of terms, or a license of its own.",
}

FILE = re.compile(r"^(licen[sc]e|copying|unlicense|notice|copyright)", re.I)


def metadata(manifest):
    out = subprocess.check_output(
        ["cargo", "metadata", "--format-version", "1", "--manifest-path", str(manifest)],
        text=True, encoding="utf-8", cwd=ROOT)
    return json.loads(out)


def shipped(manifest):
    m = metadata(manifest)
    pk = {p["id"]: p for p in m["packages"]}
    nodes = {n["id"]: n for n in m["resolve"]["nodes"]}
    roots = set(m["workspace_members"])
    seen, stack = set(), list(roots)
    while stack:
        i = stack.pop()
        if i in seen:
            continue
        seen.add(i)
        stack += [d["pkg"] for d in nodes[i]["deps"] if any(k["kind"] is None for k in d["dep_kinds"])]
    return [pk[i] for i in seen if i not in roots]


def classify(text):
    t = text.lower()
    if "bzip2" in t and "julian r seward" in t:
        return "Other"
    if "apache license" in t and "version 2.0" in t:
        return "Apache-2.0"
    if "mozilla public license" in t:
        return "MPL-2.0"
    if "unicode" in t and "permission is hereby granted" in t:
        return "Unicode-3.0"
    if "altered source versions" in t:
        return "Zlib"
    if "redistribution and use in source and binary forms" in t:
        return "BSD-3-Clause" if "neither the name" in t or "endorse" in t else "BSD-2-Clause"
    if "permission to use, copy, modify, and/or distribute" in t or "permission to use, copy, modify, and distribute" in t:
        return "ISC"
    if "cc0" in t or "creative commons" in t:
        return "CC0-1.0"
    if "permission is hereby granted, free of charge" in t:
        return "MIT"
    return "Other"


def options(expr):
    """The licenses a crate offers; empty for a combined (AND) or missing expression."""
    if not expr or " AND " in expr:
        return []
    ids = [x.strip(" ()") for x in re.split(r"\s+OR\s+|/", expr)]
    return [re.sub(r"\s+WITH.*", "", i) for i in ids]


def license_files(crate_dir):
    found = []
    for root, dirs, files in os.walk(crate_dir):
        depth = Path(root).relative_to(crate_dir).parts
        dirs[:] = [] if len(depth) >= 1 else [d for d in dirs if d not in ("src", "tests", "benches", "examples", "target")]
        for f in files:
            if FILE.match(f) and not re.search(r"gpl", f, re.I) and not f.lower().endswith((".rs", ".toml", ".json")):
                found.append(Path(root) / f)
    return sorted(found)


def authors_of(p):
    names = [re.sub(r"\s*<.*?>", "", a).strip() for a in p.get("authors") or []]
    return ", ".join(n for n in names if n) or f"The {p['name']} authors"


def standard_text(kind, who, library):
    if kind == "MIT":
        return f"MIT License\n\nCopyright (c) {who}\n\n{MIT_BODY}"
    if kind == "BSD-3-Clause":
        return f"Copyright (c) {who}. All rights reserved.\n\n{BSD3_BODY}"
    return library.get(kind)


def main():
    crates = {}
    for manifest in ("Cargo.toml", "packages/ui-rust/Cargo.toml"):
        for p in shipped(ROOT / manifest):
            if not p["name"].startswith("bifurc-"):
                crates.setdefault((p["name"], p["version"]), p)

    per_crate = {}
    library = {}  # whole texts with no copyright line of their own, reusable for crates that ship none
    for key, p in crates.items():
        texts = []
        for f in license_files(Path(p["manifest_path"]).parent):
            text = f.read_text(encoding="utf-8", errors="replace").strip()
            if len(text) > 100:  # shorter is a pointer to a file outside the published crate, not a license
                texts.append(text)
                kind = classify(text)
                if kind in ("Apache-2.0", "MPL-2.0", "CC0-1.0"):
                    library.setdefault(kind, text)
        per_crate[key] = texts

    groups, standard = {}, []
    for (name, version), p in sorted(crates.items()):
        opts = options(p["license"])
        texts = per_crate[(name, version)]
        kept = [t for t in texts if classify(t) in opts] if opts else texts
        if opts and kept:
            # Dual-licensed: keep the one permissive option the crate offers first, not every file it ships.
            best = min(kept, key=lambda t: ORDER.index(classify(t)))
            kept = [best]
        kept = kept or texts
        if not kept:
            elected = next((o for o in ORDER if o in opts), None)
            body = standard_text(elected, authors_of(p), library) if elected else None
            if body is None:
                body = (f"Licensed under {p['license'] or 'an unspecified license'}. "
                        f"See {p.get('repository') or 'the project repository'}.")
            kept = [body]
            standard.append(f"{name} {version}")
        for text in kept:
            g = groups.setdefault(classify(text), {})
            g.setdefault(hashlib.sha1(text.encode()).hexdigest(), {"text": text, "crates": []})["crates"].append(f"{name} {version}")

    out_groups = []
    for kind in ORDER:
        notices = list(groups.get(kind, {}).values())
        if notices:
            out_groups.append({
                "id": kind, "name": NAMES[kind], "note": NOTES.get(kind, ""),
                "libraries": sum(len(n["crates"]) for n in notices),
                "notices": sorted(notices, key=lambda n: n["crates"][0]),
            })

    authors = sorted({json.loads(f.read_text(encoding="utf-8")).get("author", "") for f in THEMES.glob("*.json")} - {""})
    data = {
        "libraries": len(crates),
        "assets": [
            {"name": "Lucide icons", "license": "ISC",
             "detail": "The interface icons, shipped by gpui-kit-assets. Their notice is listed under ISC License."},
            {"name": "Interface themes", "license": "Apache-2.0",
             "detail": "Colour themes published with GPUI Kit, copied unchanged. Authors: " + ", ".join(authors) + "."},
            {"name": "SQLite", "license": "Public domain",
             "detail": "Bundled by libsqlite3-sys. SQLite itself is in the public domain."},
        ],
        "groups": out_groups,
    }
    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text(json.dumps(data, ensure_ascii=False, separators=(",", ":")), encoding="utf-8")
    print(f"{len(crates)} libraries, {sum(len(g['notices']) for g in out_groups)} notices, {OUT.stat().st_size // 1024} KiB")
    print(f"standard text used (no license file published): {len(standard)}")
    for g in out_groups:
        print(f"  {g['id']}: {g['libraries']} libraries, {len(g['notices'])} notices")


if __name__ == "__main__":
    sys.exit(main())
