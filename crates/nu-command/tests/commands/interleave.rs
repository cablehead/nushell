#[test]
fn interleave_cancels_slow_branch_when_downstream_drops() {
    // The slow branch's generator parks in `sleep 3sec` before its body
    // would write a marker file. The fast branch yields a single value, then
    // `take 1` drops the downstream consumer.
    //
    // After the pipeline returns, sleep 4sec lets the slow worker finish if
    // it wasn't cancelled. Then we read the marker file: if it contains the
    // post-sleep write, cancellation didn't take effect; if it's empty, the
    // worker bailed during sleep.
    let dir = tempfile::tempdir().expect("tempdir");
    let marker = dir.path().join("marker");
    std::fs::write(&marker, "").expect("init marker");

    let marker_path = marker.to_string_lossy().into_owned();
    let script = format!(
        r#"
        (
          interleave
            {{ 1 }}
            {{ generate {{|_=0| sleep 3sec; "wrote" | save --append --raw "{marker_path}"; {{out: 2, next: 0}} }} }}
        ) | take 1 | length
        sleep 4sec
        "#
    );

    let result = nu!(&script);
    assert_eq!(result.out.trim(), "1");

    let after = std::fs::read_to_string(&marker).expect("read marker");
    assert!(
        after.is_empty(),
        "slow branch's post-sleep write executed -- cancellation didn't reach it. marker contents: {after:?}"
    );
}

#[test]
fn interleave_external_commands() {
    let result = nu!("interleave \
        { nu -n -c 'print hello; print world' | lines | each { 'greeter: ' ++ $in } } \
        { nu -n -c 'print nushell; print rocks' | lines | each { 'evangelist: ' ++ $in } } | \
        each { print }; null");
    assert!(result.out.contains("greeter: hello"), "{}", result.out);
    assert!(result.out.contains("greeter: world"), "{}", result.out);
    assert!(result.out.contains("evangelist: nushell"), "{}", result.out);
    assert!(result.out.contains("evangelist: rocks"), "{}", result.out);
}
