//! List prices of the Claude models, USD per million tokens, as published on
//! platform.claude.com/docs/en/about-claude/pricing on [`PRICES_AS_OF`]
//! (ADR-0051). Spend is an estimate computed from token counts: cache writes
//! at the 5-minute rate unless the transcript said they went to the 1-hour
//! cache, the long-prompt tier of Haiku 5.5 when the prompt exceeded 100k
//! tokens, no data-residency multiplier (the transcript does not carry it),
//! and nothing for local backends or models without a listed price.

/// The date of the price list this table copies.
pub const PRICES_AS_OF: &str = "2026-10-29";

/// USD per million tokens.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rates {
    pub input: f64,
    pub output: f64,
    pub cache_write_5m: f64,
    pub cache_write_1h: f64,
    pub cache_read: f64,
}

const fn r(
    input: f64,
    cache_write_5m: f64,
    cache_write_1h: f64,
    cache_read: f64,
    output: f64,
) -> Rates {
    Rates {
        input,
        output,
        cache_write_5m,
        cache_write_1h,
        cache_read,
    }
}

const FABLE_5_1: Rates = r(10.0, 12.5, 20.0, 0.25, 50.0);
const FABLE_5: Rates = r(10.0, 12.5, 20.0, 1.0, 50.0);
const OPUS_5_5: Rates = r(4.0, 5.0, 8.0, 0.20, 20.0);
const OPUS_4_5: Rates = r(5.0, 6.25, 10.0, 0.50, 25.0);
const OPUS_4: Rates = r(15.0, 18.75, 30.0, 1.50, 75.0);
const SONNET_5_5: Rates = r(2.0, 2.5, 4.0, 0.10, 10.0);
const SONNET_5: Rates = r(2.0, 2.5, 4.0, 0.20, 10.0);
const SONNET_4: Rates = r(3.0, 3.75, 6.0, 0.30, 15.0);
const HAIKU_5_5: Rates = r(0.10, 0.125, 0.20, 0.01, 0.50);
const HAIKU_5_5_LONG: Rates = r(0.50, 0.625, 1.0, 0.05, 2.50);
const HAIKU_4_5: Rates = r(1.0, 1.25, 2.0, 0.10, 5.0);
const HAIKU_3_5: Rates = r(0.80, 1.0, 1.60, 0.08, 4.0);
const HAIKU_3: Rates = r(0.25, 0.30, 0.50, 0.03, 1.25);

/// Model families by the normalized id (provider prefixes, region prefixes,
/// `-YYYYMMDD` / `@YYYYMMDD` release dates and `-v1:0` suffixes removed).
const TABLE: &[(&str, &Rates, Option<&Rates>)] = &[
    ("claude-fable-5-1", &FABLE_5_1, None),
    ("claude-mythos-5-1", &FABLE_5_1, None),
    ("claude-fable-5", &FABLE_5, None),
    ("claude-mythos-5", &FABLE_5, None),
    ("claude-mythos-preview", &FABLE_5, None),
    ("claude-opus-5-5", &OPUS_5_5, None),
    ("claude-opus-5", &OPUS_4_5, None),
    ("claude-opus-4-8", &OPUS_4_5, None),
    ("claude-opus-4-7", &OPUS_4_5, None),
    ("claude-opus-4-6", &OPUS_4_5, None),
    ("claude-opus-4-5", &OPUS_4_5, None),
    ("claude-opus-4-1", &OPUS_4, None),
    ("claude-opus-4-0", &OPUS_4, None),
    ("claude-opus-4", &OPUS_4, None),
    ("claude-3-opus", &OPUS_4, None),
    ("claude-sonnet-5-5", &SONNET_5_5, None),
    ("claude-sonnet-5", &SONNET_5, None),
    ("claude-sonnet-4-6", &SONNET_4, None),
    ("claude-sonnet-4-5", &SONNET_4, None),
    ("claude-sonnet-4-0", &SONNET_4, None),
    ("claude-sonnet-4", &SONNET_4, None),
    ("claude-3-7-sonnet", &SONNET_4, None),
    ("claude-3-5-sonnet", &SONNET_4, None),
    ("claude-haiku-5-5", &HAIKU_5_5, Some(&HAIKU_5_5_LONG)),
    ("claude-haiku-4-5", &HAIKU_4_5, None),
    ("claude-3-5-haiku", &HAIKU_3_5, None),
    ("claude-haiku-3-5", &HAIKU_3_5, None),
    ("claude-3-haiku", &HAIKU_3, None),
];

/// Prompts longer than this (input + cache read + cache write) are billed at
/// a model's long-prompt tier where it has one (Haiku 5.5).
pub const LONG_PROMPT_TOKENS: i64 = 100_000;

/// The family key of a model id: lowercase, provider and region prefixes
/// gone (`anthropic.`, `us.`, `eu.`, `global.`, `apac.`), release dates and
/// the Bedrock `-v1:0` suffix removed.
pub fn family(model: &str) -> String {
    let mut m = model.trim().to_ascii_lowercase();
    loop {
        let before = m.len();
        for prefix in [
            "anthropic.",
            "us.",
            "eu.",
            "global.",
            "apac.",
            "jp.",
            "au.",
            "ca.",
        ] {
            if let Some(rest) = m.strip_prefix(prefix) {
                m = rest.to_string();
            }
        }
        if m.len() == before {
            break;
        }
    }
    if let Some(i) = m.find("-v1:") {
        m.truncate(i);
    }
    if let Some(i) = m.find('@') {
        m.truncate(i);
    }
    // A trailing `-YYYYMMDD` release date.
    if m.len() > 9 {
        let (head, tail) = m.split_at(m.len() - 9);
        if tail.starts_with('-') && tail[1..].chars().all(|c| c.is_ascii_digit()) {
            m = head.to_string();
        }
    }
    m
}

/// The list rates of a model, or `None` when it is not a priced Claude model
/// (local models, `<synthetic>`, unknown ids).
pub fn rates(model: &str, long_prompt: bool) -> Option<Rates> {
    let key = family(model);
    TABLE
        .iter()
        .find(|(k, _, _)| *k == key)
        .map(|(_, base, long)| {
            if long_prompt {
                long.copied().unwrap_or(**base)
            } else {
                **base
            }
        })
}

/// Token counts to price.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Counts {
    pub input: i64,
    pub output: i64,
    pub cache_read: i64,
    pub cache_write: i64,
    /// The part of `cache_write` that went to the 1-hour cache.
    pub cache_write_1h: i64,
}

impl Counts {
    /// What the model had in front of it: the long-prompt tier applies above [`LONG_PROMPT_TOKENS`].
    pub fn prompt(&self) -> i64 {
        self.input + self.cache_read + self.cache_write
    }
}

/// USD for `counts` at `rates`.
pub fn spend(counts: &Counts, rates: &Rates) -> f64 {
    let m = 1_000_000.0;
    let w1h = counts.cache_write_1h.clamp(0, counts.cache_write.max(0));
    let w5m = counts.cache_write.max(0) - w1h;
    counts.input.max(0) as f64 / m * rates.input
        + counts.output.max(0) as f64 / m * rates.output
        + counts.cache_read.max(0) as f64 / m * rates.cache_read
        + w5m as f64 / m * rates.cache_write_5m
        + w1h as f64 / m * rates.cache_write_1h
}

/// The spend of one model's counts, `None` when the model has no price. Local
/// backends (`ollama`, `lm_studio`) are free and return `Some(0.0)`.
pub fn spend_of(source: &str, model: &str, counts: &Counts) -> Option<f64> {
    if matches!(source, "ollama" | "lm_studio") {
        return Some(0.0);
    }
    let long = counts.prompt() > LONG_PROMPT_TOKENS;
    rates(model, long).map(|r| spend(counts, &r))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_normalise_to_families() {
        assert_eq!(family("claude-sonnet-4-5-20250929"), "claude-sonnet-4-5");
        assert_eq!(family("claude-opus-4-5-20251101"), "claude-opus-4-5");
        assert_eq!(family("Claude-Fable-5-1"), "claude-fable-5-1");
        assert_eq!(
            family("anthropic.claude-opus-4-5-20251101-v1:0"),
            "claude-opus-4-5"
        );
        assert_eq!(
            family("us.anthropic.claude-haiku-4-5-20251001-v1:0"),
            "claude-haiku-4-5"
        );
        assert_eq!(family("claude-sonnet-4-5@20250929"), "claude-sonnet-4-5");
        assert_eq!(family("claude-3-5-haiku-20241022"), "claude-3-5-haiku");
        assert_eq!(family("<synthetic>"), "<synthetic>");
    }

    #[test]
    fn every_current_model_has_the_published_rates() {
        let f = rates("claude-fable-5-1", false).unwrap();
        assert_eq!(
            (
                f.input,
                f.cache_write_5m,
                f.cache_write_1h,
                f.cache_read,
                f.output
            ),
            (10.0, 12.5, 20.0, 0.25, 50.0)
        );
        let o = rates("claude-opus-5-5", false).unwrap();
        assert_eq!((o.input, o.cache_read, o.output), (4.0, 0.2, 20.0));
        let s = rates("claude-sonnet-5-5", false).unwrap();
        assert_eq!((s.input, s.cache_read, s.output), (2.0, 0.1, 10.0));
        let h = rates("claude-haiku-5-5", false).unwrap();
        assert_eq!((h.input, h.output), (0.1, 0.5));
        let hl = rates("claude-haiku-5-5", true).unwrap();
        assert_eq!((hl.input, hl.output), (0.5, 2.5));
        // Models without a long-prompt tier keep their base rates.
        assert_eq!(
            rates("claude-opus-4-5-20251101", true),
            rates("claude-opus-4-5", false)
        );
        assert!(rates("llama3.1:8b", false).is_none());
        assert!(rates("<synthetic>", false).is_none());
    }

    #[test]
    fn spend_prices_every_token_kind_at_its_rate() {
        // 1M of each kind on Opus 5.5: 4 + 20 + 0.2 + 5 (5-minute writes).
        let c = Counts {
            input: 1_000_000,
            output: 1_000_000,
            cache_read: 1_000_000,
            cache_write: 1_000_000,
            cache_write_1h: 0,
        };
        assert!((spend(&c, &OPUS_5_5) - 29.2).abs() < 1e-9);
        // Half the writes to the 1-hour cache: 0.5 × 5 + 0.5 × 8.
        let c1 = Counts {
            cache_write_1h: 500_000,
            ..c
        };
        assert!((spend(&c1, &OPUS_5_5) - 30.7).abs() < 1e-9);
        // A 1-hour share above the writes themselves is clamped.
        let c2 = Counts {
            cache_write_1h: 5_000_000,
            ..c
        };
        assert!((spend(&c2, &OPUS_5_5) - 32.2).abs() < 1e-9);
        // The usual Claude Code shape: a few input tokens, mostly cache reads.
        let c3 = Counts {
            input: 2,
            output: 682,
            cache_read: 48_171,
            cache_write: 20_297,
            cache_write_1h: 20_297,
        };
        let s = spend_of("claude_code", "claude-fable-5-1", &c3).unwrap();
        let expect =
            2.0 / 1e6 * 10.0 + 682.0 / 1e6 * 50.0 + 48_171.0 / 1e6 * 0.25 + 20_297.0 / 1e6 * 20.0;
        assert!((s - expect).abs() < 1e-9);
    }

    #[test]
    fn local_backends_are_free_and_unknown_models_unpriced() {
        let c = Counts {
            input: 10,
            output: 10,
            ..Counts::default()
        };
        assert_eq!(spend_of("ollama", "llama3.1:8b", &c), Some(0.0));
        assert_eq!(spend_of("lm_studio", "qwen", &c), Some(0.0));
        assert_eq!(spend_of("claude_code", "<synthetic>", &c), None);
        assert_eq!(
            spend_of("claude_api", "claude-opus-5-5", &Counts::default()),
            Some(0.0)
        );
    }

    #[test]
    fn haiku_5_5_switches_tier_on_long_prompts() {
        let short = Counts {
            input: 50_000,
            output: 1_000,
            ..Counts::default()
        };
        let long = Counts {
            input: 50_000,
            cache_read: 60_000,
            output: 1_000,
            ..Counts::default()
        };
        let s = spend_of("claude_code", "claude-haiku-5-5", &short).unwrap();
        let l = spend_of("claude_code", "claude-haiku-5-5", &long).unwrap();
        assert!((s - (0.05 * 0.1 + 0.001 * 0.5)).abs() < 1e-9);
        assert!((l - (0.05 * 0.5 + 0.06 * 0.05 + 0.001 * 2.5)).abs() < 1e-9);
    }
}
