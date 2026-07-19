use crate::RegId;

/// Whether a [`Handler`] catches an error or always runs when its `try` block is left.
///
/// Both live on the same [`HandlerStack`] so their relative nesting order is structural: a
/// `try`'s `finally` is pushed before its `catch`, so the `catch` sits above and is found first
/// for an error in the `try` body, while the `finally` runs afterward on the way out. On any exit
/// (error, `return`, `break`/`continue`, `exit`) the evaluator walks the stack, running the
/// [`Finally`](HandlerKind::Finally) handlers and handling or discarding the
/// [`Catch`](HandlerKind::Catch) ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandlerKind {
    /// A `catch` block: handles an error escaping the `try` body, stopping its propagation.
    Catch,
    /// A `finally` block: runs whenever the `try` is left, then the pending exit continues.
    Finally,
}

/// Describes a `catch`/`finally` handler stored during IR evaluation.
#[derive(Debug, Clone, Copy)]
pub struct Handler {
    /// Instruction index within the block that will handle the error or run the `finally`
    pub handler_index: usize,
    /// Register to put the error information into, when an error occurs
    pub error_register: Option<RegId>,
    /// Whether this handler catches an error or always runs on the way out
    pub kind: HandlerKind,
}

/// Keeps track of the `catch` and `finally` handlers pushed during evaluation of an IR block.
#[derive(Debug, Clone, Default)]
pub struct HandlerStack {
    handlers: Vec<Handler>,
}

impl HandlerStack {
    pub const fn new() -> HandlerStack {
        HandlerStack { handlers: vec![] }
    }

    /// Get the current base of the stack, which establishes a frame.
    pub fn get_base(&self) -> usize {
        self.handlers.len()
    }

    /// Push a new handler onto the stack.
    pub fn push(&mut self, handler: Handler) {
        self.handlers.push(handler);
    }

    /// Whether any `finally` handler is pending in the current frame (at or above `base`). Used to
    /// keep the streaming fast path for a `return` that has no `finally` to run.
    pub fn has_finally(&self, base: usize) -> bool {
        self.handlers[base..]
            .iter()
            .any(|h| h.kind == HandlerKind::Finally)
    }

    /// Try to pop a handler from the stack. Won't go below `base`, to avoid retrieving a
    /// handler belonging to a parent frame.
    pub fn pop(&mut self, base: usize) -> Option<Handler> {
        if self.handlers.len() > base {
            self.handlers.pop()
        } else {
            None
        }
    }

    /// Reset the stack to the state it was in at the beginning of the frame, in preparation to
    /// return control to the parent frame.
    pub fn leave_frame(&mut self, base: usize) {
        if self.handlers.len() >= base {
            self.handlers.truncate(base);
        } else {
            panic!(
                "HandlerStack bug: tried to leave frame at {base}, but current base is {}",
                self.get_base()
            )
        }
    }
}
