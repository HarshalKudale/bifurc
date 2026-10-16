//! `cargo xtask licenses [--check]`: collects the license notices of every third-party crate Bifurc ships and
//! writes (or, with `--check`, verifies) `crates/app/licenses/third-party.json`, the file the Settings > Licenses
//! page embeds.
//!
//! It reads `cargo metadata` for the workspace, follows normal dependencies only (build tools and test crates
//! are not shipped), and for each crate takes its LICENSE/COPYING/NOTICE files from the cargo registry. A crate
//! that publishes no license file gets the standard text of its license, with its authors as the copyright
//! holder. Texts are grouped by license and de-duplicated, so a notice shared by several crates is stored once
//! with all their names. Cargo needs the registry sources, so run `cargo fetch` first on a fresh checkout.

use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

use serde_json::{Value, json};

use crate::Result;

const NAMES: [(&str, &str); 11] = [
    ("MIT", "MIT License"),
    ("Apache-2.0", "Apache License 2.0"),
    ("BSD-3-Clause", "BSD 3-Clause License"),
    ("BSD-2-Clause", "BSD 2-Clause License"),
    ("ISC", "ISC License"),
    ("Zlib", "zlib License"),
    ("Unicode-3.0", "Unicode License v3"),
    ("MPL-2.0", "Mozilla Public License 2.0"),
    ("CC0-1.0", "CC0 1.0 Universal"),
    ("0BSD", "BSD Zero Clause License"),
    ("Other", "Other licenses"),
];
const ORDER: [&str; 11] = [
    "MIT",
    "Apache-2.0",
    "BSD-3-Clause",
    "BSD-2-Clause",
    "ISC",
    "Zlib",
    "Unicode-3.0",
    "0BSD",
    "CC0-1.0",
    "MPL-2.0",
    "Other",
];

const MIT_BODY: &str = "Permission is hereby granted, free of charge, to any person obtaining a copy of this software and associated documentation files (the \"Software\"), to deal in the Software without restriction, including without limitation the rights to use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of the Software, and to permit persons to whom the Software is furnished to do so, subject to the following conditions:\n\nThe above copyright notice and this permission notice shall be included in all copies or substantial portions of the Software.\n\nTHE SOFTWARE IS PROVIDED \"AS IS\", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.";
const BSD3_BODY: &str = "Redistribution and use in source and binary forms, with or without modification, are permitted provided that the following conditions are met:\n\n1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following disclaimer.\n\n2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the following disclaimer in the documentation and/or other materials provided with the distribution.\n\n3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote products derived from this software without specific prior written permission.\n\nTHIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS \"AS IS\" AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.";

fn note(kind: &str) -> &'static str {
    match kind {
        "MPL-2.0" => {
            "These libraries are used unmodified. Their source code is available from crates.io and the repositories each one names."
        }
        "Other" => "Libraries whose license is a combination of terms, or a license of its own.",
        _ => "",
    }
}

fn classify(text: &str) -> &'static str {
    let t = text.to_lowercase();
    let has = |s: &str| t.contains(s);
    if has("bzip2") && has("julian r seward") {
        "Other"
    } else if has("apache license") && has("version 2.0") {
        "Apache-2.0"
    } else if has("mozilla public license") {
        "MPL-2.0"
    } else if has("unicode") && has("permission is hereby granted") {
        "Unicode-3.0"
    } else if has("altered source versions") {
        "Zlib"
    } else if has("redistribution and use in source and binary forms") {
        if has("neither the name") || has("endorse") { "BSD-3-Clause" } else { "BSD-2-Clause" }
    } else if has("permission to use, copy, modify, and/or distribute")
        || has("permission to use, copy, modify, and distribute")
    {
        "ISC"
    } else if has("cc0") || has("creative commons") {
        "CC0-1.0"
    } else if has("permission is hereby granted, free of charge") {
        "MIT"
    } else {
        "Other"
    }
}

fn rank(kind: &str) -> usize {
    ORDER.iter().position(|k| *k == kind).unwrap_or(usize::MAX)
}

/// The licenses a crate offers; empty for a combined (AND) or missing expression.
fn options(expr: &str) -> Vec<String> {
    if expr.is_empty() || expr.contains(" AND ") {
        return Vec::new();
    }
    let mut parts: Vec<Vec<&str>> = vec![Vec::new()];
    let expr = expr.replace('/', " OR ");
    for token in expr.split_whitespace() {
        if token == "OR" {
            parts.push(Vec::new());
        } else {
            parts.last_mut().unwrap().push(token);
        }
    }
    parts
        .into_iter()
        .map(|p| {
            let id = p.join(" ");
            let id = id.trim_matches([' ', '(', ')']);
            id.split(" WITH").next().unwrap_or(id).to_owned()
        })
        .collect()
}

fn is_license_file(name: &str) -> bool {
    let n = name.to_lowercase();
    ["license", "licence", "copying", "unlicense", "notice", "copyright"].iter().any(|p| n.starts_with(p))
        && !n.contains("gpl")
        && ![".rs", ".toml", ".json"].iter().any(|e| n.ends_with(e))
}

/// License files in the crate root and one directory down (not `src`, `tests`, `benches`, `examples`, `target`).
fn license_files(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut scan = |d: &Path| {
        for entry in fs::read_dir(d).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_file() && is_license_file(&entry.file_name().to_string_lossy()) {
                found.push(path);
            }
        }
    };
    scan(dir);
    for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if entry.path().is_dir() && !["src", "tests", "benches", "examples", "target"].contains(&name.as_str()) {
            scan(&entry.path());
        }
    }
    // Case-insensitive, so the order does not depend on the platform's file system.
    found.sort_by_key(|p| (p.to_string_lossy().to_lowercase(), p.clone()));
    found
}

fn read_text(path: &Path) -> String {
    let bytes = fs::read(path).unwrap_or_default();
    String::from_utf8_lossy(&bytes).replace("\r\n", "\n").replace('\r', "\n").trim().to_owned()
}

fn authors_of(p: &Value) -> String {
    let names: Vec<String> = p["authors"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(|a| match (a.find('<'), a.find('>')) {
            (Some(l), Some(r)) if l < r => format!("{}{}", a[..l].trim_end(), &a[r + 1..]).trim().to_owned(),
            _ => a.trim().to_owned(),
        })
        .filter(|n| !n.is_empty())
        .collect();
    if names.is_empty() { format!("The {} authors", p["name"].as_str().unwrap_or_default()) } else { names.join(", ") }
}

/// Every non-workspace package reachable from a workspace member through normal dependencies.
fn shipped(repo: &Path) -> Result<BTreeMap<(String, String), Value>> {
    let out = Command::new(env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .args(["metadata", "--format-version", "1", "--manifest-path"])
        .arg(repo.join("Cargo.toml"))
        .current_dir(repo)
        .output()
        .map_err(|e| format!("cargo metadata: {e}"))?;
    if !out.status.success() {
        return Err(format!("cargo metadata failed:\n{}", String::from_utf8_lossy(&out.stderr)));
    }
    let m: Value = serde_json::from_slice(&out.stdout).map_err(|e| format!("cargo metadata output: {e}"))?;
    let id_of = |v: &Value| v["id"].as_str().unwrap_or_default().to_owned();
    let packages: BTreeMap<String, &Value> =
        m["packages"].as_array().into_iter().flatten().map(|p| (id_of(p), p)).collect();
    let nodes: BTreeMap<String, &Value> =
        m["resolve"]["nodes"].as_array().into_iter().flatten().map(|n| (id_of(n), n)).collect();
    let roots: BTreeSet<String> =
        m["workspace_members"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_owned).collect();
    let (mut seen, mut stack) = (BTreeSet::new(), roots.iter().cloned().collect::<Vec<_>>());
    while let Some(id) = stack.pop() {
        if !seen.insert(id.clone()) {
            continue;
        }
        for dep in nodes.get(&id).and_then(|n| n["deps"].as_array()).into_iter().flatten() {
            if dep["dep_kinds"].as_array().into_iter().flatten().any(|k| k["kind"].is_null()) {
                stack.push(dep["pkg"].as_str().unwrap_or_default().to_owned());
            }
        }
    }
    let mut crates = BTreeMap::new();
    for id in seen.difference(&roots) {
        let p = packages.get(id).ok_or(format!("package {id} missing from metadata"))?;
        let name = p["name"].as_str().unwrap_or_default();
        if !name.starts_with("bifurc-") {
            crates
                .entry((name.to_owned(), p["version"].as_str().unwrap_or_default().to_owned()))
                .or_insert_with(|| (*p).clone());
        }
    }
    Ok(crates)
}

/// The exact bytes of `third-party.json`.
fn generate(repo: &Path) -> Result<String> {
    let crates = shipped(repo)?;

    let mut per_crate: BTreeMap<&(String, String), Vec<String>> = BTreeMap::new();
    let mut library: BTreeMap<&str, String> = BTreeMap::new(); // whole texts reusable for crates that ship none
    for (key, p) in &crates {
        let dir = Path::new(p["manifest_path"].as_str().unwrap_or_default())
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default();
        let mut texts = Vec::new();
        for f in license_files(&dir) {
            let text = read_text(&f);
            if text.chars().count() > 100 {
                // shorter is a pointer to a file outside the published crate, not a license
                let kind = classify(&text);
                if ["Apache-2.0", "MPL-2.0", "CC0-1.0"].contains(&kind) {
                    library.entry(kind).or_insert_with(|| text.clone());
                }
                texts.push(text);
            }
        }
        per_crate.insert(key, texts);
    }

    // kind -> notices in first-seen order: (text, crates)
    let mut groups: BTreeMap<&str, Vec<(String, Vec<String>)>> = BTreeMap::new();
    for (key, p) in &crates {
        let (name, version) = key;
        let license = p["license"].as_str().unwrap_or_default();
        let opts = options(license);
        let texts = &per_crate[key];
        let mut kept: Vec<String> = if opts.is_empty() {
            texts.clone()
        } else {
            texts.iter().filter(|t| opts.iter().any(|o| o == classify(t))).cloned().collect()
        };
        if !opts.is_empty() && !kept.is_empty() {
            // Dual-licensed: keep the one permissive option the crate offers first, not every file it ships.
            let best = kept.iter().min_by_key(|t| rank(classify(t))).unwrap().clone();
            kept = vec![best];
        }
        if kept.is_empty() {
            kept = texts.clone();
        }
        if kept.is_empty() {
            let who = authors_of(p);
            let body = ORDER.iter().find(|o| opts.iter().any(|x| x == *o)).and_then(|k| match *k {
                "MIT" => Some(format!("MIT License\n\nCopyright (c) {who}\n\n{MIT_BODY}")),
                "BSD-3-Clause" => Some(format!("Copyright (c) {who}. All rights reserved.\n\n{BSD3_BODY}")),
                other => library.get(other).cloned(),
            });
            kept = vec![body.unwrap_or_else(|| {
                format!(
                    "Licensed under {}. See {}.",
                    if license.is_empty() { "an unspecified license" } else { license },
                    p["repository"].as_str().filter(|r| !r.is_empty()).unwrap_or("the project repository")
                )
            })];
        }
        for text in kept {
            let g = groups.entry(classify(&text)).or_default();
            let label = format!("{name} {version}");
            match g.iter_mut().find(|(t, _)| *t == text) {
                Some((_, names)) => names.push(label),
                None => g.push((text, vec![label])),
            }
        }
    }

    let mut out_groups = Vec::new();
    for kind in ORDER {
        let Some(mut notices) = groups.remove(kind) else { continue };
        notices.sort_by(|a, b| a.1[0].cmp(&b.1[0])); // stable
        out_groups.push(json!({
            "id": kind,
            "name": NAMES.iter().find(|(k, _)| *k == kind).map_or("", |(_, n)| n),
            "note": note(kind),
            "libraries": notices.iter().map(|n| n.1.len()).sum::<usize>(),
            "notices": notices.into_iter().map(|(text, crates)| json!({"text": text, "crates": crates})).collect::<Vec<_>>(),
        }));
    }

    let mut authors = BTreeSet::new();
    let themes = repo.join("crates/app/themes");
    for entry in fs::read_dir(&themes).map_err(|e| format!("{}: {e}", themes.display()))?.flatten() {
        if entry.path().extension().is_some_and(|e| e == "json") {
            let v: Value = serde_json::from_str(&fs::read_to_string(entry.path()).map_err(|e| e.to_string())?)
                .map_err(|e| format!("{}: {e}", entry.path().display()))?;
            if let Some(a) = v["author"].as_str().filter(|a| !a.is_empty()) {
                authors.insert(a.to_owned());
            }
        }
    }
    let authors = authors.into_iter().collect::<Vec<_>>().join(", ");
    let data = json!({
        "libraries": crates.len(),
        "assets": [
            {"name": "Lucide icons", "license": "ISC",
             "detail": "The interface icons, shipped by gpui-kit-assets. Their notice is listed under ISC License."},
            {"name": "Interface themes", "license": "Apache-2.0",
             "detail": format!("Colour themes published with GPUI Kit, copied unchanged. Authors: {authors}.")},
            {"name": "SQLite", "license": "Public domain",
             "detail": "Bundled by libsqlite3-sys. SQLite itself is in the public domain."},
        ],
        "groups": out_groups,
    });
    serde_json::to_string(&data).map_err(|e| e.to_string())
}

pub fn run(repo: &Path, check: bool) -> Result<()> {
    let out = repo.join("crates/app/licenses/third-party.json");
    let fresh = generate(repo)?;
    if check {
        let old = fs::read_to_string(&out).unwrap_or_default();
        if old == fresh {
            println!("third-party.json is up to date");
            return Ok(());
        }
        let at = old.chars().zip(fresh.chars()).take_while(|(a, b)| a == b).count();
        let snippet = |s: &str| s.chars().skip(at.saturating_sub(40)).take(120).collect::<String>();
        let libs = |s: &str| serde_json::from_str::<Value>(s).ok().and_then(|v| v["libraries"].as_u64()).unwrap_or(0);
        return Err(format!(
            "{} is stale (committed: {} bytes, {} libraries; current dependencies: {} bytes, {} libraries).\n  first difference at char {at}:\n    committed: {:?}\n    current:   {:?}\n  run `cargo xtask licenses` and commit the result",
            out.display(),
            old.len(),
            libs(&old),
            fresh.len(),
            libs(&fresh),
            snippet(&old),
            snippet(&fresh)
        ));
    }
    fs::create_dir_all(out.parent().unwrap()).map_err(|e| e.to_string())?;
    fs::write(&out, &fresh).map_err(|e| format!("{}: {e}", out.display()))?;
    println!("wrote {} ({} KiB)", out.display(), fresh.len() / 1024);
    Ok(())
}
