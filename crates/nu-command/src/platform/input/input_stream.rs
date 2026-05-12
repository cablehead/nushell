use std::io::stdout;
use std::time::Duration;

use crossterm::{execute, terminal};
use nu_engine::command_prelude::*;
use nu_protocol::{ListStream, Signals, shell_error::io::IoError};

use super::events::{DeferredConsoleRestore, EventTypeFilter, parse_event};
use super::input_listen::get_event_type_filter;

#[derive(Clone)]
pub struct InputStream;

impl Command for InputStream {
    fn name(&self) -> &str {
        "input stream"
    }

    fn search_terms(&self) -> Vec<&str> {
        vec!["prompt", "interactive", "keycode", "stream", "events"]
    }

    fn signature(&self) -> Signature {
        Signature::build(self.name())
            .category(Category::Platform)
            .named(
                "types",
                SyntaxShape::List(Box::new(SyntaxShape::String)),
                "Listen for events of specified types only (can be one of: focus, key, mouse, paste, resize).",
                Some('t'),
            )
            .switch(
                "raw",
                "Add raw_code field with numeric value of keycode and raw_flags with bit mask flags.",
                Some('r'),
            )
            .input_output_types(vec![(Type::Nothing, Type::list(Type::Any))])
    }

    fn description(&self) -> &str {
        "Continuously emit user interface events as a stream."
    }

    fn extra_description(&self) -> &str {
        "Like `input listen`, but returns a stream of events that can be composed with
`interleave`, `take until`, and other stream operations to build event-loop style
applications. The stream terminates -- and the terminal is restored to its prior
state -- when the consumer stops reading (e.g. via `take until`), when Ctrl-C is
pressed, or when the script exits.

Event records have the same shape as `input listen`. See `help input listen` for
details on the per-type record format and the modifier/key_type variants."
    }

    fn examples(&self) -> Vec<Example<'_>> {
        vec![
            Example {
                description: "Print every key press until 'q' is pressed.",
                example: "input stream --types [key] | take until {|k| $k.code == 'q' } | each {|k| print $k.code }",
                result: None,
            },
            Example {
                description: "Interleave key events with a 1-second tick.",
                example: "input stream --types [key] | interleave { generate {|_=0| sleep 1sec; {out: (date now), next: 0} } } | take 5",
                result: None,
            },
        ]
    }

    fn run(
        &self,
        engine_state: &EngineState,
        stack: &mut Stack,
        call: &Call,
        _input: PipelineData,
    ) -> Result<PipelineData, ShellError> {
        let head = call.head;
        let filter = get_event_type_filter(engine_state, stack, call, head)?;
        let add_raw = call.has_flag(engine_state, stack, "raw")?;

        terminal::enable_raw_mode().map_err(|err| IoError::new(err, head, None))?;
        let console_state = filter.enable_events(head)?;
        let guard = TerminalGuard {
            console_state: Some(console_state),
        };

        let signals = engine_state.signals().clone();
        let iter = EventStream {
            head,
            filter,
            add_raw,
            signals: signals.clone(),
            _guard: guard,
        };

        Ok(PipelineData::list_stream(
            ListStream::new(iter, head, signals),
            None,
        ))
    }
}

/// Restores terminal state when dropped. Held by the iterator so it lives as long
/// as the stream is alive; runs whether the stream is fully drained, partially
/// consumed (e.g. `take until`), or dropped on Ctrl-C.
struct TerminalGuard {
    console_state: Option<DeferredConsoleRestore>,
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        if let Some(state) = self.console_state.take() {
            state.restore();
        }
        let _ = terminal::disable_raw_mode();
        // Best-effort: also pop kitty enhancement flags if they were pushed.
        // `input listen` only pushes them when `use_kitty_protocol` is set; we don't
        // push them here yet. If/when we add the flag, mirror the pop in this Drop.
        let _ = execute!(stdout(), crossterm::cursor::Show);
    }
}

struct EventStream {
    head: Span,
    filter: EventTypeFilter,
    add_raw: bool,
    signals: Signals,
    _guard: TerminalGuard,
}

impl Iterator for EventStream {
    type Item = Value;

    fn next(&mut self) -> Option<Self::Item> {
        // Poll on a short interval so the thread wakes regularly to check
        // signals (Ctrl-C, downstream stop) between polls. Without this the
        // thread would park indefinitely in `event::read()`.
        const POLL_INTERVAL: Duration = Duration::from_millis(100);

        loop {
            if self.signals.interrupted() {
                return None;
            }

            match crossterm::event::poll(POLL_INTERVAL) {
                Ok(true) => match crossterm::event::read() {
                    Ok(ev) => {
                        if let Some(value) = parse_event(self.head, &ev, &self.filter, self.add_raw)
                        {
                            return Some(value);
                        }
                        // Filter dropped this event (e.g. key release). Keep polling.
                    }
                    Err(_) => return None,
                },
                Ok(false) => continue,
                Err(_) => return None,
            }
        }
    }
}
