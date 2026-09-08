//! Regression test for a build script that rebuilt itself, and everything
//! depending on it, on every single build even when nothing had changed.
//!
//! `build.rs` runs `shadow_rs::ShadowBuilder::builder().build()`, which
//! rewrites `shadow.rs` in `OUT_DIR` on every invocation because it embeds
//! the exact time it ran. Cargo's staleness check for files pulled in via
//! `include!` (which the `shadow!` macro in `core_commands::version` uses)
//! is based on mtime, not content, so nu-cmd-lang, and every crate
//! depending on it, was rebuilt on every build regardless of whether
//! anything meaningful had changed.
//!
//! This can't be expressed as a narrow unit test: the bug is about how
//! Cargo fingerprints a real build. It also only shows up under a release
//! build, since shadow-rs's default `Lazy` build pattern skips rerunning
//! the build script at all for debug builds. The smallest thing that
//! actually exercises it is building the crate twice in a row and checking
//! whether the second build considers it dirty.

use std::process::Command;

fn build_nu_cmd_lang_release_verbose() -> String {
    let output = Command::new(env!("CARGO"))
        .args([
            "build",
            "--release",
            "--verbose",
            "--package",
            "nu-cmd-lang",
        ])
        .output()
        .expect("failed to spawn `cargo build`");
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        output.status.success(),
        "cargo build --release -p nu-cmd-lang failed:\n{stderr}"
    );
    stderr
}

#[test]
fn shadow_rs_output_does_not_force_a_rebuild() {
    // Make sure nu-cmd-lang is fully built before the real check, so the
    // second build below is the only one that can report it as dirty.
    build_nu_cmd_lang_release_verbose();

    let second_build = build_nu_cmd_lang_release_verbose();
    let dirty_nu_cmd_lang = second_build
        .lines()
        .find(|line| line.contains("Dirty") && line.contains("nu-cmd-lang"));

    assert!(
        dirty_nu_cmd_lang.is_none(),
        "a second `cargo build --release -p nu-cmd-lang` with nothing changed \
         reported the crate as dirty, meaning its build script's output is \
         not stable across runs:\n{}",
        dirty_nu_cmd_lang.unwrap_or_default()
    );
}
