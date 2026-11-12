//! API prices per model, from models.dev, kept in `prices.json` and renewed once a day. They
//! turn token counts into what the same usage would have cost on the API.

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::stats::{CACHE_READ, CACHE_WRITE_1H, CACHE_WRITE_5M, INPUT, OUTPUT, Tokens};

const SOURCE: &str = "https://models.dev/api.json";
const FILE: &str = "prices.json";
/// Seconds a price list is used before it is fetched again.
const FRESH_FOR: i64 = 86_400;
/// The catalog is a few megabytes and growing.
const MAX_BYTES: u64 = 64 << 20;

/// Dollars per million tokens.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Rate {
    pub input: f64,
    pub output: f64,
    #[serde(default)]
    pub cache_read: Option<f64>,
    /// Writing a five minute cache entry.
    #[serde(default)]
    pub cache_write: Option<f64>,
    /// Prices for requests whose prompt is larger than a tier's size.
    #[serde(default)]
    pub tiers: Vec<Tier>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Tier {
    pub size: u64,
    pub input: f64,
    pub output: f64,
    #[serde(default)]
    pub cache_read: Option<f64>,
    #[serde(default)]
    pub cache_write: Option<f64>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Prices {
    /// Unix seconds.
    #[serde(default)]
    pub fetched_at: i64,
    #[serde(default)]
    pub models: HashMap<String, Rate>,
}

/// The part of models.dev's catalog that is read.
#[derive(Deserialize)]
struct Catalog {
    anthropic: CatalogProvider,
}

#[derive(Deserialize)]
struct CatalogProvider {
    models: HashMap<String, CatalogModel>,
}

#[derive(Deserialize)]
struct CatalogModel {
    cost: Option<CatalogCost>,
}

#[derive(Deserialize)]
struct CatalogCost {
    input: Option<f64>,
    output: Option<f64>,
    cache_read: Option<f64>,
    cache_write: Option<f64>,
    #[serde(default)]
    tiers: Vec<CatalogTier>,
}

#[derive(Deserialize)]
struct CatalogTier {
    input: Option<f64>,
    output: Option<f64>,
    cache_read: Option<f64>,
    cache_write: Option<f64>,
    tier: Option<CatalogTierKind>,
}

#[derive(Deserialize)]
struct CatalogTierKind {
    #[serde(rename = "type")]
    kind: String,
    size: Option<u64>,
}

impl Prices {
    pub fn load(dir: &Path) -> Prices {
        std::fs::read_to_string(dir.join(FILE)).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
    }

    pub fn stale(&self, now: i64) -> bool {
        now - self.fetched_at >= FRESH_FOR
    }

    /// Fetches the current list and saves it in `dir`.
    pub fn fetch(agent: &ureq::Agent, dir: &Path, now: i64) -> Result<Prices, String> {
        let mut response = agent
            .get(SOURCE)
            .header("User-Agent", concat!("nerda-bar/", env!("CARGO_PKG_VERSION")))
            .call()
            .map_err(|e| e.to_string())?;
        if response.status().as_u16() != 200 {
            return Err(format!("HTTP {}", response.status().as_u16()));
        }
        let body = response.body_mut().with_config().limit(MAX_BYTES).read_to_vec().map_err(|e| e.to_string())?;
        let prices = Prices::from_catalog(&body, now)?;
        if std::fs::create_dir_all(dir).is_ok()
            && let Ok(text) = serde_json::to_string(&prices)
        {
            let _ = std::fs::write(dir.join(FILE), text);
        }
        Ok(prices)
    }

    fn from_catalog(body: &[u8], now: i64) -> Result<Prices, String> {
        let catalog: Catalog = serde_json::from_slice(body).map_err(|e| e.to_string())?;
        let mut models = HashMap::new();
        for (id, model) in catalog.anthropic.models {
            let Some(cost) = model.cost else { continue };
            let (Some(input), Some(output)) = (cost.input, cost.output) else { continue };
            let mut tiers: Vec<Tier> = cost
                .tiers
                .into_iter()
                .filter_map(|tier| {
                    let size = tier.tier.filter(|kind| kind.kind == "context")?.size?;
                    Some(Tier { size, input: tier.input?, output: tier.output?, cache_read: tier.cache_read, cache_write: tier.cache_write })
                })
                .collect();
            tiers.sort_by_key(|tier| tier.size);
            models.insert(id, Rate { input, output, cache_read: cost.cache_read, cache_write: cost.cache_write, tiers });
        }
        if models.is_empty() {
            return Err("the catalog lists no Claude prices".to_owned());
        }
        Ok(Prices { fetched_at: now, models })
    }

    /// The rate for a model id as Claude Code logs it: the id itself, the id without its
    /// release date, or the longest listed id it starts with.
    fn rate(&self, model: &str) -> Option<&Rate> {
        let model = model.rsplit('/').next().unwrap_or(model);
        self.models
            .get(model)
            .or_else(|| self.models.get(crate::stats::display_name(model)))
            .or_else(|| self.models.iter().filter(|(id, _)| model.starts_with(id.as_str())).max_by_key(|(id, _)| id.len()).map(|(_, rate)| rate))
    }

    /// Dollars for `tokens` of `model`, sent in requests with about `prompt` prompt tokens.
    /// `None` when the model has no listed price.
    pub fn cost(&self, model: &str, prompt: u64, tokens: &Tokens) -> Option<f64> {
        let rate = self.rate(model)?;
        let tier = rate.tiers.iter().rev().find(|tier| prompt >= tier.size && prompt > 0);
        let (input, output, cache_read, cache_write) = match tier {
            Some(tier) => (tier.input, tier.output, tier.cache_read, tier.cache_write),
            None => (rate.input, rate.output, rate.cache_read, rate.cache_write),
        };
        // Anthropic's cache multipliers, where the list has no price of its own: a read is a
        // tenth of input, a five minute write 1.25 times, and an hour write twice.
        let dollars = tokens[INPUT] as f64 * input
            + tokens[OUTPUT] as f64 * output
            + tokens[CACHE_READ] as f64 * cache_read.unwrap_or(input * 0.1)
            + tokens[CACHE_WRITE_5M] as f64 * cache_write.unwrap_or(input * 1.25)
            + tokens[CACHE_WRITE_1H] as f64 * input * 2.0;
        Some(dollars / 1e6)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CATALOG: &str = r#"{
        "openai": {"models": {"gpt": {"cost": {"input": 1, "output": 2}}}},
        "anthropic": {"id": "anthropic", "models": {
            "claude-opus-4-5": {"cost": {"input": 5, "output": 25, "cache_read": 0.5, "cache_write": 6.25}},
            "claude-opus-4-5-20251101": {"cost": {"input": 5, "output": 25, "cache_read": 0.5, "cache_write": 6.25}},
            "claude-haiku-5-5": {"cost": {"input": 0.1, "output": 0.5, "cache_read": 0.01, "cache_write": 0.125,
                "tiers": [{"input": 0.5, "output": 2.5, "cache_read": 0.05, "cache_write": 0.625, "tier": {"type": "context", "size": 100000}}]}},
            "claude-free": {"name": "no cost listed"}
        }}
    }"#;

    #[test]
    fn the_catalog_keeps_claude_models_with_a_price() {
        let prices = Prices::from_catalog(CATALOG.as_bytes(), 7).unwrap();
        assert_eq!(prices.fetched_at, 7);
        assert_eq!(prices.models.len(), 3);
        assert_eq!(prices.models["claude-haiku-5-5"].tiers[0].size, 100_000);
        assert!(Prices::from_catalog(br#"{"anthropic": {"models": {}}}"#, 7).is_err());
    }

    #[test]
    fn model_ids_find_their_rate() {
        let prices = Prices::from_catalog(CATALOG.as_bytes(), 7).unwrap();
        assert!(prices.rate("claude-opus-4-5").is_some());
        assert!(prices.rate("claude-opus-4-5-20251101").is_some());
        assert!(prices.rate("claude-opus-4-5-20990101").is_some());
        assert!(prices.rate("claude-opus-4-5[1m]").is_some());
        assert!(prices.rate("gpt").is_none());
    }

    #[test]
    fn long_prompts_pay_their_tier() {
        let prices = Prices::from_catalog(CATALOG.as_bytes(), 7).unwrap();
        let tokens: Tokens = [1_000_000, 1_000_000, 0, 0, 0];
        assert_eq!(prices.cost("claude-haiku-5-5", 50_000, &tokens), Some(0.6));
        assert_eq!(prices.cost("claude-haiku-5-5", 100_000, &tokens), Some(3.0));
        assert_eq!(prices.cost("mystery", 0, &tokens), None);
    }
}
