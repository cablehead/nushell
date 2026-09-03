use nu_engine::{ClosureEvalOnce, command_prelude::*};
use nu_protocol::{engine::Closure, shell_error::io::IoError};
use std::{sync::mpsc, thread};

#[derive(Clone)]
pub struct Interleave;

impl Command for Interleave {
    fn name(&self) -> &str {
        "interleave"
    }

    fn description(&self) -> &str {
        "Read multiple streams in parallel and combine them into one stream."
    }

    fn extra_description(&self) -> &str {
        "This combinator is useful for reading output from multiple commands.

If input is provided to `interleave`, the input will be combined with the
output of the closures. This enables `interleave` to be used at any position
within a pipeline.

Because items from each stream will be inserted into the final stream as soon
as they are available, there is no guarantee of how the final output will be
ordered. However, the order of items from any given stream is guaranteed to be
preserved as they were in that stream.

If interleaving streams in a fair (round-robin) manner is desired, consider
using `zip { ... } | flatten` instead."
    }

    fn signature(&self) -> Signature {
        Signature::build("interleave")
            .input_output_types(vec![
                (Type::List(Type::Any.into()), Type::List(Type::Any.into())),
                (Type::Nothing, Type::List(Type::Any.into())),
            ])
            .named(
                "buffer-size",
                SyntaxShape::Int,
                "Number of items to buffer from the streams. Increases memory usage, but can help \
                    performance when lots of output is produced.",
                Some('b'),
            )
            .rest(
                "closures",
                SyntaxShape::Closure(None),
                "The closures that will generate streams to be combined.",
            )
            .allow_variants_without_examples(true)
            .category(Category::Filters)
    }

    fn examples(&self) -> Vec<Example<'_>> {
        vec![
            Example {
                example: "seq 1 50 | wrap a | interleave { seq 1 50 | wrap b }",
                description: "Read two sequences of numbers into separate columns of a table.
Note that the order of rows with 'a' columns and rows with 'b' columns is arbitrary.",
                result: None,
            },
            Example {
                example: "seq 1 3 | interleave { seq 4 6 } | sort",
                description: "Read two sequences of numbers, one from input. Sort for consistency.",
                result: Some(Value::test_list(vec![
                    Value::test_int(1),
                    Value::test_int(2),
                    Value::test_int(3),
                    Value::test_int(4),
                    Value::test_int(5),
                    Value::test_int(6),
                ])),
            },
            Example {
                example: r#"interleave { "foo\nbar\n" | lines } { "baz\nquux\n" | lines } | sort"#,
                description: "Read two sequences, but without any input. Sort for consistency.",
                result: Some(Value::test_list(vec![
                    Value::test_string("bar"),
                    Value::test_string("baz"),
                    Value::test_string("foo"),
                    Value::test_string("quux"),
                ])),
            },
            Example {
                example: r#"(
interleave
    { nu -c "print hello; print world" | lines | each { "greeter: " ++ $in } }
    { nu -c "print nushell; print rocks" | lines | each { "evangelist: " ++ $in } }
)"#,
                description: "Run two commands in parallel and annotate their output.",
                result: None,
            },
            Example {
                example: "seq 1 20000 | interleave --buffer-size 16 { seq 1 20000 } | math sum",
                description: "Use a buffer to increase the performance of high-volume streams.",
                result: None,
            },
        ]
    }

    fn run(
        &self,
        engine_state: &EngineState,
        stack: &mut Stack,
        call: &Call,
        input: PipelineData,
    ) -> Result<PipelineData, ShellError> {
        let head = call.head;
        let closures: Vec<Closure> = call.rest(engine_state, stack, 0)?;
        let buffer_size: usize = call
            .get_flag(engine_state, stack, "buffer-size")?
            .unwrap_or(0);

        let (tx, rx) = mpsc::sync_channel(buffer_size);

        // Spawn the threads for the input and closure outputs
        (!input.is_nothing())
            .then(|| Ok(input))
            .into_iter()
            .chain(closures.into_iter().map(|closure| {
                ClosureEvalOnce::new(engine_state, stack, closure)
                    .run_with_input(PipelineData::empty())
            }))
            .try_for_each(|stream| {
                stream.and_then(|stream| {
                    // Then take the stream and spawn a thread to send it to our channel
                    let tx = tx.clone();
                    thread::Builder::new()
                        .name("interleave consumer".into())
                        .spawn(move || {
                            for value in stream {
                                if tx.send(value).is_err() {
                                    // Stop sending if the channel is dropped
                                    break;
                                }
                            }
                        })
                        .map(|_| ())
                        .map_err(|err| IoError::new(err, head, None).into())
                })
            })?;

        // Now that threads are writing to the channel, we just return it as a stream
        Ok(rx
            .into_iter()
            .into_pipeline_data(head, engine_state.signals().clone()))
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use nu_protocol::{ByteStreamType, Signals};
    use std::io::{self, Read};
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    };
    use std::time::Duration;

    #[test]
    fn test_examples() -> nu_test_support::Result {
        nu_test_support::test().examples(Interleave)
    }

    /// Blocks in `read` until the test releases it, standing in for a socket
    /// or a child process pipe.
    ///
    /// `Signals::empty()` means this stream ignores interrupts, the same as
    /// a real child process's stdout (see `ByteStream::child()`). So if the
    /// producer thread stops, `interleave` is the only thing that could have
    /// stopped it. That is what makes this a test of `interleave`.
    struct SlowReader {
        /// Sent just before `read` blocks, so the test knows the producer
        /// is inside the stream.
        started: mpsc::SyncSender<()>,
        /// `read` blocks here until the test sends (or drops) it.
        gate: mpsc::Receiver<()>,
    }

    impl Read for SlowReader {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let _ = self.started.send(());
            match self.gate.recv() {
                // Pretend a single byte arrived.
                Ok(()) if !buf.is_empty() => {
                    buf[0] = b'x';
                    Ok(1)
                }
                Ok(()) => Ok(0),
                // The test dropped `gate_tx`: report EOF.
                Err(_) => Ok(0),
            }
        }
    }

    /// A command whose output is a [`SlowReader`] byte stream, standing in
    /// for an external command.
    #[derive(Clone)]
    struct SlowProducer {
        started: mpsc::SyncSender<()>,
        // `Command::run` only takes `&self`, so this is taken exactly once.
        gate: Arc<Mutex<Option<mpsc::Receiver<()>>>>,
    }

    impl Command for SlowProducer {
        fn name(&self) -> &str {
            "interleave-test-slow-producer"
        }

        fn description(&self) -> &str {
            "Test-only: a byte stream that blocks in `read` until released."
        }

        fn signature(&self) -> Signature {
            Signature::build("interleave-test-slow-producer")
                .input_output_types(vec![(Type::Nothing, Type::Any)])
        }

        fn run(
            &self,
            _engine_state: &EngineState,
            _stack: &mut Stack,
            call: &Call,
            _input: PipelineData,
        ) -> Result<PipelineData, ShellError> {
            let gate = self
                .gate
                .lock()
                .expect("gate lock")
                .take()
                .expect("interleave-test-slow-producer invoked more than once");
            let reader = SlowReader {
                started: self.started.clone(),
                gate,
            };
            let stream =
                ByteStream::read(reader, call.head, Signals::empty(), ByteStreamType::Binary);
            Ok(PipelineData::byte_stream(stream, None))
        }
    }

    /// A producer blocked in its stream's `read` must stop when the pipeline
    /// is interrupted.
    ///
    /// `SlowReader` sends on `started` once per `read`. The test waits for
    /// the producer to block, interrupts, then releases the read. A second
    /// `started` means the producer went back for more data after the
    /// interrupt, which is the bug.
    #[test]
    fn producer_ignores_interrupt_while_blocked_in_inner_stream() {
        let mut tester = nu_test_support::test();

        let (started_tx, started_rx) = mpsc::sync_channel::<()>(0);
        let (gate_tx, gate_rx) = mpsc::sync_channel::<()>(0);
        let producer = SlowProducer {
            started: started_tx,
            gate: Arc::new(Mutex::new(Some(gate_rx))),
        };

        let mut working_set = StateWorkingSet::new(&tester.engine_state);
        working_set.add_decl(Box::new(producer));
        let delta = working_set.render();
        tester
            .engine_state
            .merge_delta(delta)
            .expect("merge_delta should succeed");

        let interrupt = Arc::new(AtomicBool::new(false));
        tester
            .engine_state
            .set_signals(Signals::new(interrupt.clone()));

        // Buffered, so a producer that gets past `read` can send even
        // though nothing is draining this pipeline.
        let result = tester
            .run_raw_with_data(
                "interleave --buffer-size 4 { interleave-test-slow-producer }",
                PipelineData::empty(),
            )
            .expect("interleave itself must return without blocking");

        // Wait for the producer to block inside `read`.
        started_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("producer thread never reached its inner stream");

        // Ctrl-C, while it is blocked.
        interrupt.store(true, Ordering::SeqCst);

        // Release the read, as a real source eventually would.
        gate_tx.send(()).expect("gate receiver still alive");

        // The producer must now stop, not ask the stream for more.
        match started_rx.recv_timeout(Duration::from_millis(500)) {
            Ok(()) => panic!(
                "producer asked its stream for more data after the pipeline \
                 was interrupted"
            ),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // Stopped after the interrupt.
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                // Thread exited outright. Also fine.
            }
        }

        drop(result);
    }
}
