//! What a provider reports, in the shape the menu bar and the panel read: limits with their
//! reset times, the plan, and how the last refresh went.

use serde::{Deserialize, Serialize};

use crate::time;

/// The remaining share at or under which a limit turns orange, then red.
pub const WARNING_LEFT: f64 = 30.0;
pub const CRITICAL_LEFT: f64 = 10.0;
/// A limit's pace is read only once this share of its window has passed and this much of it is
/// used: before that a single request projects to nonsense.
const PACE_MIN_ELAPSED: f64 = 0.05;
const PACE_MIN_USED: f64 = 5.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Normal,
    Warning,
    Critical,
}

/// One rate limit window.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Limit {
    /// Stable within a provider: "session", "weekly", "weekly-sonnet".
    pub id: String,
    pub label: String,
    /// 0 to 100, as the provider reports it.
    pub used: f64,
    /// Unix seconds; `None` while the window hasn't started.
    pub resets_at: Option<i64>,
    /// The window's length in seconds.
    pub window: i64,
    /// A limit on one model rather than on the whole plan.
    pub scoped: bool,
}

impl Limit {
    pub fn left(&self) -> f64 {
        (100.0 - self.used).clamp(0.0, 100.0)
    }

    pub fn used_up(&self) -> bool {
        self.left().round() <= 0.0
    }

    pub fn severity(&self) -> Severity {
        let left = self.left().round();
        if left <= CRITICAL_LEFT {
            Severity::Critical
        } else if left <= WARNING_LEFT {
            Severity::Warning
        } else {
            Severity::Normal
        }
    }

    /// Seconds until the reset, when one is scheduled and still ahead.
    pub fn resets_in(&self, now: i64) -> Option<i64> {
        self.resets_at.map(|at| at - now).filter(|left| *left > 0)
    }

    /// How much of the window has passed, 0 to 1.
    pub fn elapsed(&self, now: i64) -> Option<f64> {
        let left = self.resets_in(now)?;
        (self.window > 0).then(|| (1.0 - left as f64 / self.window as f64).clamp(0.0, 1.0))
    }

    /// Seconds until the limit is used up at the pace so far, when that comes before the reset.
    pub fn runs_out_in(&self, now: i64) -> Option<i64> {
        let (elapsed, resets_in) = (self.elapsed(now)?, self.resets_in(now)?);
        if elapsed < PACE_MIN_ELAPSED || self.used < PACE_MIN_USED || self.used_up() {
            return None;
        }
        let spent_seconds = elapsed * self.window as f64;
        let runway = ((100.0 - self.used) * spent_seconds / self.used) as i64;
        (runway < resets_in).then_some(runway)
    }
}

/// Pay-as-you-go spend past the plan's limits.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExtraUsage {
    pub used: f64,
    /// `None` when no monthly cap is set.
    pub limit: Option<f64>,
    pub currency: String,
}

/// A provider's limits as of one successful refresh.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    /// "Ultra", "Pro"; empty when the provider doesn't say.
    pub plan: String,
    pub limits: Vec<Limit>,
    pub extra: Option<ExtraUsage>,
    /// Unix seconds of the refresh.
    pub fetched_at: i64,
}

impl Snapshot {
    pub fn limit(&self, id: &str) -> Option<&Limit> {
        self.limits.iter().find(|l| l.id == id)
    }

    /// Limits whose window ended since the refresh read as fresh: nothing used, and no reset
    /// scheduled until the next request starts a window.
    pub fn rolled(&self, now: i64) -> Snapshot {
        let mut rolled = self.clone();
        for limit in &mut rolled.limits {
            if limit.resets_at.is_some_and(|at| at <= now) {
                limit.used = 0.0;
                limit.resets_at = None;
            }
        }
        rolled
    }
}

/// Why the limits on screen are not current.
#[derive(Clone, Debug, PartialEq)]
pub enum Problem {
    /// No login for this provider on this Mac.
    SignedOut,
    /// The login is there but its token has expired.
    LoginExpired,
    /// The provider asked us to wait; Unix seconds when it may be asked again.
    RateLimited { until: i64 },
    Offline(String),
    Failed(String),
}

impl Problem {
    pub fn title(&self) -> &'static str {
        match self {
            Problem::SignedOut => "Not signed in",
            Problem::LoginExpired => "Login expired",
            Problem::RateLimited { .. } => "Updates paused",
            Problem::Offline(_) => "Offline",
            Problem::Failed(_) => "Couldn't update",
        }
    }

    pub fn detail(&self, now: i64) -> String {
        match self {
            Problem::SignedOut => "Run `claude` in a terminal and sign in. Nerda Bar reads that login.".to_owned(),
            Problem::LoginExpired => "Run `claude` once to renew it. Nerda Bar never changes your login.".to_owned(),
            Problem::RateLimited { until } => format!("Claude asked for a pause. Trying again in {}.", time::span(until - now)),
            Problem::Offline(reason) | Problem::Failed(reason) => reason.clone(),
        }
    }
}

/// Which limit the menu bar shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum MenuBarMetric {
    /// The session, or whatever plan-wide limit is used up and so is what blocks work.
    #[default]
    Session,
    Weekly,
    /// The plan-wide limit with the least left.
    Lowest,
}

/// The limit the menu bar shows for `metric`.
pub fn menu_bar_limit(snapshot: &Snapshot, metric: MenuBarMetric) -> Option<&Limit> {
    let general = || snapshot.limits.iter().filter(|l| !l.scoped);
    let lowest = || general().min_by(|a, b| a.left().total_cmp(&b.left()));
    match metric {
        MenuBarMetric::Lowest => lowest(),
        MenuBarMetric::Weekly => snapshot.limit("weekly").or_else(lowest),
        MenuBarMetric::Session => general().find(|l| l.used_up()).or_else(|| snapshot.limit("session")).or_else(lowest),
    }
}

/// The limits the menu bar shows: the one `metric` picks, then with `models` every limit
/// that counts one model alone, as those run out on their own.
pub fn menu_bar_limits(snapshot: &Snapshot, metric: MenuBarMetric, models: bool) -> Vec<&Limit> {
    let main = menu_bar_limit(snapshot, metric);
    let scoped = snapshot.limits.iter().filter(|l| models && l.scoped);
    main.into_iter().chain(scoped).collect()
}

/// The letter that names a limit in the menu bar: "S", "W", "F" for "Fable weekly".
pub fn tag(limit: &Limit) -> char {
    limit.label.chars().next().map_or('?', |c| c.to_ascii_uppercase())
}

/// A percentage as the menu bar and the panel print it: left or used, whole.
pub fn percent(limit: &Limit, show_used: bool) -> String {
    let value = if show_used { limit.used.clamp(0.0, 100.0) } else { limit.left() };
    format!("{}%", value.round() as i64)
}

/// A token count in three significant figures: "1.8B", "751.5M", "19.4K", "812".
pub fn tokens(count: u64) -> String {
    let scaled = |value: f64, unit: &str| if value >= 100.0 { format!("{value:.0}{unit}") } else { format!("{value:.1}{unit}") };
    match count {
        0..1_000 => count.to_string(),
        1_000..1_000_000 => scaled(count as f64 / 1e3, "K"),
        1_000_000..1_000_000_000 => scaled(count as f64 / 1e6, "M"),
        _ => scaled(count as f64 / 1e9, "B"),
    }
}

/// Dollars: cents under a hundred, whole with thousands separators from there.
pub fn money(amount: f64) -> String {
    if amount < 100.0 {
        return format!("${amount:.2}");
    }
    let whole = amount.round() as u64;
    let digits = whole.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3 + 1);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    format!("${out}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limit(id: &str, used: f64, resets_in: i64, window: i64) -> Limit {
        Limit { id: id.to_owned(), label: id.to_owned(), used, resets_at: Some(1_000_000 + resets_in), window, scoped: false }
    }

    const NOW: i64 = 1_000_000;

    #[test]
    fn severity_follows_what_is_left() {
        assert_eq!(limit("a", 50.0, 60, 100).severity(), Severity::Normal);
        assert_eq!(limit("a", 70.0, 60, 100).severity(), Severity::Warning);
        assert_eq!(limit("a", 90.0, 60, 100).severity(), Severity::Critical);
        assert!(limit("a", 99.6, 60, 100).used_up());
        assert!(!limit("a", 99.4, 60, 100).used_up());
    }

    #[test]
    fn a_fast_pace_runs_out_before_the_reset() {
        // Half the five hours gone with 80% used: the remaining 20% lasts 37.5 minutes.
        let fast = limit("session", 80.0, 9000, 18_000);
        assert_eq!(fast.elapsed(NOW), Some(0.5));
        assert_eq!(fast.runs_out_in(NOW), Some(2250));
        // 40% used at the halfway mark lasts past the reset.
        assert_eq!(limit("session", 40.0, 9000, 18_000).runs_out_in(NOW), None);
        // Too early in the window to project.
        assert_eq!(limit("session", 30.0, 17_800, 18_000).runs_out_in(NOW), None);
    }

    #[test]
    fn a_window_that_ended_reads_as_fresh() {
        let snapshot = Snapshot { plan: String::new(), limits: vec![limit("session", 80.0, 60, 18_000), limit("weekly", 40.0, 9000, 604_800)], extra: None, fetched_at: NOW };
        let later = snapshot.rolled(NOW + 120);
        assert_eq!(later.limits[0].used, 0.0);
        assert_eq!(later.limits[0].resets_at, None);
        assert_eq!(later.limits[1].used, 40.0);
    }

    #[test]
    fn the_menu_bar_shows_the_session_until_something_else_blocks() {
        let mut snapshot = Snapshot { plan: String::new(), limits: vec![limit("session", 20.0, 60, 18_000), limit("weekly", 60.0, 9000, 604_800)], extra: None, fetched_at: NOW };
        assert_eq!(menu_bar_limit(&snapshot, MenuBarMetric::Session).unwrap().id, "session");
        assert_eq!(menu_bar_limit(&snapshot, MenuBarMetric::Weekly).unwrap().id, "weekly");
        assert_eq!(menu_bar_limit(&snapshot, MenuBarMetric::Lowest).unwrap().id, "weekly");
        snapshot.limits[1].used = 100.0;
        assert_eq!(menu_bar_limit(&snapshot, MenuBarMetric::Session).unwrap().id, "weekly");
    }

    #[test]
    fn the_menu_bar_adds_each_model_limit_with_its_letter() {
        let mut fable = limit("weekly-fable", 89.0, 60, 100);
        fable.label = "Fable weekly".to_owned();
        fable.scoped = true;
        let mut weekly = limit("weekly", 77.0, 60, 100);
        weekly.label = "Weekly".to_owned();
        let snapshot = Snapshot { plan: String::new(), limits: vec![weekly, fable], extra: None, fetched_at: NOW };
        let shown = menu_bar_limits(&snapshot, MenuBarMetric::Session, true);
        assert_eq!(shown.iter().map(|l| tag(l)).collect::<String>(), "WF");
        assert_eq!(menu_bar_limits(&snapshot, MenuBarMetric::Session, false).len(), 1);
    }

    #[test]
    fn counts_and_money_read_short() {
        assert_eq!(tokens(812), "812");
        assert_eq!(tokens(19_400), "19.4K");
        assert_eq!(tokens(751_500_000), "752M");
        assert_eq!(tokens(23_900_000), "23.9M");
        assert_eq!(tokens(1_800_000_000), "1.8B");
        assert_eq!(money(44.82), "$44.82");
        assert_eq!(money(185.7), "$186");
        assert_eq!(money(1395.2), "$1,395");
        assert_eq!(percent(&limit("a", 23.6, 60, 100), false), "76%");
        assert_eq!(percent(&limit("a", 23.6, 60, 100), true), "24%");
    }
}
