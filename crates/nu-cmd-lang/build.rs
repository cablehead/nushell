use std::path::Path;
use std::process::Command;

/// The name shadow-rs gives the file it writes into `OUT_DIR`. Not part of
/// its public API: if shadow-rs ever renames it, `main` simply finds no
/// previous file to compare against and every build regenerates it, same as
/// today.
const SHADOW_RS_OUTPUT: &str = "shadow.rs";

fn main() {
    // Look up the current Git commit ourselves instead of relying on shadow_rs,
    // because shadow_rs does it in a really slow-to-compile way (it builds libgit2)
    // Allow overriding it with `NU_COMMIT_HASH` from outside, such as with nix.
    let hash = get_git_hash().unwrap_or(
        option_env!("NU_COMMIT_HASH")
            .unwrap_or_default()
            .to_string(),
    );
    println!("cargo:rustc-env=NU_COMMIT_HASH={hash}");

    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR is set by cargo for build scripts");
    let shadow_path = Path::new(&out_dir).join(SHADOW_RS_OUTPUT);
    let previous = std::fs::metadata(&shadow_path)
        .ok()
        .and_then(|meta| meta.modified().ok())
        .zip(std::fs::read(&shadow_path).ok());

    shadow_rs::ShadowBuilder::builder()
        .build()
        .expect("shadow builder build should success");

    // shadow-rs embeds the exact time it ran (the "Generation time" header
    // comment, and the BUILD_TIME* constants) into `shadow.rs` on every
    // invocation, so it rewrites this file with different contents on every
    // build even when nothing else changed. Cargo's staleness check for
    // files pulled in via `include!` (which the `shadow!` macro uses) is
    // based on mtime, not content, so even writing back byte-identical
    // content would still dirty the crate. If the only thing that changed
    // since the last build is the embedded build time, put the previous
    // file (and its mtime) back so Cargo sees nu-cmd-lang, and everything
    // that depends on it, as unchanged.
    //
    // This is purely an optimization: any failure along the way just leaves
    // the freshly generated file in place, which is what happened on every
    // build before this was added.
    if let Some((previous_mtime, previous_contents)) = previous
        && let Ok(regenerated) = std::fs::read(&shadow_path)
        && strip_build_time(&previous_contents) == strip_build_time(&regenerated)
        && std::fs::write(&shadow_path, &previous_contents).is_ok()
        && let Ok(file) = std::fs::File::open(&shadow_path)
    {
        let _ = file.set_modified(previous_mtime);
    }
}

/// Drops every line touched by shadow-rs's embedded build time (the
/// "Generation time" header comment and the `BUILD_TIME*` constants), so
/// what's left can be compared to tell whether two generated files differ
/// for any reason other than when they were generated.
fn strip_build_time(contents: &[u8]) -> Vec<u8> {
    String::from_utf8_lossy(contents)
        .lines()
        .filter(|line| !line.contains("BUILD_TIME") && !line.starts_with("// Generation time:"))
        .collect::<Vec<_>>()
        .join("\n")
        .into_bytes()
}

fn get_git_hash() -> Option<String> {
    Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|hash| hash.trim().to_string())
}
