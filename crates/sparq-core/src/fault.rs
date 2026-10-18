//! Test-only fault injection for the storage steps that publish, mark, rename and remove
//! (compiled into `cfg(test)` builds only, never into a library build). A test arms a plan on
//! its own thread; every instrumented step asks [`point`] first, and fails when the plan says
//! so:
//!
//! - [`fail_when`]: the first matching step fails, later steps run (an I/O error the code
//!   is meant to roll back from);
//! - [`fail_every`]: every matching step fails (a step that keeps failing);
//! - [`crash_at`]: the first matching step fails and so does every later step, matching or
//!   not (the process died there: what is on disk is what a crash would leave);
//! - [`fail_then_crash`]: the first matching step fails, the next `after` steps run, and every
//!   step from then on fails (an error, then a crash part-way through its rollback);
//! - [`panic_at`]: the first matching step panics;
//! - [`observe`]: nothing fails; a callback sees every step before it runs.
//!
//! The steps: `rename` (a publishing rename; the path is its source), `mark` and `clear` (a
//! staging record written or removed; the path is the directory), `create` (a staging
//! directory about to be made; its staging path), `remove-entry` (one entry of a staging
//! directory being removed), `remove-record` and `remove-dir` (the record, then the directory,
//! last; the path is the staging directory).

use std::cell::RefCell;
use std::path::Path;

type Pred = Box<dyn FnMut(&str, &Path) -> bool>;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Once,
    Every,
    Crash,
    FailThenCrash(usize),
    Panic,
    // Only the Unix-only swap tests observe.
    #[cfg_attr(windows, allow(dead_code))]
    Observe,
}

struct Plan {
    pred: Pred,
    mode: Mode,
    fired: usize,
    /// For [`Mode::FailThenCrash`]: steps still to run before the crash, once the first fired.
    countdown: Option<usize>,
}

thread_local! {
    static PLAN: RefCell<Option<Plan>> = const { RefCell::new(None) };
}

fn arm(mode: Mode, pred: impl FnMut(&str, &Path) -> bool + 'static) {
    PLAN.with(|p| {
        *p.borrow_mut() = Some(Plan {
            pred: Box::new(pred),
            mode,
            fired: 0,
            countdown: None,
        })
    });
}

/// The first step matching `pred` fails; the rest run.
pub(crate) fn fail_when(pred: impl FnMut(&str, &Path) -> bool + 'static) {
    arm(Mode::Once, pred);
}

/// Every step matching `pred` fails.
pub(crate) fn fail_every(pred: impl FnMut(&str, &Path) -> bool + 'static) {
    arm(Mode::Every, pred);
}

/// The first step matching `pred` fails, and every step after it.
pub(crate) fn crash_at(pred: impl FnMut(&str, &Path) -> bool + 'static) {
    arm(Mode::Crash, pred);
}

/// The first step matching `pred` fails; the `after` steps after it run; every step after
/// those fails.
pub(crate) fn fail_then_crash(after: usize, pred: impl FnMut(&str, &Path) -> bool + 'static) {
    arm(Mode::FailThenCrash(after), pred);
}

/// The first step matching `pred` panics.
pub(crate) fn panic_at(pred: impl FnMut(&str, &Path) -> bool + 'static) {
    arm(Mode::Panic, pred);
}

/// `f` sees every step before it runs; nothing fails.
#[cfg_attr(windows, allow(dead_code))]
pub(crate) fn observe(mut f: impl FnMut(&str, &Path) + 'static) {
    arm(Mode::Observe, move |step, path| {
        f(step, path);
        false
    });
}

/// Disarms the plan, returning how many steps it made fail.
pub(crate) fn disarm() -> usize {
    PLAN.with(|p| p.borrow_mut().take().map_or(0, |plan| plan.fired))
}

/// The step `step` on `path` is about to run: `Err` when the plan makes it fail.
pub(crate) fn point(step: &str, path: &Path) -> std::io::Result<()> {
    let panic = PLAN.with(|p| -> std::io::Result<bool> {
        let mut p = p.borrow_mut();
        let Some(plan) = p.as_mut() else {
            return Ok(false);
        };
        let fail = |plan: &mut Plan| {
            plan.fired += 1;
            Err(std::io::Error::other(format!(
                "injected failure at {step} {}",
                path.display()
            )))
        };
        match plan.mode {
            Mode::Crash if plan.fired > 0 => return fail(plan),
            Mode::FailThenCrash(_) if plan.fired > 0 => {
                return match plan.countdown {
                    Some(0) => fail(plan),
                    Some(n) => {
                        plan.countdown = Some(n - 1);
                        Ok(false)
                    }
                    None => Ok(false),
                };
            }
            _ => {}
        }
        let eligible = matches!(plan.mode, Mode::Every | Mode::Observe) || plan.fired == 0;
        if eligible && (plan.pred)(step, path) {
            match plan.mode {
                Mode::Panic => {
                    plan.fired += 1;
                    return Ok(true);
                }
                Mode::FailThenCrash(after) => plan.countdown = Some(after),
                _ => {}
            }
            return fail(plan);
        }
        Ok(false)
    })?;
    if panic {
        panic!("injected panic at {step} {}", path.display());
    }
    Ok(())
}
