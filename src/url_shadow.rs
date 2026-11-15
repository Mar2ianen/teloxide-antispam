//! Shadow-only URL evidence interpretation.
//!
//! This module deliberately does not feed `analyze_risk`, captcha, or ban
//! decisions. Consumers can persist the breakdown and calibrate it against
//! reviewed labels before considering a policy change.

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UrlShadowEvidence {
    /// Number of parsed URLs outside the current chat and its configured
    /// Telegram-affiliated channels.
    pub external_url_count: u16,
    pub affiliated_url_count: u16,
    /// URLs extracted from a QR code and classified as external.
    pub external_qr_url_count: u16,
    /// Exact/hash-based hit from a fresh, locally loaded threat feed.
    pub verified_threat_feed_hit: bool,
    /// Independently computed promotion/solicitation evidence.
    pub promotional_content: bool,
    /// An exact Telegram user ID link whose identity was locally verified as
    /// a confirmed spammer. Usernames alone must never set this field.
    pub confirmed_spammer_user_target: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct UrlShadowSignal {
    pub label: &'static str,
    pub coefficient: i16,
    pub explanation: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct UrlShadowAnalysis {
    /// Advisory subtotal only. It is not a probability or production risk
    /// score and is not bounded to the existing anti-spam score scale.
    pub suggested_delta: i16,
    pub signals: Vec<UrlShadowSignal>,
}

pub fn analyze_url_shadow(evidence: UrlShadowEvidence) -> UrlShadowAnalysis {
    let mut signals = Vec::new();

    if evidence.verified_threat_feed_hit {
        signals.push(UrlShadowSignal {
            label: "verified_url_threat_feed_hit",
            coefficient: 24,
            explanation: "A parsed URL exactly matched a fresh local threat-feed snapshot.",
        });
    }
    if evidence.confirmed_spammer_user_target {
        signals.push(UrlShadowSignal {
            label: "confirmed_spammer_user_target",
            coefficient: 16,
            explanation: "A numeric Telegram user target matches a locally confirmed spammer.",
        });
    }
    if evidence.promotional_content && evidence.external_url_count > 0 {
        signals.push(UrlShadowSignal {
            label: "promotion_with_external_url",
            coefficient: 8,
            explanation: "Promotion evidence co-occurs with at least one non-affiliated URL.",
        });
    }
    if evidence.promotional_content && evidence.external_qr_url_count > 0 {
        signals.push(UrlShadowSignal {
            label: "promotion_with_external_qr_url",
            coefficient: 8,
            explanation: "Promotion evidence co-occurs with an external URL encoded in a QR.",
        });
    }

    // Affiliated references are retained for evaluation, but do not subtract
    // from or increase the shadow score. This avoids treating configuration
    // omissions as proof of malicious intent.
    let suggested_delta = signals.iter().map(|signal| signal.coefficient).sum();
    UrlShadowAnalysis {
        suggested_delta,
        signals,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_only_and_affiliated_links_do_not_add_shadow_risk() {
        let analysis = analyze_url_shadow(UrlShadowEvidence {
            external_url_count: 2,
            affiliated_url_count: 1,
            ..UrlShadowEvidence::default()
        });
        assert_eq!(analysis.suggested_delta, 0);
        assert!(analysis.signals.is_empty());
    }

    #[test]
    fn promotion_and_external_qr_url_are_explained_separately() {
        let analysis = analyze_url_shadow(UrlShadowEvidence {
            external_qr_url_count: 1,
            promotional_content: true,
            ..UrlShadowEvidence::default()
        });
        assert_eq!(analysis.suggested_delta, 8);
        assert_eq!(analysis.signals[0].label, "promotion_with_external_qr_url");
    }

    #[test]
    fn fresh_feed_hit_is_supporting_evidence_not_a_policy_decision() {
        let analysis = analyze_url_shadow(UrlShadowEvidence {
            verified_threat_feed_hit: true,
            ..UrlShadowEvidence::default()
        });
        assert_eq!(analysis.suggested_delta, 24);
        assert_eq!(analysis.signals.len(), 1);
        assert_eq!(analysis.signals[0].label, "verified_url_threat_feed_hit");
    }
}
