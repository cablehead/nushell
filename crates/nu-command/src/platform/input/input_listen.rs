use crossterm::{execute, terminal};
use nu_engine::command_prelude::*;
use std::time::Duration;

use nu_utils::time::Instant;

use nu_protocol::shell_error::{generic::GenericError, io::IoError};
use std::io::stdout;

use super::events::{EventTypeFilter, parse_event};

#[derive(Clone)]
pub struct InputListen;

impl Command for InputListen {
    fn name(&self) -> &str {
        "input listen"
    }

    fn search_terms(&self) -> Vec<&str> {
        vec!["prompt", "interactive", "keycode"]
    }

    fn signature(&self) -> Signature {
        Signature::build(self.name())
            .category(Category::Platform)
            .named(
                "types",
                SyntaxShape::List(Box::new(SyntaxShape::String)),
                "Listen for event of specified types only (can be one of: focus, key, mouse, paste, resize).",
                Some('t'),
            )
            .switch(
                "raw",
                "Add raw_code field with numeric value of keycode and raw_flags with bit mask flags.",
                Some('r'),
            )
            .named(
                "timeout",
                SyntaxShape::Duration,
                "How long to wait for input before returning.",
                Some('o')
            )
            .input_output_types(vec![(
                Type::Nothing,
                Type::Record([
                    ("keycode".to_string(), Type::String),
                    ("modifiers".to_string(), Type::List(Box::new(Type::String))),
                ].into()),
            )])
    }

    fn description(&self) -> &str {
        "Listen for user interface events."
    }

    fn extra_description(&self) -> &str {
        "There are 5 different type of events: focus, key, mouse, paste, resize. Each will produce a
corresponding record, distinguished by type field:
```
    { type: focus event: (gained|lost) }
    { type: key key_type: <key_type> code: <string> modifiers: [ <modifier> ... ] }
    { type: mouse col: <int> row: <int> kind: <string> modifiers: [ <modifier> ... ] }
    { type: paste content: <string> }
    { type: resize col: <int> row: <int> }
```
There are 6 `modifier` variants: shift, control, alt, super, hyper, meta.
There are 4 `key_type` variants:
    f - f1, f2, f3 ... keys
    char - alphanumeric and special symbols (a, A, 1, $ ...)
    media - dedicated media keys (play, pause, tracknext ...)
    other - keys not falling under previous categories (up, down, backspace, enter ...)"
    }
    fn examples(&self) -> Vec<Example<'_>> {
        vec![Example {
            description: "Listen for a keyboard shortcut and find out how nu receives it.",
            example: "input listen --types [key]",
            result: None,
        }]
    }
    fn run(
        &self,
        engine_state: &EngineState,
        stack: &mut Stack,
        call: &Call,
        _input: PipelineData,
    ) -> Result<PipelineData, ShellError> {
        let head = call.head;
        let event_type_filter = get_event_type_filter(engine_state, stack, call, head)?;
        let timeout: Option<Duration> = call.get_flag(engine_state, stack, "timeout")?;
        let add_raw = call.has_flag(engine_state, stack, "raw")?;
        let config = stack.get_config(engine_state);

        terminal::enable_raw_mode().map_err(|err| IoError::new(err, head, None))?;

        if config.use_kitty_protocol {
            if let Ok(false) = crossterm::terminal::supports_keyboard_enhancement() {
                println!("WARN: The terminal doesn't support use_kitty_protocol config.\r");
            }

            // enable kitty protocol
            //
            // Note that, currently, only the following support this protocol:
            // * [kitty terminal](https://sw.kovidgoyal.net/kitty/)
            // * [foot terminal](https://codeberg.org/dnkl/foot/issues/319)
            // * [WezTerm terminal](https://wezfurlong.org/wezterm/config/lua/config/enable_kitty_keyboard.html)
            // * [notcurses library](https://github.com/dankamongmen/notcurses/issues/2131)
            // * [neovim text editor](https://github.com/neovim/neovim/pull/18181)
            // * [kakoune text editor](https://github.com/mawww/kakoune/issues/4103)
            // * [dte text editor](https://gitlab.com/craigbarnes/dte/-/issues/138)
            // * [ghostty terminal](https://github.com/ghostty-org/ghostty/pull/317)
            //
            // Refer to https://sw.kovidgoyal.net/kitty/keyboard-protocol/ if you're curious.
            let _ = execute!(
                stdout(),
                crossterm::event::PushKeyboardEnhancementFlags(
                    crossterm::event::KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                )
            );
        }

        let console_state = event_type_filter.enable_events(head)?;
        let start = Instant::now();
        let mut remaining_time = timeout;

        loop {
            if let Some(t) = remaining_time
                && !crossterm::event::poll(t).map_err(|_| {
                    ShellError::Generic(GenericError::new("Error with user input", "", head))
                })?
            {
                terminal::disable_raw_mode().map_err(|err| IoError::new(err, head, None))?;
                return Err(ShellError::Generic(GenericError::new(
                    "Timed out while waiting for user input",
                    "no input was received within the timeout duration",
                    head,
                )));
            }
            let event = crossterm::event::read().map_err(|_| {
                ShellError::Generic(GenericError::new("Error with user input", "", head))
            })?;
            let event = parse_event(head, &event, &event_type_filter, add_raw);
            if let Some(event) = event {
                terminal::disable_raw_mode().map_err(|err| IoError::new(err, head, None))?;
                if config.use_kitty_protocol {
                    let _ = execute!(
                        std::io::stdout(),
                        crossterm::event::PopKeyboardEnhancementFlags
                    );
                }

                console_state.restore();
                return Ok(event.into_pipeline_data());
            }

            remaining_time = timeout.map(|t| t.saturating_sub(start.elapsed()));
        }
    }
}

pub(super) fn get_event_type_filter(
    engine_state: &EngineState,
    stack: &mut Stack,
    call: &Call,
    head: Span,
) -> Result<EventTypeFilter, ShellError> {
    let event_type_filter = call.get_flag::<Value>(engine_state, stack, "types")?;
    let event_type_filter = event_type_filter
        .map(|list| EventTypeFilter::from_value(list, head))
        .transpose()?
        .unwrap_or_else(EventTypeFilter::all);
    Ok(event_type_filter)
}
