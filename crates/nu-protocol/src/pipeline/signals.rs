use crate::{ShellError, Span};
use nu_glob::Interruptible;
use serde::{Deserialize, Serialize};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

/// Used to check for signals to suspend or terminate the execution of Nushell code.
///
/// A `Signals` can hold a primary source (the engine-wide Ctrl-C flag) and any
/// number of *additional* sources chained via [`Signals::chain`]. An interrupt
/// is observed when any source flag is set; this lets a parent's Ctrl-C and a
/// scoped local cancellation flag (e.g. one owned by `interleave`) coexist
/// without the local flag cascading back into the parent.
///
/// The struct is intentionally pointer-sized: `Value::Range` and `Value::List`
/// embed `Option<Signals>` inline, so growing this type would grow `Value`.
#[derive(Debug, Clone)]
pub struct Signals {
    inner: Option<Arc<SignalsInner>>,
}

#[derive(Debug)]
struct SignalsInner {
    primary: Option<Arc<AtomicBool>>,
    chained: Vec<Arc<AtomicBool>>,
}

impl Signals {
    /// A [`Signals`] that is not hooked up to any event/signals source.
    ///
    /// So, this [`Signals`] will never be interrupted.
    pub const EMPTY: Self = Signals { inner: None };

    /// Create a new [`Signals`] with `ctrlc` as the interrupt source.
    ///
    /// Once `ctrlc` is set to `true`, [`check`](Self::check) will error
    /// and [`interrupted`](Self::interrupted) will return `true`.
    pub fn new(ctrlc: Arc<AtomicBool>) -> Self {
        Self {
            inner: Some(Arc::new(SignalsInner {
                primary: Some(ctrlc),
                chained: Vec::new(),
            })),
        }
    }

    /// Create a [`Signals`] that is not hooked up to any event/signals source.
    ///
    /// So, the returned [`Signals`] will never be interrupted.
    ///
    /// This should only be used in test code, or if the stream/iterator being created
    /// already has an underlying [`Signals`].
    pub const fn empty() -> Self {
        Self::EMPTY
    }

    /// Returns a [`Signals`] that observes `parent` and `local` together.
    ///
    /// [`interrupted`](Self::interrupted) returns `true` if either source is
    /// triggered. [`trigger`](Self::trigger) writes only to `local`, never to
    /// `parent` -- so a scope can cancel itself without cascading into the
    /// engine-wide Ctrl-C state.
    pub fn chain(parent: Signals, local: Arc<AtomicBool>) -> Signals {
        let (primary, mut chained) = match parent.inner {
            Some(inner) => {
                let SignalsInner { primary, chained } = (*inner).clone_fields();
                (primary, chained)
            }
            None => (None, Vec::new()),
        };
        chained.push(local);
        Signals {
            inner: Some(Arc::new(SignalsInner { primary, chained })),
        }
    }

    /// Returns an `Err` if an interrupt has been triggered.
    ///
    /// Otherwise, returns `Ok`.
    #[inline]
    pub fn check(&self, span: &Span) -> Result<(), ShellError> {
        #[inline]
        #[cold]
        fn interrupt_error(span: &Span) -> Result<(), ShellError> {
            Err(ShellError::Interrupted { span: *span })
        }

        if self.interrupted() {
            interrupt_error(span)
        } else {
            Ok(())
        }
    }

    /// Triggers an interrupt.
    ///
    /// If this `Signals` was created via [`Signals::chain`], only the most
    /// recently chained local flag is written to; the parent flag is left
    /// untouched. This prevents a scoped cancellation from cascading into the
    /// engine-wide Ctrl-C state.
    pub fn trigger(&self) {
        let Some(inner) = &self.inner else {
            return;
        };
        if let Some(local) = inner.chained.last() {
            local.store(true, Ordering::Relaxed);
        } else if let Some(primary) = &inner.primary {
            primary.store(true, Ordering::Relaxed);
        }
    }

    /// Returns whether an interrupt has been triggered.
    #[inline]
    pub fn interrupted(&self) -> bool {
        let Some(inner) = &self.inner else {
            return false;
        };
        inner
            .primary
            .as_deref()
            .is_some_and(|b| b.load(Ordering::Relaxed))
            || inner.chained.iter().any(|b| b.load(Ordering::Relaxed))
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.inner.is_none()
    }

    pub fn reset(&self) {
        let Some(inner) = &self.inner else {
            return;
        };
        if let Some(primary) = &inner.primary {
            primary.store(false, Ordering::Relaxed);
        }
        for local in &inner.chained {
            local.store(false, Ordering::Relaxed);
        }
    }
}

impl SignalsInner {
    fn clone_fields(&self) -> Self {
        SignalsInner {
            primary: self.primary.clone(),
            chained: self.chained.clone(),
        }
    }
}

impl Interruptible for Signals {
    #[inline]
    fn interrupted(&self) -> bool {
        self.interrupted()
    }
}

/// The types of things that can be signaled. It's anticipated this will change as we learn more
/// about how we'd like signals to be handled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SignalAction {
    Interrupt,
    Reset,
}
