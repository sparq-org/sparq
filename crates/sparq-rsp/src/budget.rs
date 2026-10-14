//! sq-xqu: per-registered-query budget plumbing — the
//! [`QueryBudget`] a continuous query applies to EVERY window evaluation,
//! plus the optional per-window timeout that tightens the budget's deadline
//! at each evaluation start (a stored `deadline` is an ABSOLUTE instant, which
//! is the wrong shape for a long-lived registered query).

use sparq_engine::QueryBudget;

/// The registered budget configuration shared by all continuous-query forms.
///
/// `base` is applied to every window evaluation as-is: the row / byte caps and
/// the cancellation flag are naturally per-evaluation limits, while a
/// `deadline` inside it stays absolute (once it passes, every later window
/// fails). `per_window_timeout` is the relative form embedders actually want:
/// at each window evaluation start the effective deadline becomes the EARLIER
/// of the absolute deadline and `now + timeout`, bounding the worst-case cost
/// of ONE window without ever extending the absolute limit.
#[derive(Debug, Clone, Default)]
pub(crate) struct BudgetSpec {
    pub base: QueryBudget,
    #[cfg(not(target_arch = "wasm32"))]
    pub per_window_timeout: Option<std::time::Duration>,
}

impl BudgetSpec {
    /// The effective [`QueryBudget`] for one window evaluation: `base`, with
    /// the deadline tightened by `per_window_timeout` (measured from NOW, the
    /// moment this window's evaluation starts). The two deadline forms
    /// COMPOSE: the effective deadline is the EARLIER of `base`'s absolute
    /// deadline and `now + timeout`, so a refreshed window deadline can never
    /// grant time past the absolute one. A timeout so large that
    /// `now + timeout` is unrepresentable can never trip, so it is treated as
    /// unlimited: `base`'s own absolute deadline (if any) is kept instead of
    /// panicking on `Instant` overflow.
    pub fn window_budget(&self) -> QueryBudget {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(t) = self.per_window_timeout {
            let mut b = self.base.clone();
            if let Some(window_deadline) = std::time::Instant::now().checked_add(t) {
                b.deadline = Some(match b.deadline {
                    Some(absolute) => absolute.min(window_deadline),
                    None => window_deadline,
                });
            }
            return b;
        }
        self.base.clone()
    }
}
