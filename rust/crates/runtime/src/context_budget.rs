//! Context budget management for session context windows.
//!
//! Provides utilities for estimating the current context size, deciding
//! whether compaction is needed, and selecting an appropriate compaction
//! strategy based on usage thresholds.

use crate::compact::estimate_session_tokens;
use crate::session::Session;

/// Budget tracking struct that encapsulates context-window capacity and
/// current utilisation.
#[derive(Debug, Clone, PartialEq)]
pub struct ContextBudget {
    /// Maximum tokens the model context window can hold.
    pub max_tokens: u32,
    /// Fraction of `max_tokens` at which a warning is raised (0.0 – 1.0).
    pub warning_threshold: f64,
    /// Estimated current token count.
    pub current_tokens: u32,
    /// Number of messages in the session.
    pub messages_count: usize,
}

impl Default for ContextBudget {
    fn default() -> Self {
        Self {
            max_tokens: 1_000_000,
            warning_threshold: 0.8,
            current_tokens: 0,
            messages_count: 0,
        }
    }
}

/// Strategy describing what compaction action to take.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompactionStrategy {
    /// No compaction is needed yet.
    None,
    /// Summarise the oldest messages into a compact summary.
    SummaryOldest,
    /// Rewind the session by removing the given number of messages.
    Rewind(u32),
}

/// Estimates the current context budget from a session snapshot.
pub fn estimate_context_size(session: &Session) -> Result<ContextBudget, BudgetError> {
    let estimated = estimate_session_tokens(session);
    Ok(ContextBudget {
        current_tokens: estimated as u32,
        messages_count: session.messages.len(),
        ..ContextBudget::default()
    })
}

/// Returns `true` when `current_tokens` exceeds `warning_threshold * max_tokens`.
#[must_use]
pub fn should_compact(budget: &ContextBudget) -> bool {
    let threshold = (budget.max_tokens as f64 * budget.warning_threshold) as u32;
    budget.current_tokens >= threshold
}

/// Selects a compaction strategy based on the current budget utilisation.
#[must_use]
pub fn get_compaction_strategy(budget: &ContextBudget) -> CompactionStrategy {
    let ratio = budget.current_tokens as f64 / budget.max_tokens as f64;

    if ratio < budget.warning_threshold {
        return CompactionStrategy::None;
    }

    // When usage is ≥ 95%, be aggressive: rewind messages.
    if ratio >= 0.95 {
        let rewind_count = (budget.messages_count as f64 * 0.5).ceil() as u32;
        return CompactionStrategy::Rewind(rewind_count.max(1));
    }

    // When usage is between warning_threshold and 95%, summarise.
    CompactionStrategy::SummaryOldest
}

/// Formats a human-readable budget summary string.
///
/// # Example
/// ```
/// # use runtime::context_budget::{ContextBudget, format_budget_summary};
/// let budget = ContextBudget {
///     current_tokens: 450_000,
///     max_tokens: 1_000_000,
///     ..ContextBudget::default()
/// };
/// assert_eq!(format_budget_summary(&budget), "上下文: 450K/1M (45%)");
/// ```
#[must_use]
pub fn format_budget_summary(budget: &ContextBudget) -> String {
    fn format_token_count(tokens: u32) -> String {
        if tokens >= 1_000_000 && tokens % 1_000_000 == 0 {
            format!("{}M", tokens / 1_000_000)
        } else if tokens >= 1_000_000 {
            format!("{:.1}M", tokens as f64 / 1_000_000.0)
        } else if tokens >= 1_000 {
            format!("{}K", tokens / 1000)
        } else {
            format!("{}", tokens)
        }
    }
    let max_str = format_token_count(budget.max_tokens);
    let current_str = format_token_count(budget.current_tokens);
    let pct = (budget.current_tokens as f64 / budget.max_tokens as f64 * 100.0).round() as u32;
    format!("上下文: {current_str}/{max_str} ({pct}%)")
}

/// Errors that can occur during budget estimation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BudgetError {
    /// The session is empty or invalid.
    EmptySession,
}

impl std::fmt::Display for BudgetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BudgetError::EmptySession => write!(f, "session is empty"),
        }
    }
}

impl std::error::Error for BudgetError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{ContentBlock, ConversationMessage, MessageRole, Session};

    // ── ContextBudget defaults ──────────────────────────────────────────────

    #[test]
    fn test_default_budget() {
        let budget = ContextBudget::default();
        assert_eq!(budget.max_tokens, 1_000_000);
        assert!((budget.warning_threshold - 0.8).abs() < f64::EPSILON);
        assert_eq!(budget.current_tokens, 0);
        assert_eq!(budget.messages_count, 0);
    }

    // ── estimate_context_size ───────────────────────────────────────────────

    #[test]
    fn test_estimate_context_size_empty_session() {
        let session = Session::new();
        let budget = estimate_context_size(&session).unwrap();
        assert_eq!(budget.current_tokens, 0);
        assert_eq!(budget.messages_count, 0);
    }

    #[test]
    fn test_estimate_context_size_with_messages() {
        let mut session = Session::new();
        session.messages = vec![
            ConversationMessage::user_text("hello world"),
            ConversationMessage::assistant(vec![ContentBlock::Text {
                text: "hi there".to_string(),
            }]),
        ];
        let budget = estimate_context_size(&session).unwrap();
        // "hello world" -> 11/4+1 ≈ 3, "hi there" -> 8/4+1 = 3, total ≈ 6
        assert!(budget.current_tokens > 0);
        assert_eq!(budget.messages_count, 2);
        assert_eq!(budget.max_tokens, 1_000_000); // uses default
    }

    #[test]
    fn test_estimate_context_size_preserves_defaults() {
        let session = Session::new();
        let budget = estimate_context_size(&session).unwrap();
        assert_eq!(budget.max_tokens, 1_000_000);
        assert!((budget.warning_threshold - 0.8).abs() < f64::EPSILON);
    }

    // ── should_compact ──────────────────────────────────────────────────────

    #[test]
    fn test_should_compact_below_threshold() {
        let budget = ContextBudget {
            max_tokens: 1_000_000,
            warning_threshold: 0.8,
            current_tokens: 700_000,
            messages_count: 10,
        };
        assert!(!should_compact(&budget));
    }

    #[test]
    fn test_should_compact_at_threshold() {
        let budget = ContextBudget {
            max_tokens: 1_000_000,
            warning_threshold: 0.8,
            current_tokens: 800_000,
            messages_count: 10,
        };
        assert!(should_compact(&budget));
    }

    #[test]
    fn test_should_compact_above_threshold() {
        let budget = ContextBudget {
            max_tokens: 1_000_000,
            warning_threshold: 0.8,
            current_tokens: 950_000,
            messages_count: 10,
        };
        assert!(should_compact(&budget));
    }

    #[test]
    fn test_should_compact_zero_current() {
        let budget = ContextBudget {
            max_tokens: 1_000_000,
            warning_threshold: 0.8,
            current_tokens: 0,
            messages_count: 0,
        };
        assert!(!should_compact(&budget));
    }

    #[test]
    fn test_should_compact_custom_threshold() {
        let budget = ContextBudget {
            max_tokens: 10_000,
            warning_threshold: 0.5,
            current_tokens: 5_000,
            messages_count: 5,
        };
        assert!(should_compact(&budget));
    }

    #[test]
    fn test_should_compact_just_below_custom_threshold() {
        let budget = ContextBudget {
            max_tokens: 10_000,
            warning_threshold: 0.5,
            current_tokens: 4_999,
            messages_count: 5,
        };
        assert!(!should_compact(&budget));
    }

    // ── get_compaction_strategy ─────────────────────────────────────────────

    #[test]
    fn test_strategy_none_when_below_threshold() {
        let budget = ContextBudget {
            max_tokens: 1_000_000,
            warning_threshold: 0.8,
            current_tokens: 500_000,
            messages_count: 10,
        };
        assert_eq!(get_compaction_strategy(&budget), CompactionStrategy::None);
    }

    #[test]
    fn test_strategy_summary_oldest_when_moderately_over_threshold() {
        let budget = ContextBudget {
            max_tokens: 1_000_000,
            warning_threshold: 0.8,
            current_tokens: 850_000,
            messages_count: 10,
        };
        assert_eq!(
            get_compaction_strategy(&budget),
            CompactionStrategy::SummaryOldest
        );
    }

    #[test]
    fn test_strategy_summary_oldest_at_94_pct() {
        let budget = ContextBudget {
            max_tokens: 1_000_000,
            warning_threshold: 0.8,
            current_tokens: 940_000,
            messages_count: 10,
        };
        assert_eq!(
            get_compaction_strategy(&budget),
            CompactionStrategy::SummaryOldest
        );
    }

    #[test]
    fn test_strategy_rewind_when_95_pct_or_above() {
        let budget = ContextBudget {
            max_tokens: 1_000_000,
            warning_threshold: 0.8,
            current_tokens: 950_000,
            messages_count: 10,
        };
        assert_eq!(
            get_compaction_strategy(&budget),
            CompactionStrategy::Rewind(5) // 50% of 10
        );
    }

    #[test]
    fn test_strategy_rewind_at_100_pct() {
        let budget = ContextBudget {
            max_tokens: 1_000_000,
            warning_threshold: 0.8,
            current_tokens: 1_000_000,
            messages_count: 20,
        };
        assert_eq!(
            get_compaction_strategy(&budget),
            CompactionStrategy::Rewind(10) // 50% of 20
        );
    }

    #[test]
    fn test_strategy_rewind_min_one_message() {
        let budget = ContextBudget {
            max_tokens: 1_000,
            warning_threshold: 0.5,
            current_tokens: 1_000,
            messages_count: 1,
        };
        assert_eq!(
            get_compaction_strategy(&budget),
            CompactionStrategy::Rewind(1)
        );
    }

    #[test]
    fn test_strategy_rewind_rounds_up() {
        let budget = ContextBudget {
            max_tokens: 1_000,
            warning_threshold: 0.5,
            current_tokens: 1_000,
            messages_count: 3,
        };
        assert_eq!(
            get_compaction_strategy(&budget),
            CompactionStrategy::Rewind(2) // ceil(3*0.5) = 2
        );
    }

    // ── format_budget_summary ───────────────────────────────────────────────

    #[test]
    fn test_format_budget_summary_exact() {
        let budget = ContextBudget {
            current_tokens: 450_000,
            max_tokens: 1_000_000,
            ..ContextBudget::default()
        };
        assert_eq!(format_budget_summary(&budget), "上下文: 450K/1M (45%)");
    }

    #[test]
    fn test_format_budget_summary_small() {
        let budget = ContextBudget {
            current_tokens: 50,
            max_tokens: 1_000,
            ..ContextBudget::default()
        };
        assert_eq!(format_budget_summary(&budget), "上下文: 50/1K (5%)");
    }

    #[test]
    fn test_format_budget_summary_hundred_pct() {
        let budget = ContextBudget {
            current_tokens: 1_000_000,
            max_tokens: 1_000_000,
            ..ContextBudget::default()
        };
        assert_eq!(format_budget_summary(&budget), "上下文: 1M/1M (100%)");
    }

    #[test]
    fn test_format_budget_summary_zero() {
        let budget = ContextBudget {
            current_tokens: 0,
            max_tokens: 1_000_000,
            ..ContextBudget::default()
        };
        assert_eq!(format_budget_summary(&budget), "上下文: 0/1M (0%)");
    }

    #[test]
    fn test_format_budget_summary_rounding() {
        let budget = ContextBudget {
            current_tokens: 123_456,
            max_tokens: 500_000,
            ..ContextBudget::default()
        };
        assert_eq!(format_budget_summary(&budget), "上下文: 123K/500K (25%)");
    }

    // ── BudgetError ─────────────────────────────────────────────────────────

    #[test]
    fn test_budget_error_display() {
        let err = BudgetError::EmptySession;
        assert_eq!(format!("{err}"), "session is empty");
    }
}
