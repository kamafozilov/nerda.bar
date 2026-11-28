//! What the panel knows of each provider: Claude's and Codex's limits, logs and readings, and
//! DeepSeek's balance; how the worker's events land in it, and how each provider's menu bar
//! item reads it.

use std::sync::Arc;

use rune::Tx;

use crate::burn::History;
use crate::codex::Redeemed;
use crate::deepseek::{self, Ledger};
use crate::model::{self, Limit, MenuBarMetric, Problem, Provider, Severity, Snapshot};
use crate::stats::StatsView;
use crate::status_item;
use crate::time;
use crate::ui::{Phase, Ui};
use crate::worker::{Command, Event};

/// A provider with plan limits: Claude, Codex.
#[derive(Default)]
pub struct Agent {
    pub snapshot: Option<Snapshot>,
    pub problem: Option<Problem>,
    /// Unix seconds when the problem on screen was first met.
    pub problem_at: i64,
    pub refreshing: bool,
    pub stats: Option<Arc<StatsView>>,
    /// Every limit's readings, for the limit chart.
    pub history: Arc<History>,
    /// The id of the limit the chart shows, once one was picked.
    pub chart: Option<String>,
}

impl Agent {
    /// The limits as of `now`: windows that ended since the refresh read as fresh.
    pub fn limits(&self, now: i64) -> Option<Snapshot> {
        self.snapshot.as_ref().map(|snapshot| snapshot.rolled(now))
    }

    /// The limit the chart shows: the one picked, while the plan still has it, else the
    /// default.
    pub fn chart_limit<'a>(&self, snapshot: &'a Snapshot) -> Option<&'a Limit> {
        self.chart.as_deref().and_then(|id| snapshot.limit(id)).or_else(|| model::chart_default(snapshot))
    }

    fn fail(&mut self, problem: Problem, now: i64) {
        if self.problem.as_ref() != Some(&problem) {
            self.problem_at = now;
        }
        self.problem = Some(problem);
        self.refreshing = false;
    }
}

/// DeepSeek: its balance and how it moved.
#[derive(Default)]
pub struct DeepSeek {
    pub ledger: Option<Arc<Ledger>>,
    pub problem: Option<Problem>,
    pub problem_at: i64,
    pub refreshing: bool,
}

/// Where spending a Codex limit reset stands.
#[derive(Clone, Debug, PartialEq)]
pub enum ResetFlow {
    Idle,
    /// The question is on screen for this credit.
    Confirm(String),
    /// The request for this credit runs, under this id: asking again with the same id can
    /// never spend a second reset.
    Resetting { credit: String, request: String },
    Done,
    /// It didn't go through, why, whether that is certain, and the request's id for a retry.
    Failed { credit: String, reason: String, certain: bool, request: String },
}

impl ResetFlow {
    /// The credit the question on screen is about.
    pub fn credit(&self) -> Option<&str> {
        match self {
            ResetFlow::Confirm(credit) | ResetFlow::Resetting { credit, .. } | ResetFlow::Failed { credit, .. } => Some(credit),
            _ => None,
        }
    }
}

/// An event from the worker thread.
pub fn apply(s: &mut Ui, event: Event, tx: &mut Tx) {
    if s.phase != Phase::Shown {
        tx.instant();
    }
    s.now = time::unix_now();
    let now = s.now;
    match event {
        Event::Refreshing(Provider::DeepSeek) => s.deepseek.refreshing = true,
        Event::Refreshing(provider) => s.agent_mut(provider).refreshing = true,
        Event::Limits(provider, Ok(snapshot)) => {
            let agent = s.agent_mut(provider);
            agent.snapshot = Some(snapshot);
            agent.problem = None;
            agent.refreshing = false;
        }
        Event::Limits(provider, Err(problem)) => s.agent_mut(provider).fail(problem, now),
        Event::Stats(provider, stats) => s.agent_mut(provider).stats = Some(stats),
        Event::History(provider, history) => s.agent_mut(provider).history = history,
        Event::Balance(Ok(ledger)) => {
            s.deepseek.ledger = Some(ledger);
            s.deepseek.problem = None;
            s.deepseek.refreshing = false;
        }
        Event::Balance(Err(problem)) => {
            if s.deepseek.problem.as_ref() != Some(&problem) {
                s.deepseek.problem_at = now;
            }
            s.deepseek.problem = Some(problem);
            s.deepseek.refreshing = false;
        }
        Event::Redeemed(redeemed) => {
            let ResetFlow::Resetting { credit, request } = std::mem::replace(&mut s.reset, ResetFlow::Idle) else { return };
            s.reset = match redeemed {
                Redeemed::Reset | Redeemed::NothingToReset => ResetFlow::Done,
                Redeemed::Failed(reason) => ResetFlow::Failed { credit, reason, certain: true, request },
                Redeemed::Unknown(reason) => ResetFlow::Failed { credit, reason, certain: false, request },
            };
        }
    }
    s.sync_menu_bar();
}

impl Ui {
    pub fn agent(&self, provider: Provider) -> &Agent {
        match provider {
            Provider::Codex => &self.codex,
            _ => &self.claude,
        }
    }

    pub fn agent_mut(&mut self, provider: Provider) -> &mut Agent {
        match provider {
            Provider::Codex => &mut self.codex,
            _ => &mut self.claude,
        }
    }

    /// What the menu bar shows of `provider`'s plan, whether it counts what is used, and
    /// whether each model's own limit shows beside it.
    pub fn menu_bar(&self, provider: Provider) -> (MenuBarMetric, bool, bool) {
        match provider {
            Provider::Codex => (self.config.codex.menu_bar, self.config.codex.show_used, self.config.codex.models),
            _ => (self.config.menu_bar, self.config.show_used, self.config.menu_bar_models),
        }
    }

    pub fn show_used(&self, provider: Provider) -> bool {
        self.menu_bar(provider).1
    }

    /// Puts each provider's numbers on its menu bar item, adding the item once there is
    /// something to show and taking it away when there is nothing, or when the settings
    /// switch it off.
    pub fn sync_menu_bar(&self) {
        if self.demo {
            return;
        }
        for provider in [Provider::Claude, Provider::Codex] {
            let agent = self.agent(provider);
            let (metric, show_used, models) = self.menu_bar(provider);
            status_item::set_show_used(provider, show_used);
            // Codex gets an item only once it is signed in; Claude's is there from the start,
            // saying so when it is not.
            if !self.config.in_menu_bar(provider) || (provider == Provider::Codex && agent.snapshot.is_none()) {
                status_item::remove(provider.id());
                continue;
            }
            let mark = if provider == Provider::Codex { &status_item::OPENAI_MARK } else { &status_item::CLAUDE_MARK };
            status_item::install(&self.me, &self.data_dir, provider.id(), provider.name(), mark);
            let limits = agent.limits(self.now);
            let shown = limits.as_ref().map(|snapshot| model::menu_bar_limits(snapshot, metric.effective(Some(snapshot)), models)).unwrap_or_default();
            // One limit reads alone; with several, each takes its name.
            let tagged = shown.len() > 1;
            let mut parts: Vec<status_item::Part> =
                shown.iter().map(|limit| status_item::Part { tag: tagged.then(|| model::tag(limit)), text: model::percent(limit, show_used), severity: limit.severity() }).collect();
            if parts.is_empty() {
                parts.push(status_item::Part { tag: None, text: "–".to_owned(), severity: Severity::Normal });
            }
            let mut tooltip: Vec<String> = Vec::new();
            if let Some(problem) = &agent.problem {
                tooltip.push(format!("{}: {}", provider.name(), problem.title()));
            }
            for limit in limits.iter().flat_map(|snapshot| &snapshot.limits) {
                let reset = limit.resets_in(self.now).map_or_else(String::new, |left| format!(", resets in {}", time::span(left)));
                tooltip.push(format!("{}: {} left{reset}", limit.label, model::percent(limit, false)));
            }
            status_item::set(provider.id(), None, &parts, agent.problem.is_some(), &tooltip.join("\n"));
        }
        // DeepSeek: its balance, once a key is there, while the settings show it.
        match self.deepseek.ledger.as_ref().and_then(|ledger| ledger.last.as_ref()).filter(|_| self.config.deepseek_in_menu_bar) {
            Some(balance) => {
                status_item::install(&self.me, &self.data_dir, Provider::DeepSeek.id(), Provider::DeepSeek.name(), &status_item::DEEPSEEK_MARK);
                let severity = deepseek::severity(balance, self.config.deepseek_warn_below);
                let text = deepseek::money(balance.total, &balance.currency);
                let note = match severity {
                    Severity::Critical => ", too low for API calls",
                    Severity::Warning => ", running low",
                    Severity::Normal => "",
                };
                status_item::set(Provider::DeepSeek.id(), None, &[status_item::Part { tag: None, text: text.clone(), severity }], self.deepseek.problem.is_some(), &format!("DeepSeek balance: {text}{note}"));
            }
            None => status_item::remove(Provider::DeepSeek.id()),
        }
        self.sync_app_item();
    }

    pub(crate) fn send(&self, command: Command) {
        if let Some(worker) = &self.worker {
            worker.send(command);
        }
    }
}

/// "Use…" on a reset: ask first.
pub fn ask_reset(s: &mut Ui, credit: &str) {
    s.reset = ResetFlow::Confirm(credit.to_owned());
}

pub fn cancel_reset(s: &mut Ui) {
    s.reset = ResetFlow::Idle;
}

/// The person confirmed: spend the reset. A retry after a failure reuses the request's id, so
/// a reset the first request did spend is never spent twice.
pub fn confirm_reset(s: &mut Ui) {
    let (credit, request) = match &s.reset {
        ResetFlow::Confirm(credit) => (credit.clone(), crate::codex::request_id()),
        ResetFlow::Failed { credit, request, .. } => (credit.clone(), request.clone()),
        _ => return,
    };
    s.reset = ResetFlow::Resetting { credit: credit.clone(), request: request.clone() };
    if s.demo {
        return;
    }
    s.send(Command::Redeem { credit, request_id: request });
}
