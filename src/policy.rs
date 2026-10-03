//! Pure score-based moderation policy.
//!
//! Two steps: message deletion at the review threshold and a ban at the
//! higher threshold. This module only returns a proposed action. Evidence
//! persistence, authorization, dry-run, and idempotent execution belong to
//! the consumer; calling it does not perform any Telegram action.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AutoPolicy {
    /// Review threshold chosen by the consumer.
    pub review_threshold: i32,
    /// Ban and message-deletion threshold.
    pub ban_threshold: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoAction {
    /// Below the review threshold: do nothing.
    None,
    /// At or above review, below ban: propose first-message deletion.
    /// The consumer handles review delivery separately.
    DeleteMessages,
    /// At or above ban: propose a ban and recent-message deletion.
    Ban,
}

pub fn decide_auto_action(score: i32, policy: &AutoPolicy) -> AutoAction {
    if score >= policy.ban_threshold {
        AutoAction::Ban
    } else if score >= policy.review_threshold {
        AutoAction::DeleteMessages
    } else {
        AutoAction::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy() -> AutoPolicy {
        AutoPolicy {
            review_threshold: 70,
            ban_threshold: 90,
        }
    }

    #[test]
    fn low_score_takes_no_action() {
        assert_eq!(decide_auto_action(69, &policy()), AutoAction::None);
    }

    #[test]
    fn review_band_deletes_messages() {
        assert_eq!(
            decide_auto_action(70, &policy()),
            AutoAction::DeleteMessages
        );
        assert_eq!(
            decide_auto_action(89, &policy()),
            AutoAction::DeleteMessages
        );
    }

    #[test]
    fn ban_threshold_bans() {
        assert_eq!(decide_auto_action(90, &policy()), AutoAction::Ban);
        assert_eq!(decide_auto_action(100, &policy()), AutoAction::Ban);
    }
}
