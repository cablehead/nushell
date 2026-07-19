use fancy_regex::Regex;
use nu_protocol::{PipelineData, test_value};
use nu_test_support::{fs::Stub::FileWithContent, prelude::*, tester::TestError};

#[test]
fn record_with_redefined_key() {
    let actual = nu!("{x: 1, x: 2}");

    assert!(actual.err.contains("redefined"));
}

#[test]
fn run_file_parse_error() {
    let actual = nu!(
        cwd: "tests/fixtures/eval",
        "nu script.nu"
    );

    assert!(actual.err.contains("unknown type"));
}

enum ExpectedOut<'a> {
    /// Equals a string exactly
    Eq(&'a str),
    /// Matches a regex
    Matches(&'a str),
    /// Produces an error (match regex)
    Error(&'a str),
    /// Drops a file that contains these contents
    FileEq(&'a str, &'a str),
}
use self::ExpectedOut::*;

fn test_eval(source: &str, expected_out: ExpectedOut) {
    Playground::setup("test_eval", |dirs, _playground| {
        let actual = nu!(
            cwd: dirs.test(),
            source,
        );

        match expected_out {
            Eq(eq) => {
                assert_eq!(actual.out, eq);
                assert!(actual.status.success());
            }
            Matches(regex) => {
                let compiled_regex = Regex::new(regex).expect("regex failed to compile");
                assert!(
                    compiled_regex.is_match(&actual.out).unwrap_or(false),
                    "eval out does not match: {}\n{}",
                    regex,
                    actual.out,
                );
                assert!(actual.status.success());
            }
            Error(regex) => {
                let compiled_regex = Regex::new(regex).expect("regex failed to compile");
                assert!(
                    compiled_regex.is_match(&actual.err).unwrap_or(false),
                    "eval err does not match: {regex}"
                );
                assert!(!actual.status.success());
            }
            FileEq(path, contents) => {
                let read_contents =
                    std::fs::read_to_string(dirs.test().join(path)).expect("failed to read file");
                assert_eq!(read_contents.trim(), contents);
                assert!(actual.status.success());
            }
        }
    });
}

#[test]
fn literal_bool() {
    test_eval("true", Eq("true"))
}

#[test]
fn literal_int() {
    test_eval("1", Eq("1"))
}

#[test]
fn literal_float() {
    test_eval("1.5", Eq("1.5"))
}

#[test]
fn literal_filesize() {
    test_eval("30MB", Eq("30.0 MB"))
}

#[test]
fn literal_duration() {
    test_eval("30ms", Eq("30ms"))
}

#[test]
fn literal_binary() {
    test_eval("0x[1f 2f f0]", Matches("Length.*1f.*2f.*f0"))
}

#[test]
fn literal_closure() {
    test_eval("{||}", Matches("closure_"))
}

#[test]
fn literal_closure_to_nuon() {
    test_eval("{||} | to nuon --serialize", Eq("\"{||}\""))
}

#[test]
fn literal_closure_to_json() {
    test_eval("{||} | to json --serialize", Eq("\"{||}\""))
}

#[test]
fn literal_closure_to_toml() {
    test_eval("{a: {||}} | to toml --serialize", Eq("a = \"{||}\""))
}

#[test]
fn literal_closure_to_yaml() {
    test_eval("{||} | to yaml --serialize", Eq(r#"!closure "{||}""#))
}

#[test]
fn literal_range() {
    test_eval("0..2..10", Matches("10"))
}

#[test]
fn literal_list() {
    test_eval("[foo bar baz]", Matches("foo.*bar.*baz"))
}

#[test]
fn literal_record() {
    test_eval("{foo: bar, baz: quux}", Matches("foo.*bar.*baz.*quux"))
}

#[test]
fn literal_table() {
    test_eval("[[a b]; [1 2] [3 4]]", Matches("a.*b.*1.*2.*3.*4"))
}

#[test]
fn literal_string() {
    test_eval(r#""foobar""#, Eq("foobar"))
}

#[test]
fn literal_raw_string() {
    test_eval("r#'bazquux'#", Eq("bazquux"))
}

#[test]
fn literal_date() {
    test_eval("2020-01-01T00:00:00Z", Matches("2020"))
}

#[test]
fn literal_nothing() {
    test_eval("null", Eq(""))
}

#[test]
fn list_spread() {
    test_eval("[foo bar ...[baz quux]] | length", Eq("4"))
}

#[test]
fn record_spread() {
    test_eval("{foo: bar ...{baz: quux}} | columns | length", Eq("2"))
}

#[test]
fn binary_op_example() {
    test_eval(
        "(([1 2] ++ [3 4]) == [1 2 3 4]) and (([1] ++ [2 3 4]) == [1 2 3 4])",
        Eq("true"),
    )
}

#[test]
fn binary_op_rhs_collects_in_variable() {
    // Regression test for #18323: a binary op whose RHS collects `$in` (e.g. through `not`,
    // a list literal, or a subexpression) used to clobber the LHS register and emit a
    // `register_uninitialized` compiler error.
    //
    // `$in` (not `$it`) is deliberate: it is the form that triggers the bug. The `$it`
    // equivalent compiles fine on `main`, so it would not guard this regression.
    test_eval(
        "[[v]; [1] [2] [6]] | where $in.v > 0 and not ($in.v > 5) | get v | to nuon",
        Eq("[1, 2]"),
    );
    test_eval(
        "[[v]; [1] [2] [6]] | where ($in.v == 1) or (0..($in.v) | is-empty) | get v | to nuon",
        Eq("[1]"),
    );
}

#[test]
fn range_from_expressions() {
    test_eval("(1 + 1)..(2 + 2)", Matches("2.*3.*4"))
}

#[test]
fn list_from_expressions() {
    test_eval(
        "[('foo' | str upcase) ('BAR' | str downcase)]",
        Matches("FOO.*bar"),
    )
}

#[test]
fn record_from_expressions() {
    test_eval("{('foo' | str upcase): 42}", Matches("FOO.*42"))
}

#[test]
fn call_spread() {
    test_eval(
        "echo foo bar ...[baz quux nushell]",
        Matches("foo.*bar.*baz.*quux.*nushell"),
    )
}

#[test]
fn call_flag() {
    test_eval("print -e message", Eq("")) // should not be visible on stdout
}

#[test]
fn call_named() {
    test_eval("10.123 | into string --decimals 1", Eq("10.1"))
}

#[test]
fn external_call() {
    test_eval("nu --testbin cococo foo=bar baz", Eq("foo=bar baz"))
}

#[test]
fn external_call_redirect_pipe() {
    test_eval(
        "nu --testbin cococo foo=bar baz | str upcase",
        Eq("FOO=BAR BAZ"),
    )
}

#[test]
fn external_call_redirect_capture() {
    test_eval(
        "echo (nu --testbin cococo foo=bar baz) | str upcase",
        Eq("FOO=BAR BAZ"),
    )
}

#[test]
fn external_call_redirect_file() {
    test_eval(
        "nu --testbin cococo hello out> hello.txt",
        FileEq("hello.txt", "hello"),
    )
}

#[test]
fn let_variable() {
    test_eval("let foo = 'test'; print $foo", Eq("test"))
}

#[test]
fn let_variable_mutate_error() {
    test_eval(
        "let foo = 'test'; $foo = 'bar'; print $foo",
        Error("immutable"),
    )
}

#[test]
fn constant() {
    test_eval("const foo = 1 + 2; print $foo", Eq("3"))
}

#[test]
fn constant_assign_error() {
    test_eval(
        "const foo = 1 + 2; $foo = 4; print $foo",
        Error("immutable"),
    )
}

#[test]
fn mut_variable() {
    test_eval("mut foo = 'test'; $foo = 'bar'; print $foo", Eq("bar"))
}

#[test]
fn mut_variable_append_assign() {
    test_eval(
        "mut foo = 'test'; $foo ++= 'bar'; print $foo",
        Eq("testbar"),
    )
}

#[test]
fn bind_in_variable_to_input() {
    test_eval("3 | (4 + $in)", Eq("7"))
}

#[test]
fn if_true() {
    test_eval("if true { 'foo' }", Eq("foo"))
}

#[test]
fn if_false() {
    test_eval("if false { 'foo' } | describe", Eq("nothing"))
}

#[test]
fn if_else_true() {
    test_eval("if 5 > 3 { 'foo' } else { 'bar' }", Eq("foo"))
}

#[test]
fn if_else_false() {
    test_eval("if 5 < 3 { 'foo' } else { 'bar' }", Eq("bar"))
}

#[test]
fn match_empty_fallthrough() {
    test_eval("match 42 { }; 'pass'", Eq("pass"))
}

#[test]
fn match_value() {
    test_eval("match 1 { 1 => 'pass', 2 => 'fail' }", Eq("pass"))
}

#[test]
fn match_value_default() {
    test_eval(
        "match 3 { 1 => 'fail1', 2 => 'fail2', _ => 'pass' }",
        Eq("pass"),
    )
}

#[test]
fn match_value_fallthrough() {
    test_eval("match 3 { 1 => 'fail1', 2 => 'fail2' }", Eq(""))
}

#[test]
fn match_variable() {
    test_eval(
        "match 'pass' { $s => { print $s }, _ => { print 'fail' } }",
        Eq("pass"),
    )
}

#[test]
fn match_variable_in_list() {
    test_eval("match [fail pass] { [$f, $p] => { print $p } }", Eq("pass"))
}

#[test]
fn match_passthrough_input() {
    test_eval(
        "'yes' | match [pass fail] { [$p, ..] => (collect { |y| $y ++ $p }) }",
        Eq("yespass"),
    )
}

#[test]
fn while_mutate_var() {
    test_eval("mut x = 2; while $x > 0 { print $x; $x -= 1 }", Eq("21"))
}

#[test]
fn for_list() {
    test_eval("for v in [1 2 3] { print ($v * 2) }", Eq("246"))
}

#[test]
fn for_seq() {
    test_eval("for v in (seq 1 4) { print ($v * 2) }", Eq("2468"))
}

#[test]
fn early_return() {
    test_eval("do { return 'foo'; 'bar' }", Eq("foo"))
}

#[test]
fn early_return_from_if() {
    test_eval("do { if true { return 'pass' }; 'fail' }", Eq("pass"))
}

#[test]
fn early_return_from_loop() {
    test_eval("do { loop { return 'pass' } }", Eq("pass"))
}

#[test]
fn early_return_from_while() {
    test_eval(
        "do { let x = true; while $x { return 'pass' } }",
        Eq("pass"),
    )
}

#[test]
fn early_return_from_for() {
    test_eval("do { for x in [pass fail] { return $x } }", Eq("pass"))
}

#[test]
fn early_return_keeps_metadata() -> Result {
    // An early `return` used to drop pipeline metadata that a value in tail position kept.
    // https://github.com/nushell/nushell/issues/18552
    test()
        .run(
            r#"def foo [] { if true { return ("body" | metadata set { merge {my: 302} }) } };
            foo | metadata | get my"#,
        )
        .expect_value_eq(302)
}

#[test]
fn early_return_keeps_stream() -> Result {
    // An early `return` used to collect its value; it should stay a stream like a value in
    // tail position does. Assert on the pipeline structure rather than `describe` output, so a
    // regression that collects the stream into a list is caught directly.
    let output = test().run_raw("def foo [] { return (1..3 | each { |x| $x }) }; foo")?;
    let PipelineData::ListStream(stream, _) = output.body else {
        panic!("early return should stay a stream")
    };
    stream
        .into_value()
        .map_err(TestError::from)
        .expect_value_eq(vec![1i64, 2, 3])
}

#[test]
#[serial]
fn early_return_with_finally_runs_cleanup_and_keeps_value() -> Result {
    // In-process `print` output isn't captured, so the `finally` block reports through the root
    // job's mailbox (`job send 0`) instead. The recovered message and the returned value confirm
    // the cleanup ran and the early-return value survived it. The pipeline is single-threaded, so
    // by the time `job recv` runs the message is already queued and no timeout is needed.
    test()
        .run(
            r#"def foo [] { try { return 1 } finally { "cleanup" | job send 0 } };
            let val = foo;
            {
                finally: (job recv --timeout 0sec),
                returned: $val,
            }"#,
        )
        .expect_value_eq(test_value!({
            finally: "cleanup",
            returned: 1,
        }))
}

#[test]
fn early_return_with_finally_keeps_metadata() -> Result {
    test()
        .run(
            r#"def foo [] { try { return ("body" | metadata set { merge {my: 302} }) } finally { } };
            foo | metadata | get my"#,
        )
        .expect_value_eq(302)
}

#[test]
fn early_return_not_intercepted_by_catch() -> Result {
    test()
        .run("def foo [] { try { return early } catch { 'caught' } }; foo")
        .expect_value_eq("early")
}

#[test]
fn early_return_in_export_env_stays_in_env_block() -> Result {
    // `return` inside `export-env` ends the environment block; it used to unwind further and
    // abort the enclosing command.
    test()
        .run("def foo [] { export-env { return }; 'after' }; foo")
        .expect_value_eq("after")
}

#[test]
fn early_return_in_export_env_guard_skips_rest_of_env_block() -> Result {
    test()
        .run(
            "def foo [] { export-env { if true { return }; $env.FOO = 'set' }; $env.FOO? };
            foo",
        )
        .expect_value_eq(())
}

#[test]
#[deps(NU)]
fn early_return_inside_command_does_not_skip_main() -> Result {
    // A `return` inside a command called at the top level of a script is consumed where the
    // command is called; only a top-level `return` should prevent `main` from running. This runs
    // the `nu` binary because that "skip main" decision lives in file evaluation, not in the
    // in-process engine.
    Playground::setup(
        "early_return_inside_command_does_not_skip_main",
        |dirs, sandbox| -> Result {
            sandbox.with_files(&[FileWithContent(
                "script.nu",
                "def helper [] { return 1 }\nhelper\ndef main [] { print 'main ran' }",
            )]);

            let result: CompleteResult =
                test().cwd(dirs.test()).run("nu -n script.nu | complete")?;
            assert_eq!(result.exit_code, 0);
            assert_contains("main ran", result.stdout);
            Ok(())
        },
    )
}

#[test]
fn early_return_in_module_export_env_does_not_abort_caller() -> Result {
    Playground::setup(
        "early_return_in_module_export_env_does_not_abort_caller",
        |dirs, sandbox| -> Result {
            sandbox.with_files(&[FileWithContent(
                "mod.nu",
                "export-env { return }\nexport def hi [] { 'hi' }",
            )]);
            test()
                .cwd(dirs.test())
                .run("def foo [] { use mod.nu *; hi }; foo")
                .expect_value_eq("hi")
        },
    )
}

// ===================================================================================
// Control-flow signals through `finally`: behavior matrix
//
// Markers printed by each case: F = finally ran, I/O = inner/outer finally, C = catch ran,
// A = code after the try ran, D = code after a loop ran. Trailing letter(s) = the returned
// value. So "FV" means: finally ran, then the command returned "V".
//
// How control flow interacts with `finally`.
//
// The rule: leaving a `try` runs its `finally` first, then the exit keeps going. The code after
// the try/finally does not run, and the exit is not lost. Nested finallys run inner first, then
// outer. If the finally itself exits, its exit replaces whatever the try was doing.
//
// Each case runs in-process via `finally_scenario`. A step of interest announces itself with
// `"<name>" | job send 0`; the scenario drains those into an ordered `ran` list and reports
// `{ ran, returned }` (or `{ ran, errored }` when the outcome is an error). So `ran` shows which
// steps ran and in what order, and `returned` shows the value that came out. Each test's expected
// record is the source of truth; the list below is a map.
//
// A. A signal leaves the try: the finally runs, the signal keeps going, code after is skipped.
//    1  return, finally, code after
//    2  return, catch + finally, code after
//    3  return, catch, no finally                [guard]
//    4  return, finally, tail                     [guard]
//    5  return, nested finallys
//    6  no signal, finally, code after            [guard]  (the only case where code after runs)
//    7  break, finally
//    8  break, nested finallys
//    9  continue, finally
//   10  continue, nested finallys
//
// B. The finally itself exits: its exit wins over the try's pending exit.
//   11  finally returns over a pending return
//   12  finally errors  over a pending return
//   13  finally breaks  over a pending return
//   14  finally continues over a pending return
//
// An error leaves a try the same way: the finally runs and the error propagates (and an enclosing
// `catch` may intercept it after the inner finallys run; see the enclosing-catch tests below).
//
// All of these hold. Every exit walks one unified `catch`/`finally` handler stack in nesting order
// (eval_ir.rs): `return`/error/`exit` unwind to the block base via the `run-finally` chain, and
// `break`/`continue` unwind to their loop via `jump-early` (compile/keyword.rs), running each
// pending finally and discarding each catch in the way. A `break`/`continue` that leaves a finally
// supersedes a pending return (13, 14 return the tail, not the leaked value). One that stays inside
// the finally, targeting a loop nested in it, is local and keeps the pending return instead (see
// the local_*_in_finally tests).

/// Run a `finally` scenario in-process and return the record it builds. A step in the snippet
/// announces itself with `"<name>" | job send 0`; the `drain` command (made available here) empties
/// those from the root job's mailbox into an ordered list, so a scenario ends in, for example,
/// `{ ran: (drain), returned: $returned }`.
fn finally_scenario(code: &str) -> Result<Value> {
    let drain = "def drain [] { mut r = []; loop { let m = (try { job recv --timeout 0sec } catch { break }); $r = ($r | append $m) }; $r }";
    test().run(format!("{drain}\n{code}"))
}

#[test]
#[serial]
fn finally_return_skips_code_after() -> Result {
    finally_scenario(
        r#"def foo [] { try { return "returned" } finally { "finally" | job send 0 }; "after try" | job send 0; "tail" }
        let returned = foo
        { ran: (drain), returned: $returned }"#,
    )
    .expect_value_eq(test_value!({ ran: ["finally"], returned: "returned" }))
}

#[test]
#[serial]
fn finally_return_with_catch_skips_code_after() -> Result {
    finally_scenario(
        r#"def foo [] { try { return "returned" } catch { "catch" | job send 0 } finally { "finally" | job send 0 }; "after try" | job send 0; "tail" }
        let returned = foo
        { ran: (drain), returned: $returned }"#,
    )
    .expect_value_eq(test_value!({ ran: ["finally"], returned: "returned" }))
}

#[test]
#[serial]
fn return_with_catch_no_finally_skips_code_after() -> Result {
    // guard: without a finally, `return` already skips the code after the try.
    finally_scenario(
        r#"def foo [] { try { return "returned" } catch { "catch" | job send 0 }; "after try" | job send 0; "tail" }
        let returned = foo
        { ran: (drain), returned: $returned }"#,
    )
    .expect_value_eq(test_value!({ ran: [], returned: "returned" }))
}

#[test]
#[serial]
fn finally_return_in_tail_runs_finally() -> Result {
    // guard: the everyday case, `return` last in the body, keeps working.
    finally_scenario(
        r#"def foo [] { try { return "returned" } finally { "finally" | job send 0 } }
        let returned = foo
        { ran: (drain), returned: $returned }"#,
    )
    .expect_value_eq(test_value!({ ran: ["finally"], returned: "returned" }))
}

#[test]
#[serial]
fn finally_return_runs_nested_finallys_then_skips_code_after() -> Result {
    finally_scenario(
        r#"def foo [] { try { try { return "returned" } finally { "inner" | job send 0 } } finally { "outer" | job send 0 }; "after try" | job send 0; "tail" }
        let returned = foo
        { ran: (drain), returned: $returned }"#,
    )
    .expect_value_eq(test_value!({ ran: ["inner", "outer"], returned: "returned" }))
}

#[test]
#[serial]
fn finally_and_code_after_both_run_on_success() -> Result {
    // 6 (guard): with nothing leaving the try, the finally runs and the code after runs too.
    finally_scenario(
        r#"def foo [] { try { "body" } finally { "finally" | job send 0 }; "after try" | job send 0; "tail" }
        let returned = foo
        { ran: (drain), returned: $returned }"#,
    )
    .expect_value_eq(test_value!({ ran: ["finally", "after try"], returned: "tail" }))
}

#[test]
#[serial]
fn finally_runs_on_break() -> Result {
    finally_scenario(
        r#"for x in [1] { try { break } finally { "finally" | job send 0 } }
        "after loop" | job send 0
        { ran: (drain) }"#,
    )
    .expect_value_eq(test_value!({ ran: ["finally", "after loop"] }))
}

#[test]
#[serial]
fn finally_runs_on_break_nested() -> Result {
    finally_scenario(
        r#"for x in [1] { try { try { break } finally { "inner" | job send 0 } } finally { "outer" | job send 0 } }
        "after loop" | job send 0
        { ran: (drain) }"#,
    )
    .expect_value_eq(test_value!({ ran: ["inner", "outer", "after loop"] }))
}

#[test]
#[serial]
fn finally_runs_on_continue() -> Result {
    finally_scenario(
        r#"for x in [1 2] { try { continue } finally { "finally" | job send 0 } }
        "after loop" | job send 0
        { ran: (drain) }"#,
    )
    .expect_value_eq(test_value!({ ran: ["finally", "finally", "after loop"] }))
}

#[test]
#[serial]
fn finally_runs_on_continue_nested() -> Result {
    finally_scenario(
        r#"for x in [1 2] { try { try { continue } finally { "inner" | job send 0 } } finally { "outer" | job send 0 } }
        "after loop" | job send 0
        { ran: (drain) }"#,
    )
    .expect_value_eq(test_value!({ ran: ["inner", "outer", "inner", "outer", "after loop"] }))
}

#[test]
#[serial]
fn finally_return_overrides_pending_return() -> Result {
    // 11 (guard): a `return` in the finally wins over a `return` pending from the try.
    finally_scenario(
        r#"def foo [] { try { return 5 } finally { "finally" | job send 0; return 7 } }
        let returned = foo
        { ran: (drain), returned: $returned }"#,
    )
    .expect_value_eq(test_value!({ ran: ["finally"], returned: 7 }))
}

#[test]
#[serial]
fn finally_error_overrides_pending_return() -> Result {
    // 12 (guard): an error in the finally wins over a `return` pending from the try.
    finally_scenario(
        r#"def foo [] { try { return 5 } finally { "finally" | job send 0; error make { msg: "from finally" } } }
        let errored = (try { foo; null } catch { |e| $e.msg })
        { ran: (drain), errored: $errored }"#,
    )
    .expect_value_eq(test_value!({ ran: ["finally"], errored: "from finally" }))
}

#[test]
#[serial]
fn finally_break_overrides_pending_return() -> Result {
    finally_scenario(
        r#"def foo [] { for x in [1 2] { try { return "returned" } finally { "finally" | job send 0; break } }; "after loop" | job send 0; "tail" }
        let returned = foo
        { ran: (drain), returned: $returned }"#,
    )
    .expect_value_eq(test_value!({ ran: ["finally", "after loop"], returned: "tail" }))
}

#[test]
#[serial]
fn finally_continue_overrides_pending_return() -> Result {
    finally_scenario(
        r#"def foo [] { for x in [1 2] { try { return "returned" } finally { "finally" | job send 0; continue } }; "after loop" | job send 0; "tail" }
        let returned = foo
        { ran: (drain), returned: $returned }"#,
    )
    .expect_value_eq(test_value!({ ran: ["finally", "finally", "after loop"], returned: "tail" }))
}

// A `break`/`continue` that stays inside the finally (it targets a loop nested in the finally, not
// an enclosing one) is local: it does not supersede the pending return, which resumes once the
// finally completes. Contrast 13/14, where the break/continue leaves the finally and wins.
#[test]
#[serial]
fn local_break_in_finally_keeps_pending_return() -> Result {
    finally_scenario(
        r#"def foo [] {
            try { return "returned" } finally { for y in [1] { break }; "finally" | job send 0 }
            "after try" | job send 0
            "tail"
        }
        let returned = foo
        { ran: (drain), returned: $returned }"#,
    )
    .expect_value_eq(test_value!({ ran: ["finally"], returned: "returned" }))
}

#[test]
#[serial]
fn local_continue_in_finally_keeps_pending_return() -> Result {
    finally_scenario(
        r#"def foo [] {
            try { return "returned" } finally { for y in [1 2] { continue }; "finally" | job send 0 }
            "after try" | job send 0
            "tail"
        }
        let returned = foo
        { ran: (drain), returned: $returned }"#,
    )
    .expect_value_eq(test_value!({ ran: ["finally"], returned: "returned" }))
}

// An error that is caught by an *enclosing* `catch` must still run the `finally` of every inner
// `try` it unwinds past, in order, before the catch handles it. These cases were wrong before the
// `catch`/`finally` handler stacks were unified into one walked in nesting order: the enclosing
// catch was consulted before the inner finally, so the inner finally was skipped.

#[test]
#[serial]
fn error_runs_inner_finally_before_outer_catch() -> Result {
    finally_scenario(
        r#"let returned = (try { try { error make { msg: "a" } } finally { "finally" | job send 0 } } catch { |e| $e.msg })
        { ran: (drain), returned: $returned }"#,
    )
    .expect_value_eq(test_value!({ ran: ["finally"], returned: "a" }))
}

#[test]
#[serial]
fn catch_that_throws_runs_inner_finally_before_outer_catch() -> Result {
    // The inner `catch` throws while an outer `catch` is waiting: the inner `finally` must run
    // before the outer catch, and the outer catch sees the catch's error.
    finally_scenario(
        r#"let returned = (try { try { error make { msg: "a" } } catch { "catch" | job send 0; error make { msg: "b" } } finally { "finally" | job send 0 } } catch { |e| $e.msg })
        { ran: (drain), returned: $returned }"#,
    )
    .expect_value_eq(test_value!({ ran: ["catch", "finally"], returned: "b" }))
}

#[test]
#[serial]
fn error_runs_nested_finallys_before_outer_catch() -> Result {
    finally_scenario(
        r#"let returned = (try { try { try { error make { msg: "a" } } finally { "f1" | job send 0 } } finally { "f2" | job send 0 } } catch { |e| $e.msg })
        { ran: (drain), returned: $returned }"#,
    )
    .expect_value_eq(test_value!({ ran: ["f1", "f2"], returned: "a" }))
}

#[test]
#[serial]
fn catch_that_throws_runs_nested_finallys_before_outer_catch() -> Result {
    finally_scenario(
        r#"let returned = (try { try { try { error make { msg: "a" } } catch { "c1" | job send 0; error make { msg: "b" } } finally { "f1" | job send 0 } } finally { "f2" | job send 0 } } catch { |e| $e.msg })
        { ran: (drain), returned: $returned }"#,
    )
    .expect_value_eq(test_value!({ ran: ["c1", "f1", "f2"], returned: "b" }))
}

#[test]
#[serial]
fn break_in_catch_runs_finally() -> Result {
    // A `break` from inside a `catch` still leaves through that `try`'s `finally`.
    finally_scenario(
        r#"for x in [1] { try { error make { msg: "a" } } catch { "catch" | job send 0; break } finally { "finally" | job send 0 } }
        "after loop" | job send 0
        { ran: (drain) }"#,
    )
    .expect_value_eq(test_value!({ ran: ["catch", "finally", "after loop"] }))
}

// A `finally` that completes abruptly (throws or exits) while a `return`/`break` is pending
// discards that pending exit; the finally's exit takes over (Java JLS 14.20.2, Python). If the
// finally completes normally instead (an error inside it is caught inside it), the pending exit
// survives.

#[test]
fn finally_error_caught_outside_discards_pending_return() -> Result {
    // The inner `finally` throws, discarding the pending `return 99`; the throw is caught by the
    // outer `catch`, and the return does not re-emerge, so the code after the try runs.
    test_eval(
        "def foo [] { try { try { return 99 } finally { error make { msg: rf } } } catch { null }; 7 }
        foo",
        Eq("7"),
    );
    Ok(())
}

#[test]
fn finally_error_caught_inside_keeps_pending_return() -> Result {
    // The `finally` completes normally (its error is caught within it), so the pending `return`
    // survives.
    test_eval(
        r#"def foo [] { try { return 5 } finally { try { error make { msg: x } } catch { "c" } } }
        foo"#,
        Eq("5"),
    );
    Ok(())
}

#[test]
#[serial]
fn finally_error_discards_pending_break() -> Result {
    // The inner `finally` throws, discarding the pending `break`; the throw is caught by the outer
    // `catch`, so the loop keeps iterating instead of breaking.
    finally_scenario(
        r#"for x in [1 2] {
            "i" | job send 0
            try { try { break } finally { error make { msg: rf } } } catch { "c" | job send 0 } finally { "f" | job send 0 }
        }
        "after" | job send 0
        { ran: (drain) }"#,
    )
    .expect_value_eq(test_value!({ ran: ["i", "c", "f", "i", "c", "f", "after"] }))
}

#[test]
#[serial]
fn finally_error_caught_outside_a_nested_finally_resumes_the_outer_exit_once() -> Result {
    // A `finally` (running for a pending `return`) contains a nested `try` whose own finally
    // throws. That error escapes the nested try and is caught alongside it, which discards the
    // nested try's normal completion but must not replay it: the catch continuation ("after") runs
    // once, and the outer `return 9` still comes out.
    finally_scenario(
        r#"def foo [] {
            try { return 9 } finally {
                try { try { "b" | job send 0 } finally { error make { msg: e } } } catch { "c" | job send 0 }
                "after" | job send 0
            }
        }
        let returned = (foo)
        { ran: (drain), returned: $returned }"#,
    )
    .expect_value_eq(test_value!({ ran: ["b", "c", "after"], returned: 9 }))
}

#[test]
#[serial]
fn finally_error_caught_inside_a_nested_finally_keeps_the_outer_return() -> Result {
    // The error is caught inside the outer `finally` (by a nested `catch`), so the outer finally
    // completes normally and the pending `return 3` survives, even though the error ran a nested
    // `finally` on the way.
    finally_scenario(
        r#"def foo [] {
            try { return 3 } finally {
                try { try { error make { msg: e } } finally { "if" | job send 0 } } catch { "c" | job send 0 }
            }
        }
        let returned = (foo)
        { ran: (drain), returned: $returned }"#,
    )
    .expect_value_eq(test_value!({ ran: ["if", "c"], returned: 3 }))
}

#[test]
fn nested_return_abandoned_in_finally_restores_outer_return() -> Result {
    // Inside the outer `return`'s finally, a nested `return` is itself abandoned (its own finally
    // throws, caught inside the outer finally, so the outer finally completes normally). The nested
    // return does not take over, so the outer `return "R1"` survives.
    test_eval(
        r#"def foo [] { try { return "R1" } finally { try { try { return "R2" } finally { error make { msg: boom } } } catch {|e| null } } }
        foo"#,
        Eq("R1"),
    );
    Ok(())
}

#[test]
fn try_no_catch() {
    test_eval("try { error make { msg: foo } }; 'pass'", Eq("pass"))
}

#[test]
fn try_catch_no_var() {
    test_eval(
        "try { error make { msg: foo } } catch { 'pass' }",
        Eq("pass"),
    )
}

#[test]
fn try_catch_var() {
    test_eval(
        "try { error make { msg: foo } } catch { |err| $err.msg }",
        Eq("foo"),
    )
}

#[test]
fn try_catch_with_non_literal_closure_no_var() {
    test_eval(
        r#"
            let error_handler = { || "pass" }
            try { error make { msg: foobar } } catch $error_handler
        "#,
        Eq("pass"),
    )
}

#[test]
fn try_catch_with_non_literal_closure() {
    test_eval(
        "
            let error_handler = { |err| $err.msg }
            try { error make { msg: foobar } } catch $error_handler
        ",
        Eq("foobar"),
    )
}

#[test]
fn try_catch_external() {
    test_eval(
        "try { nu -c 'exit 1' } catch { $env.LAST_EXIT_CODE }",
        Eq("1"),
    )
}

#[test]
fn row_condition() {
    test_eval(
        "[[a b]; [1 2] [3 4]] | where a < 3 | to nuon",
        Eq("[[a, b]; [1, 2]]"),
    )
}

#[test]
fn custom_command() {
    test_eval(
        r#"
            def cmd [a: int, b: string = 'fail', ...c: string, --x: int] { $"($a)($b)($c)($x)" }
            cmd 42 pass foo --x 30
        "#,
        Eq("42pass[foo]30"),
    )
}
