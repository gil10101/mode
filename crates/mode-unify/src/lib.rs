//! Feature unification for subset builds. This crate has no code: its
//! manifest names the crates whose features only Tauri's graph turns on, with
//! exactly the features a `cargo test --workspace` resolves for them, and
//! mode-core depends on it. Any selection that contains mode-core or mode-app
//! (`cargo test -p mode-core`, `cargo build -p mode-app -p mode-hook`, the
//! native CI jobs) therefore resolves every shared crate like the workspace
//! build does and reuses its artifacts instead of compiling a second set.
//!
//! The test below compares `cargo tree` for those selections with the
//! workspace's, per target, and prints the entry to add to `Cargo.toml` when
//! a dependency update makes them drift apart.

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::path::PathBuf;
    use std::process::Command;

    /// The targets Mode ships; checked when their packages are available
    /// offline (the host's always are).
    const TARGETS: [(&str, &str); 3] = [
        ("x86_64-unknown-linux-gnu", "cfg(target_os = \"linux\")"),
        ("aarch64-apple-darwin", "cfg(target_os = \"macos\")"),
        ("x86_64-pc-windows-msvc", "cfg(windows)"),
    ];

    /// Selections that must reuse the workspace build's dependency artifacts:
    /// a test or build of one crate, the smoke build, the native CI jobs.
    const SUBSETS: [&[&str]; 4] = [
        &["-p", "mode-core", "-e", "normal,build,dev"],
        &["-p", "mode-app", "-p", "mode-hook", "-e", "normal,build"],
        &[
            "-p",
            "mode-core",
            "-p",
            "mode-hook",
            "-p",
            "mode-wire",
            "-e",
            "normal,build,dev",
        ],
        &["--workspace", "-e", "normal,build"],
    ];

    fn workspace_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    /// `name vX.Y.Z` → every feature set it resolves with (one per build
    /// side), registry crates only. `None` when the packages of `target` are
    /// not available offline.
    fn resolved(target: &str, args: &[&str]) -> Option<BTreeMap<String, BTreeSet<String>>> {
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
        let out = Command::new(cargo)
            .args(["tree", "--locked", "--offline", "--prefix", "none"])
            .args(["--format", "{p}|{f}", "--target", target])
            .args(args)
            .current_dir(workspace_root())
            .output()
            .expect("cargo runs");
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            assert!(
                err.contains("offline") || err.contains("download"),
                "cargo tree {args:?} --target {target} failed:\n{err}"
            );
            return None;
        }
        let mut map: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for line in String::from_utf8_lossy(&out.stdout).lines() {
            let Some((pkg, feats)) = line.split_once('|') else {
                continue;
            };
            // Workspace members print their path: `mode-core v0.1.0 (/…/crates/mode-core)`.
            if pkg.contains(" (/") || pkg.contains(":\\") {
                continue;
            }
            let pkg = pkg.trim_end_matches(" (proc-macro)").to_string();
            let feats = feats.trim_end_matches(" (*)").to_string();
            map.entry(pkg).or_default().insert(feats);
        }
        Some(map)
    }

    #[test]
    fn subset_builds_resolve_like_the_workspace() {
        let mut drift = Vec::new();
        let mut checked = Vec::new();
        for (target, cfg) in TARGETS {
            let Some(reference) = resolved(target, &["--workspace", "-e", "normal,build,dev"])
            else {
                eprintln!("{target}: packages not available offline; skipped");
                continue;
            };
            checked.push(target);
            for args in SUBSETS {
                let sub = resolved(target, args).expect("the reference resolved offline");
                for (pkg, sets) in &sub {
                    let known = reference.get(pkg).cloned().unwrap_or_default();
                    for feats in sets.difference(&known) {
                        let (name, version) = pkg.split_once(" v").unwrap_or((pkg, ""));
                        drift.push(format!(
                            "{target} `cargo test|build {}`: {name} {version} resolves with [{feats}]; \
                             the workspace build has {known:?}. Add to crates/mode-unify/Cargo.toml \
                             under [target.'{cfg}'.dependencies] (or [build-dependencies] for a \
                             build-time crate): {name} = {{ version = \"{version}\", features = [...] }} \
                             with the workspace's features",
                            args.join(" "),
                        ));
                    }
                }
            }
        }
        assert!(!checked.is_empty(), "no target could be resolved");
        assert!(drift.is_empty(), "feature drift:\n{}", drift.join("\n"));
    }
}
