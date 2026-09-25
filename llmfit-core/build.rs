//! Aggregate community benchmark submissions (data/community/<slug>/*.json)
//! and hardware profiles (data/hardware/*.json) into single JSON arrays
//! embedded in the binary. This is what closes the contribution loop: a
//! submission merged into the repo ships to every user in the next release,
//! with no CI step or network fetch involved.

use std::env;
use std::fs;
use std::path::{Component, Path, PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=data/community");
    println!("cargo:rerun-if-changed=data/hardware");

    emit_git_build_sha();
    embed_community_benchmarks();
    embed_hardware_profiles();
}

/// Record a short git revision for local benchmark provenance (`tool.build`).
///
/// Git worktrees point `.git` at a `gitdir:` file; we resolve that directory for
/// `rerun-if-changed` paths. Tarball or crates.io builds without a `.git` tree
/// omit both rerun hooks and `LLMFIT_GIT_SHA`.
fn emit_git_build_sha() {
    println!("cargo:rerun-if-env-changed=LLMFIT_GIT_SHA");
    if let Ok(sha) = env::var("LLMFIT_GIT_SHA") {
        let sha = sha.trim();
        if !sha.is_empty() {
            println!("cargo:rustc-env=LLMFIT_GIT_SHA={}", sha);
        }
        return;
    }

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir.join("..");
    if let Some(git_dir) = resolve_git_dir(&workspace_root) {
        register_git_rerun_paths(&manifest_dir, &git_dir);
        if let Some(sha) = git_revision(&workspace_root) {
            println!("cargo:rustc-env=LLMFIT_GIT_SHA={}", sha);
        }
    }
}

fn resolve_git_dir(workspace_root: &Path) -> Option<PathBuf> {
    let dot_git = workspace_root.join(".git");
    if dot_git.is_dir() {
        return Some(dot_git);
    }
    if dot_git.is_file() {
        let content = fs::read_to_string(dot_git).ok()?;
        for line in content.lines() {
            if let Some(gitdir) = line.strip_prefix("gitdir: ") {
                return Some(PathBuf::from(gitdir.trim()));
            }
        }
    }
    None
}

fn register_git_rerun_paths(manifest_dir: &Path, git_dir: &Path) {
    rerun_if_changed_relative(manifest_dir, &git_dir.join("HEAD"));
    rerun_if_changed_relative(manifest_dir, &git_dir.join("packed-refs"));
    if let Ok(head) = fs::read_to_string(git_dir.join("HEAD")) {
        let head = head.trim();
        if let Some(ref_name) = head.strip_prefix("ref: ") {
            rerun_if_changed_relative(manifest_dir, &git_dir.join(ref_name.trim()));
        }
    }
}

fn rerun_if_changed_relative(manifest_dir: &Path, path: &Path) {
    if !path.exists() {
        return;
    }
    if let Some(rel) = path_relative_to_manifest(manifest_dir, path) {
        println!("cargo:rerun-if-changed={}", rel);
    }
}

fn path_relative_to_manifest(manifest_dir: &Path, path: &Path) -> Option<String> {
    let base = manifest_dir.canonicalize().ok()?;
    let path = path.canonicalize().ok()?;
    let base = base
        .components()
        .filter(|c| !matches!(c, Component::CurDir))
        .collect::<Vec<_>>();
    let path = path
        .components()
        .filter(|c| !matches!(c, Component::CurDir))
        .collect::<Vec<_>>();
    let mut shared = 0;
    while shared < base.len() && shared < path.len() && base[shared] == path[shared] {
        shared += 1;
    }
    let mut rel = PathBuf::new();
    for _ in shared..base.len() {
        rel.push("..");
    }
    for component in &path[shared..] {
        rel.push(component.as_os_str());
    }
    Some(rel.to_string_lossy().replace('\\', "/"))
}

fn git_revision(workspace_root: &Path) -> Option<String> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(workspace_root)
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let sha = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!sha.is_empty()).then_some(sha)
}

fn embed_community_benchmarks() {
    let community_dir = Path::new("data/community");
    let mut files: Vec<PathBuf> = Vec::new();
    if let Ok(entries) = fs::read_dir(community_dir) {
        for entry in entries.flatten() {
            let slug_dir = entry.path();
            if !slug_dir.is_dir() {
                continue; // README.md, schema.json
            }
            if let Ok(subs) = fs::read_dir(&slug_dir) {
                for sub in subs.flatten() {
                    let p = sub.path();
                    if p.extension().and_then(|e| e.to_str()) == Some("json") {
                        files.push(p);
                    }
                }
            }
        }
    }
    // Deterministic embed order regardless of directory iteration order.
    files.sort();

    let mut payloads: Vec<serde_json::Value> = Vec::new();
    for f in &files {
        let Ok(text) = fs::read_to_string(f) else {
            println!(
                "cargo:warning=community submission unreadable, skipped: {}",
                f.display()
            );
            continue;
        };
        match serde_json::from_str::<serde_json::Value>(&text) {
            Ok(v) => payloads.push(v),
            // CI validates submissions on PR; a bad file here should never
            // happen, but a warning beats breaking every build.
            Err(e) => println!(
                "cargo:warning=community submission invalid JSON, skipped: {}: {e}",
                f.display()
            ),
        }
    }

    let out = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR set by cargo"))
        .join("community_benchmarks.json");
    let json = serde_json::to_string(&payloads).expect("serialize community aggregate");
    fs::write(&out, json).expect("write community aggregate");
}

/// Aggregate `data/hardware/*.json` into one array, skipping the schema and
/// README that live alongside the profiles.
fn embed_hardware_profiles() {
    let hardware_dir = Path::new("data/hardware");
    let mut files: Vec<PathBuf> = Vec::new();
    if let Ok(entries) = fs::read_dir(hardware_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue; // README.md
            }
            if path.file_stem().and_then(|s| s.to_str()) == Some("schema") {
                continue; // schema.json is the contract, not a profile
            }
            files.push(path);
        }
    }
    // Deterministic embed order regardless of directory iteration order.
    files.sort();

    let mut payloads: Vec<serde_json::Value> = Vec::new();
    for f in &files {
        let Ok(text) = fs::read_to_string(f) else {
            println!(
                "cargo:warning=hardware profile unreadable, skipped: {}",
                f.display()
            );
            continue;
        };
        match serde_json::from_str::<serde_json::Value>(&text) {
            Ok(v) => payloads.push(v),
            // `cargo test -p llmfit-core` validates this directory against
            // schema.json, so a bad file here should never reach a release; a
            // warning beats breaking every build.
            Err(e) => println!(
                "cargo:warning=hardware profile invalid JSON, skipped: {}: {e}",
                f.display()
            ),
        }
    }

    let out = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR set by cargo"))
        .join("hardware_profiles.json");
    let json = serde_json::to_string(&payloads).expect("serialize hardware profile aggregate");
    fs::write(&out, json).expect("write hardware profile aggregate");
}
