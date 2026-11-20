use crate::{
    BlockId, DeclId, Filesize, RegId, ShellError, Span, Value, VarId,
    ast::{CellPath, Expression, Operator, Pattern, RangeInclusion},
    engine::{EngineState, ScopeBindings},
};
use chrono::{DateTime, FixedOffset};
use serde::{Deserialize, Serialize};
use std::{fmt, sync::Arc};

mod call;
mod display;

pub use call::*;
pub use display::{FmtInstruction, FmtIrBlock};

/// Instruction-index range where a nested block's local command/module bindings apply.
///
/// Keyword bodies (`if`/`for`/…) are IR-**inlined** into the parent block and never enter
/// `eval_ir_block`. The compiler records these regions so `scope` can resolve locals by
///  comparing the current program counter.
#[derive(Clone, Debug)]
pub struct ScopeRegion {
    /// Inclusive start index into [`IrBlock::instructions`].
    pub start: usize,
    /// Exclusive end index into [`IrBlock::instructions`].
    pub end: usize,
    pub bindings: Arc<ScopeBindings>,
}

impl ScopeRegion {
    pub fn contains(&self, instruction_index: usize) -> bool {
        self.start <= instruction_index && instruction_index < self.end
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct IrBlock {
    pub instructions: Vec<Instruction>,
    pub spans: Vec<Span>,
    #[serde(with = "serde_arc_u8_array")]
    pub data: Arc<[u8]>,
    /// Additional information that can be added to help with debugging
    pub comments: Vec<Box<str>>,
    pub register_count: u32,
    pub file_count: u32,
    /// Local scope regions for inlined nested blocks (see [`ScopeRegion`]).
    /// Not serialized — only meaningful in the process that compiled the block.
    #[serde(skip)]
    pub scope_regions: Vec<ScopeRegion>,
    /// Protected `try` regions, which drive `catch` and `finally` (see [`TryRegion`]).
    ///
    /// Nothing is pushed or popped at runtime: each `try` records, at compile time, the
    /// instruction range it protects and where its `catch` or `finally` body lives. When an
    /// error or a structured exit ([`Instruction::Leave`], [`Instruction::ReturnEarly`]) unwinds,
    /// the evaluator looks up the regions covering the current instruction.
    pub try_regions: Vec<TryRegion>,
}

/// A protected `try` region and the `catch` or `finally` body it routes to.
///
/// Regions nest by containment, so the innermost region covering an instruction is the smallest
/// one around it. A `try`'s `catch` region covers only its body, while its `finally` region covers
/// the body and the `catch`, so the `catch` is always found before its own `finally`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TryRegion {
    /// First protected instruction index (inclusive).
    pub start: usize,
    /// One past the last protected instruction index (exclusive).
    pub end: usize,
    /// Instruction index of the `catch` or `finally` body this region transfers to.
    pub target: usize,
    /// One past the last instruction of the `catch` or `finally` body (exclusive). The body
    /// occupies `[target, handler_end)`. While a `finally` runs for a pending exit, control that
    /// leaves this range other than through its `end-finally` abandons that exit.
    pub handler_end: usize,
    /// Register that receives the error (or empty, or the `try` value) on entry to the body.
    /// `None` when nothing is bound.
    pub error_register: Option<RegId>,
    /// Whether this region catches an error or always runs on the way out.
    pub kind: TryRegionKind,
}

impl TryRegion {
    /// Whether the protected range covers instruction `index`.
    pub fn covers(&self, index: usize) -> bool {
        self.start <= index && index < self.end
    }
}

/// Whether a [`TryRegion`] is a `catch` (handles an error, stopping it) or a `finally` (runs on
/// every exit, after which the pending exit continues).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TryRegionKind {
    Catch,
    Finally,
}

impl fmt::Debug for IrBlock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IrBlock")
            .field("instructions", &self.instructions)
            .field("spans", &self.spans)
            .field("data", &self.data)
            .field("comments", &self.comments)
            .field("register_count", &self.register_count)
            .field("file_count", &self.file_count)
            .field("scope_regions", &self.scope_regions.len())
            .field("try_regions", &self.try_regions)
            .finish()
    }
}

impl IrBlock {
    /// Returns a value that can be formatted with [`Display`](std::fmt::Display) to show a detailed
    /// listing of the instructions contained within this [`IrBlock`].
    pub fn display<'a>(&'a self, engine_state: &'a EngineState) -> FmtIrBlock<'a> {
        FmtIrBlock {
            engine_state,
            ir_block: self,
        }
    }
}

/// A slice into the `data` array of a block. This is a compact and cache-friendly way to store
/// string data that a block uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct DataSlice {
    pub start: u32,
    pub len: u32,
}

impl DataSlice {
    /// A data slice that contains no data. This slice is always valid.
    pub const fn empty() -> DataSlice {
        DataSlice { start: 0, len: 0 }
    }
}

impl std::ops::Index<DataSlice> for [u8] {
    type Output = [u8];

    fn index(&self, index: DataSlice) -> &Self::Output {
        &self[index.start as usize..(index.start as usize + index.len as usize)]
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Instruction {
    /// Unreachable code path (error)
    Unreachable,
    /// Load a literal value into the `dst` register
    LoadLiteral { dst: RegId, lit: Literal },
    /// Load a clone of a boxed value into the `dst` register (e.g. from const evaluation)
    LoadValue { dst: RegId, val: Box<Value> },
    /// Move a register. Value is taken from `src` (used by this instruction).
    Move { dst: RegId, src: RegId },
    /// Copy a register (must be a collected value). Value is still in `src` after this instruction.
    ///
    /// The one exception is an external whose stderr has its own pipe, which `try` passes on
    /// uncollected (see `TryCollect`): `dst` gets the value its stdout collects to, and `src`
    /// keeps the external.
    Clone { dst: RegId, src: RegId },
    /// Collect a stream in a register to a value.
    /// Because it collects to a value, nushell will ignore the errors in the stream.
    /// It's important when the stream is from an external command
    Collect { src_dst: RegId },
    /// Collect a stream in a register to a value.
    /// But it's different from `Collect` in that if there is an error in the stream, it will be
    /// returned as an error instead of being ignored.
    ///
    /// An external whose stderr has its own pipe stays a stream, so the next command still gets
    /// stdout and stderr apart. Its output is read into memory and its exit status checked.
    TryCollect { src_dst: RegId },
    /// Change the span of the contents of a register to the span of this instruction.
    Span { src_dst: RegId },
    /// Drop the value/stream in a register, without draining
    Drop { src: RegId },
    /// Drain the value/stream in a register and discard (e.g. semicolon).
    ///
    /// If passed a stream from an external command, sets $env.LAST_EXIT_CODE to the resulting exit
    /// code, and invokes any available error handler with Empty, or if not available, returns an
    /// exit-code-only stream, leaving the block.
    Drain { src: RegId },
    /// Drain the value/stream in a register and discard only if this is the last pipeline element.
    // TODO: see if it's possible to remove this
    DrainIfEnd { src: RegId },
    /// Load the value of a variable into the `dst` register.
    ///
    /// When `preserve_origin` is false (the default), the value is re-spanned to this
    /// instruction's span so error labels point at the use site. When true, the value keeps
    /// the span from where it was defined (used by `metadata $var`).
    LoadVariable {
        dst: RegId,
        var_id: VarId,
        #[serde(default)]
        preserve_origin: bool,
    },
    /// Store the value of a variable from the `src` register
    StoreVariable { var_id: VarId, src: RegId },
    /// Remove a variable from the stack, freeing up whatever resources were associated with it
    DropVariable { var_id: VarId },
    /// Load the value of an environment variable into the `dst` register
    LoadEnv { dst: RegId, key: DataSlice },
    /// Load the value of an environment variable into the `dst` register, or `Nothing` if it
    /// doesn't exist
    LoadEnvOpt { dst: RegId, key: DataSlice },
    /// Store the value of an environment variable from the `src` register
    StoreEnv { key: DataSlice, src: RegId },
    /// Add a positional arg to the next (internal) call.
    PushPositional { src: RegId },
    /// Add a list of args to the next (internal) call (spread/rest).
    AppendRest { src: RegId },
    /// Add a named arg with no value to the next (internal) call.
    PushFlag { name: DataSlice },
    /// Add a short named arg with no value to the next (internal) call.
    PushShortFlag { short: DataSlice },
    /// Add a named arg with a value to the next (internal) call.
    PushNamed { name: DataSlice, src: RegId },
    /// Add a short named arg with a value to the next (internal) call.
    PushShortNamed { short: DataSlice, src: RegId },
    /// Add parser info to the next (internal) call.
    PushParserInfo {
        name: DataSlice,
        info: Box<Expression>,
    },
    /// Set the redirection for stdout for the next call (only).
    ///
    /// The register for a file redirection is not consumed.
    RedirectOut { mode: RedirectMode },
    /// Set the redirection for stderr for the next call (only).
    ///
    /// The register for a file redirection is not consumed.
    RedirectErr { mode: RedirectMode },
    /// Throw an error if stderr wasn't redirected in the given stream. `src` is preserved.
    CheckErrRedirected { src: RegId },
    /// Open a file for redirection, pushing it onto the file stack.
    OpenFile {
        file_num: u32,
        path: RegId,
        append: bool,
    },
    /// Write data from the register to a file. This is done to finish a file redirection, in case
    /// an internal command or expression was evaluated rather than an external one.
    ///
    /// The register is left empty, except for an external whose stderr has its own pipe (the next
    /// command reads stdout and stderr apart): only its stdout is written, and the register keeps
    /// the external, so the next command still gets its stderr.
    WriteFile { file_num: u32, src: RegId },
    /// Pop a file used for redirection from the file stack.
    CloseFile { file_num: u32 },
    /// Make a call. The input is taken from `src_dst`, and the output is placed in `src_dst`,
    /// overwriting it. The argument stack is used implicitly and cleared when the call ends.
    Call { decl_id: DeclId, src_dst: RegId },
    /// Append a value onto the end of a string. Uses `to_expanded_string(", ", ...)` on the value.
    /// Used for string interpolation literals. Not the same thing as the `++` operator.
    StringAppend { src_dst: RegId, val: RegId },
    /// Convert a string into a glob. Used for glob interpolation and setting glob variables. If the
    /// value is already a glob, it won't be modified (`no_expand` will have no effect).
    GlobFrom { src_dst: RegId, no_expand: bool },
    /// Push a value onto the end of a list. Used to construct list literals.
    ListPush { src_dst: RegId, item: RegId },
    /// Spread a value onto the end of a list. Used to construct list literals.
    ListSpread { src_dst: RegId, items: RegId },
    /// Insert a key-value pair into a record. Used to construct record literals. Raises an error if
    /// the key already existed in the record.
    RecordInsert {
        src_dst: RegId,
        key: RegId,
        val: RegId,
    },
    /// Spread a record onto a record. Used to construct record literals. Any existing value for the
    /// key is overwritten.
    RecordSpread { src_dst: RegId, items: RegId },
    /// Negate a boolean.
    Not { src_dst: RegId },
    /// Do a binary operation on `lhs_dst` (left) and `rhs` (right) and write the result to
    /// `lhs_dst`.
    BinaryOp {
        lhs_dst: RegId,
        op: Operator,
        rhs: RegId,
    },
    /// Follow a cell path on the value in `src_dst`, storing the result back to `src_dst`
    FollowCellPath { src_dst: RegId, path: RegId },
    /// Clone the value at a cell path in `src`, storing the result to `dst`. The original value
    /// remains in `src`. Must be a collected value.
    CloneCellPath { dst: RegId, src: RegId, path: RegId },
    /// Update/insert a cell path to `new_value` on the value in `src_dst`, storing the modified
    /// value back to `src_dst`
    UpsertCellPath {
        src_dst: RegId,
        path: RegId,
        new_value: RegId,
    },
    /// Update/insert a cell path directly on a variable in the stack, without cloning the
    /// variable first. Combines LoadVariable + UpsertCellPath + StoreVariable into a single
    /// in-place mutation. The variable must be mutable.
    UpdateVarCellPath {
        var_id: VarId,
        cell_path: RegId,
        new_value: RegId,
    },
    /// Jump to an offset in this block
    Jump { index: usize },
    /// Branch to an offset in this block if the value of the `cond` register is a true boolean,
    /// otherwise continue execution
    BranchIf { cond: RegId, index: usize },
    /// Branch to an offset in this block if the value of the `src` register is Empty or Nothing,
    /// otherwise continue execution. The original value in `src` is preserved.
    BranchIfEmpty { src: RegId, index: usize },
    /// Match a pattern on `src`. If the pattern matches, branch to `index` after having set any
    /// variables captured by the pattern. If the pattern doesn't match, continue execution. The
    /// original value is preserved in `src` through this instruction.
    Match {
        pattern: Box<Pattern>,
        src: RegId,
        index: usize,
    },
    /// Check that a match guard is a boolean, throwing
    /// [`MatchGuardNotBool`](crate::ShellError::MatchGuardNotBool) if it isn't. Preserves `src`.
    CheckMatchGuard { src: RegId },
    /// Iterate on register `stream`, putting the next value in `dst` if present, or jumping to
    /// `end_index` if the iterator is finished
    Iterate {
        dst: RegId,
        stream: RegId,
        end_index: usize,
    },
    /// Structured exit that runs pending `finally` blocks before jumping. Used when a `try` body
    /// or `catch` completes normally, and for `break` and `continue` out of a `try`. Runs every
    /// `finally` whose region (in [`IrBlock::try_regions`]) covers this instruction but not
    /// `index`, innermost first, then jumps to `index`.
    ///
    /// `abrupt` is set for `break` and `continue`: each `finally` then sees empty as its input,
    /// as it does for `return` or an error, instead of the value the `try` produced.
    Leave { index: usize, abrupt: bool },
    /// Marks the end of an out-of-line `finally` block. Resumes the exit that entered the block:
    /// runs the next enclosing `finally`, or performs the pending jump, return or error once none
    /// remain.
    EndFinally,
    /// Return early from the block with the value in the register.
    ///
    /// Unlike `return`, this runs pending `finally` handlers first (collecting the value in that
    /// case, like the `try-collect` on the fall-through path), and flags the result as an early
    /// return. Custom command and closure calls clear that flag; only top-level file evaluation
    /// reads it, to skip `main`.
    ReturnEarly { src: RegId },
    /// Return from the block with the value in the register
    Return { src: RegId },
}

impl Instruction {
    /// Returns a value that can be formatted with [`Display`](std::fmt::Display) to show a detailed
    /// listing of the instruction.
    pub fn display<'a>(
        &'a self,
        engine_state: &'a EngineState,
        data: &'a [u8],
    ) -> FmtInstruction<'a> {
        FmtInstruction {
            engine_state,
            instruction: self,
            data,
        }
    }

    /// Get the output register, for instructions that produce some kind of immediate result.
    pub fn output_register(&self) -> Option<RegId> {
        match *self {
            Instruction::Unreachable => None,
            Instruction::LoadLiteral { dst, .. } => Some(dst),
            Instruction::LoadValue { dst, .. } => Some(dst),
            Instruction::Move { dst, .. } => Some(dst),
            Instruction::Clone { dst, .. } => Some(dst),
            Instruction::Collect { src_dst } => Some(src_dst),
            Instruction::TryCollect { src_dst } => Some(src_dst),
            Instruction::Span { src_dst } => Some(src_dst),
            Instruction::Drop { .. } => None,
            Instruction::Drain { .. } => None,
            Instruction::DrainIfEnd { .. } => None,
            Instruction::LoadVariable { dst, .. } => Some(dst),
            Instruction::StoreVariable { .. } => None,
            Instruction::DropVariable { .. } => None,
            Instruction::LoadEnv { dst, .. } => Some(dst),
            Instruction::LoadEnvOpt { dst, .. } => Some(dst),
            Instruction::StoreEnv { .. } => None,
            Instruction::PushPositional { .. } => None,
            Instruction::AppendRest { .. } => None,
            Instruction::PushFlag { .. } => None,
            Instruction::PushShortFlag { .. } => None,
            Instruction::PushNamed { .. } => None,
            Instruction::PushShortNamed { .. } => None,
            Instruction::PushParserInfo { .. } => None,
            Instruction::RedirectOut { .. } => None,
            Instruction::RedirectErr { .. } => None,
            Instruction::CheckErrRedirected { .. } => None,
            Instruction::OpenFile { .. } => None,
            Instruction::WriteFile { .. } => None,
            Instruction::CloseFile { .. } => None,
            Instruction::Call { src_dst, .. } => Some(src_dst),
            Instruction::StringAppend { src_dst, .. } => Some(src_dst),
            Instruction::GlobFrom { src_dst, .. } => Some(src_dst),
            Instruction::ListPush { src_dst, .. } => Some(src_dst),
            Instruction::ListSpread { src_dst, .. } => Some(src_dst),
            Instruction::RecordInsert { src_dst, .. } => Some(src_dst),
            Instruction::RecordSpread { src_dst, .. } => Some(src_dst),
            Instruction::Not { src_dst } => Some(src_dst),
            Instruction::BinaryOp { lhs_dst, .. } => Some(lhs_dst),
            Instruction::FollowCellPath { src_dst, .. } => Some(src_dst),
            Instruction::CloneCellPath { dst, .. } => Some(dst),
            Instruction::UpsertCellPath { src_dst, .. } => Some(src_dst),
            Instruction::UpdateVarCellPath { .. } => None,
            Instruction::Jump { .. } => None,
            Instruction::BranchIf { .. } => None,
            Instruction::BranchIfEmpty { .. } => None,
            Instruction::Match { .. } => None,
            Instruction::CheckMatchGuard { .. } => None,
            Instruction::Iterate { dst, .. } => Some(dst),
            Instruction::Leave { .. } => None,
            Instruction::EndFinally => None,
            Instruction::ReturnEarly { .. } => None,
            Instruction::Return { .. } => None,
        }
    }

    /// Returns the branch target index of the instruction if this is a branching instruction.
    pub fn branch_target(&self) -> Option<usize> {
        match self {
            Instruction::Jump { index } => Some(*index),
            Instruction::Leave { index, abrupt: _ } => Some(*index),
            Instruction::BranchIf { cond: _, index } => Some(*index),
            Instruction::BranchIfEmpty { src: _, index } => Some(*index),
            Instruction::Match {
                pattern: _,
                src: _,
                index,
            } => Some(*index),

            Instruction::Iterate {
                dst: _,
                stream: _,
                end_index,
            } => Some(*end_index),
            _ => None,
        }
    }

    /// Sets the branch target of the instruction if this is a branching instruction.
    ///
    /// Returns `Err(target_index)` if it isn't a branching instruction.
    pub fn set_branch_target(&mut self, target_index: usize) -> Result<(), usize> {
        match self {
            Instruction::Jump { index } => *index = target_index,
            Instruction::Leave { index, abrupt: _ } => *index = target_index,
            Instruction::BranchIf { cond: _, index } => *index = target_index,
            Instruction::BranchIfEmpty { src: _, index } => *index = target_index,
            Instruction::Match {
                pattern: _,
                src: _,
                index,
            } => *index = target_index,

            Instruction::Iterate {
                dst: _,
                stream: _,
                end_index,
            } => *end_index = target_index,
            _ => return Err(target_index),
        }
        Ok(())
    }

    /// Check for an interrupt before certain instructions
    pub fn check_interrupt(
        &self,
        engine_state: &EngineState,
        span: &Span,
    ) -> Result<(), ShellError> {
        match self {
            Instruction::Jump { .. } | Instruction::Leave { .. } | Instruction::Return { .. } => {
                engine_state.signals().check(span)
            }
            _ => Ok(()),
        }
    }
}

// This is to document/enforce the size of `Instruction` in bytes.
// We should try to avoid increasing the size of `Instruction`,
// and PRs that do so will have to change the number below so that it's noted in review.
const _: () = assert!(std::mem::size_of::<Instruction>() <= 24);

/// A literal value that can be embedded in an instruction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Literal {
    Bool(bool),
    Int(i64),
    Float(f64),
    Filesize(Filesize),
    Duration(i64),
    Binary(DataSlice),
    Block(BlockId),
    Closure(BlockId),
    RowCondition(BlockId),
    Range {
        start: RegId,
        step: RegId,
        end: RegId,
        inclusion: RangeInclusion,
    },
    List {
        capacity: usize,
    },
    Record {
        capacity: usize,
    },
    Filepath {
        val: DataSlice,
        no_expand: bool,
    },
    Directory {
        val: DataSlice,
        no_expand: bool,
    },
    GlobPattern {
        val: DataSlice,
        no_expand: bool,
    },
    String(DataSlice),
    RawString(DataSlice),
    CellPath(Box<CellPath>),
    Date(Box<DateTime<FixedOffset>>),
    Nothing,
    /// Represents an empty pipeline input (distinct from `Nothing` which is the `null` value).
    /// Used by `load_empty` to initialize registers with no input.
    Empty,
}

/// A redirection mode for the next call. See [`OutDest`](crate::OutDest).
///
/// This is generated by:
///
/// 1. Explicit redirection in a [`PipelineElement`](crate::ast::PipelineElement), or
/// 2. The [`pipe_redirection()`](crate::engine::Command::pipe_redirection) of the command being
///    piped into.
///
/// Not setting it uses the default, determined by [`Stack`](crate::engine::Stack).
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum RedirectMode {
    Pipe,
    PipeRaw,
    PipeSeparate,
    Value,
    Null,
    Inherit,
    Print,
    /// Use the given numbered file.
    File {
        file_num: u32,
    },
    /// Use the redirection mode requested by the caller, for a pre-return call.
    Caller,
}

/// Just a hack to allow `Arc<[u8]>` to be serialized and deserialized
mod serde_arc_u8_array {
    use serde::{Deserialize, Serialize};
    use std::sync::Arc;

    pub fn serialize<S>(data: &Arc<[u8]>, ser: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        data.as_ref().serialize(ser)
    }

    pub fn deserialize<'de, D>(de: D) -> Result<Arc<[u8]>, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let data: Vec<u8> = Deserialize::deserialize(de)?;
        Ok(data.into())
    }
}
