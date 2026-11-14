//! The connectors service: one per app. It owns the adapters, the poll
//! scheduler, the scorer state, the pending OAuth flows, the quiet / Focus
//! inputs and the alert sink, and computes the `inbox.changed`,
//! `agenda.changed` and `quiet.changed` payloads for the app's publisher
//! (08 §5.3). [`Connectors::run`] is the background loop (06 §2.3, 07 §3.4,
//! §6, §8): a 1 s scheduler tick, the 2 s scoring debounce, the write-back
//! drain, the every-minute snooze sweep and agenda refresh, calendar and
//! usage alerts, Slack DM discovery every 10 min, and the launch refresh of
//! Slack tokens older than 20 days. Everything blocking runs on the
//! blocking pool; nothing here waits on the webview.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::OffsetDateTime;

use super::accounts::{self, AccountRow, AccountTokens};
use super::ai::{self, AiConfig, AiSettings, BackendFactory, HttpBackends};
use super::gcal::GcalAdapter;
use super::gmail::GmailAdapter;
use super::health;
use super::http::HttpClient;
use super::oauth::{self, ConnectOptions, Loopback, OAuthClient, OAuthError};
use super::scheduler::{Planned, Scheduler};
use super::slack::SlackAdapter;
use super::writeback;
use super::{
    Api, ConnectorAdapter, ConnectorError, ConnectorSettings, Provider, Service, SourceKind,
    StaticToken, SyncCx, SyncOutcome,
};
use crate::db::{Db, format_timestamp};
use crate::keyring::Keyring;
use crate::notes::ctx::TzMode;
use crate::push::Event;
use crate::repo::{self, Account, AccountSource};
use crate::urgency::alerts::{
    self, AlertEvent, AlertOutcome, Environment, OsPermission, QuietChanged,
};
use crate::urgency::scoring::{self, AiCtx, BatchReport, ScorerState};
use crate::urgency::{self, with_tz};

/// Write-backs are looked at this often.
pub const DRAIN_EVERY: Duration = Duration::from_secs(5);
/// Snooze sweep S2 and the agenda's section refresh.
pub const MINUTE: Duration = Duration::from_secs(60);
/// Calendar alerts are checked this often.
pub const CALENDAR_ALERTS_EVERY: Duration = Duration::from_secs(30);
/// New Slack DMs appear within this long (06 §3.4).
pub const DISCOVERY_EVERY: Duration = Duration::from_secs(600);
/// Sender names are refreshed daily (06 §8.5).
pub const USERS_CACHE_TTL: Duration = Duration::from_secs(86_400);

type PublishFn = Arc<dyn Fn(Event) + Send + Sync>;
type SinkFn = Arc<dyn Fn(&AlertCommand) + Send + Sync>;
type Clock = Arc<dyn Fn() -> OffsetDateTime + Send + Sync>;

/// What the app does for an alert (07 §8.3, §8.6): play the cue through the
/// overlay, post / withdraw an OS notification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)] // one per alert; boxing would change the app's match
pub enum AlertCommand {
    /// The overlay plays `sound` once (`alert.fired`).
    Fired(AlertOutcome),
    /// Remove delivered notifications with these identifiers.
    Withdraw { identifiers: Vec<String> },
}

/// Options for [`Connectors::with_options`].
#[derive(Clone, Default)]
pub struct ConnectorsOptions {
    /// A fixed clock (tests).
    pub clock: Option<Clock>,
    pub tz: TzMode,
    /// The AI backend factory; default reqwest-backed.
    pub ai_factory: Option<Arc<dyn BackendFactory>>,
}

/// The OS half of the suppression inputs, set by the app crate.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OsState {
    pub focus: Option<bool>,
    pub presentation: bool,
    pub screen_locked: bool,
    pub output_muted: bool,
    pub os_permission: OsPermission,
}

struct PendingFlow {
    provider: Provider,
    state: String,
    verifier: String,
    client: OAuthClient,
    redirect_uri: String,
    loopback: Option<Loopback>,
    cancel: Arc<AtomicBool>,
    created: Instant,
}

/// `account_add_start` result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectStart {
    /// The app opens this in the system browser (or shows Copy link).
    pub auth_url: String,
    pub state: String,
    pub redirect_uri: String,
}

/// What one poll pass did.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PollReport {
    pub polled: usize,
    pub inserted: usize,
    pub updated: usize,
    pub events_changed: usize,
    pub errors: Vec<(String, String)>,
}

pub struct Connectors {
    db: Arc<Db>,
    keyring: Arc<Keyring>,
    http: Arc<dyn HttpClient>,
    gmail: GmailAdapter,
    gcal: GcalAdapter,
    slack: SlackAdapter,
    scheduler: Mutex<Scheduler>,
    scorer: Mutex<ScorerState>,
    ai_factory: Arc<dyn BackendFactory>,
    flows: Mutex<HashMap<String, PendingFlow>>,
    publish: Mutex<Option<PublishFn>>,
    sink: Mutex<Option<SinkFn>>,
    os: Mutex<OsState>,
    os_rate: Mutex<alerts::OsRateLimiter>,
    channel_cache: Mutex<HashMap<String, (Instant, Vec<super::SourceCandidate>)>>,
    discovered: Mutex<HashMap<String, Instant>>,
    score_due: Mutex<Option<Instant>>,
    polling: AtomicBool,
    clock: Option<Clock>,
    tz: TzMode,
}

impl std::fmt::Debug for Connectors {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Connectors")
            .field("keyring", &self.keyring.kind())
            .finish_non_exhaustive()
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

impl Connectors {
    pub fn new(db: Arc<Db>, keyring: Arc<Keyring>, http: Arc<dyn HttpClient>) -> Arc<Connectors> {
        Connectors::with_options(db, keyring, http, ConnectorsOptions::default())
    }

    pub fn with_options(
        db: Arc<Db>,
        keyring: Arc<Keyring>,
        http: Arc<dyn HttpClient>,
        opts: ConnectorsOptions,
    ) -> Arc<Connectors> {
        let factory = opts
            .ai_factory
            .clone()
            .unwrap_or_else(|| Arc::new(HttpBackends(Arc::clone(&http))));
        Arc::new(Connectors {
            db,
            keyring,
            http,
            gmail: GmailAdapter,
            gcal: GcalAdapter,
            slack: SlackAdapter::new(),
            scheduler: Mutex::new(Scheduler::new()),
            scorer: Mutex::new(ScorerState::default()),
            ai_factory: factory,
            flows: Mutex::new(HashMap::new()),
            publish: Mutex::new(None),
            sink: Mutex::new(None),
            os: Mutex::new(OsState::default()),
            os_rate: Mutex::new(alerts::OsRateLimiter::default()),
            channel_cache: Mutex::new(HashMap::new()),
            discovered: Mutex::new(HashMap::new()),
            score_due: Mutex::new(None),
            polling: AtomicBool::new(false),
            clock: opts.clock,
            tz: opts.tz,
        })
    }

    pub fn db(&self) -> &Arc<Db> {
        &self.db
    }

    pub fn keyring(&self) -> &Arc<Keyring> {
        &self.keyring
    }

    pub fn http(&self) -> &Arc<dyn HttpClient> {
        &self.http
    }

    pub fn tz(&self) -> TzMode {
        self.tz
    }

    pub fn now(&self) -> OffsetDateTime {
        match &self.clock {
            Some(c) => c(),
            None => OffsetDateTime::now_utc(),
        }
    }

    pub fn ai_factory(&self) -> &Arc<dyn BackendFactory> {
        &self.ai_factory
    }

    pub fn slack(&self) -> &SlackAdapter {
        &self.slack
    }

    /// Where `*.changed` requests go (`Publisher::publish` in the app).
    pub fn set_publisher(&self, f: impl Fn(Event) + Send + Sync + 'static) {
        *lock(&self.publish) = Some(Arc::new(f));
    }

    /// Where alert commands go (the app plays cues and posts notifications).
    pub fn set_alert_sink(&self, f: impl Fn(&AlertCommand) + Send + Sync + 'static) {
        *lock(&self.sink) = Some(Arc::new(f));
    }

    pub fn changed(&self, events: &[Event]) {
        let f = lock(&self.publish).clone();
        if let Some(f) = f {
            for e in events {
                f(*e);
            }
        }
    }

    /// Hand a command to the app's alert sink.
    pub fn send_alert(&self, cmd: AlertCommand) {
        let f = lock(&self.sink).clone();
        if let Some(f) = f {
            f(&cmd);
        }
    }

    /// The app reports the OS half of the quiet / suppression state.
    pub fn set_os_state(&self, s: OsState) {
        let changed = *lock(&self.os) != s;
        *lock(&self.os) = s;
        if changed {
            self.changed(&[Event::Quiet]);
        }
    }

    pub fn os_state(&self) -> OsState {
        lock(&self.os).clone()
    }

    /// The overlay is visible on the Inbox (06 §2.3 foreground boost).
    pub fn set_foreground(&self, on: bool) {
        lock(&self.scheduler).foreground = on;
    }

    /// Whether the foreground boost is on (the app reports window focus).
    pub fn is_foreground(&self) -> bool {
        lock(&self.scheduler).foreground
    }

    /// Network reachability from the app (08 §11).
    pub fn set_offline(&self, offline: bool) {
        let was = {
            let mut s = lock(&self.scheduler);
            let was = s.offline;
            s.offline = offline;
            if was && !offline {
                s.wake();
            }
            was
        };
        if was != offline {
            self.changed(&[Event::Health, Event::Accounts]);
        }
    }

    pub fn is_offline(&self) -> bool {
        lock(&self.scheduler).offline
    }

    /// The OS woke Mode: catch-up poll and sweep now.
    pub fn wake(&self) {
        lock(&self.scheduler).wake();
    }

    // ------------------------------------------------------------ payloads

    /// The payload of `inbox.changed`, `agenda.changed`, `quiet.changed`
    /// and `accounts.changed` (`None` for other events), for the app's
    /// `PayloadFn`. `inbox.changed` is `null` with no account.
    pub fn payload(&self, event: Event) -> crate::db::Result<Option<Value>> {
        let now = self.now();
        let tz = self.tz;
        match event {
            Event::Inbox => {
                let clock = crate::queries::Clock {
                    now: format_timestamp(now),
                    today: String::new(),
                    tomorrow: String::new(),
                };
                let p = self.db.read(|c| urgency::route::inbox_changed(c, &clock))?;
                Ok(Some(serde_json::to_value(p)?))
            }
            Event::Agenda => {
                let p = self
                    .db
                    .read(|c| with_tz(c, tz, |z| crate::calendar::agenda_changed(c, now, z)))?;
                Ok(Some(serde_json::to_value(p)?))
            }
            Event::Quiet => Ok(Some(serde_json::to_value(self.quiet_state()?)?)),
            Event::Accounts => {
                let rows = self.db.read(|c| accounts::list(c, now))?;
                Ok(Some(
                    serde_json::json!({ "accounts": rows, "offline": self.is_offline() }),
                ))
            }
            _ => Ok(None),
        }
    }

    /// `quiet.changed`: quiet hours and the manual pause from settings, Focus
    /// detected here (macOS) or reported by the app, presentation from the app.
    pub fn quiet_state(&self) -> crate::db::Result<QuietChanged> {
        let env = self.environment()?;
        let now = self.now();
        let tz = self.tz;
        self.db
            .read(|c| Ok(with_tz(c, tz, |z| alerts::quiet_state(&env, now, z))))
    }

    /// The suppression inputs (settings + OS state + Focus detection).
    pub fn environment(&self) -> crate::db::Result<Environment> {
        let mut env = self.db.read(Environment::load)?;
        let os = self.os_state();
        let detect = self
            .db
            .read(|c| {
                Ok(crate::settings::get_in(c, "notifications.focus_detection")?
                    .get("macos_assertions")
                    .and_then(Value::as_bool)
                    .unwrap_or(true))
            })
            .unwrap_or(true);
        env.focus = os.focus.or_else(|| alerts::detect_focus(detect));
        env.presentation = os.presentation;
        env.screen_locked = os.screen_locked;
        env.output_muted = os.output_muted;
        env.os_permission = os.os_permission;
        Ok(env)
    }

    /// `health.changed`'s `offline`, `paused_accounts`, `broken_accounts`.
    /// What the scheduler knows about an account, for its health line
    /// (06 §2.4): rate-limit pause, the last two polls, the budget, the
    /// write-back retry count.
    pub fn runtime(
        &self,
        account: &Account,
        settings: &ConnectorSettings,
    ) -> crate::db::Result<health::Runtime> {
        let now = self.now();
        let slack = account.provider == "slack";
        let service = if slack {
            Service::Slack
        } else {
            Service::Gmail
        };
        let id = account.id.clone();
        let attempts: Option<i64> = self.db.read(move |c| {
            Ok(c.query_row(
                "SELECT MAX(attempts) FROM connector_writebacks WHERE account_id = ?1 AND attempts > 0",
                [&id],
                |r| r.get(0),
            )?)
        })?;
        let mut s = lock(&self.scheduler);
        let (used, cap) = s.budget_used(&account.id, service, settings, now);
        let (last_ok, prev_ok) = s.last_polls(&account.id);
        Ok(health::Runtime {
            offline: s.offline,
            rate_limited_until: s
                .paused_until(&account.id)
                .filter(|t| *t > now)
                .map(format_timestamp),
            last_poll_failed: !last_ok,
            previous_poll_failed: !prev_ok,
            writeback_retrying: attempts.map(|n| (n, writeback::LADDER_SECS.len() as i64)),
            next_poll_in_secs: None,
            budget_used: used as i64,
            budget_cap: cap as i64,
            interval_secs: if slack {
                settings.slack_seconds
            } else {
                settings.gmail_seconds
            },
            provider_label: if slack { "Slack" } else { "Google" }.into(),
        })
    }

    pub fn health_fields(&self) -> crate::db::Result<health::HealthFields> {
        let off = self.is_offline();
        self.db.read(|c| health::health_fields(c, off))
    }

    // -------------------------------------------------------------- OAuth

    fn adapter(&self, kind: Option<SourceKind>, provider: Provider) -> &dyn ConnectorAdapter {
        match (provider, kind.map(|k| k.service())) {
            (Provider::Slack, _) => &self.slack,
            (_, Some(Service::Calendar)) => &self.gcal,
            _ => &self.gmail,
        }
    }

    /// `begin_connect` (06 §2.1, §4.1): PKCE + state, bind the loopback
    /// listener, return the URL for the browser. A second Connect for the
    /// same provider cancels the pending one (§4.3).
    pub fn begin_connect(
        &self,
        provider: Provider,
        opts: ConnectOptions,
    ) -> Result<ConnectStart, OAuthError> {
        let client = oauth::load_client(&self.keyring, provider, opts.team_id.as_deref())?;
        {
            let mut flows = lock(&self.flows);
            for f in flows.values().filter(|f| f.provider == provider) {
                f.cancel.store(true, Ordering::SeqCst);
            }
            flows.retain(|_, f| f.provider != provider);
        }
        let settings = self
            .db
            .read(ConnectorSettings::load)
            .map_err(|e| OAuthError::Connector(e.to_string()))?;
        let loopback = Loopback::bind(&settings.oauth_ports, provider == Provider::Google)?;
        let pkce = oauth::Pkce::new();
        let state = oauth::random_state();
        let redirect_uri = loopback.redirect_uri();
        let auth_url = oauth::authorization_url(
            provider,
            &client,
            &redirect_uri,
            &state,
            &pkce.challenge,
            &opts,
        );
        lock(&self.flows).insert(
            state.clone(),
            PendingFlow {
                provider,
                state: state.clone(),
                verifier: pkce.verifier,
                client,
                redirect_uri: redirect_uri.clone(),
                loopback: Some(loopback),
                cancel: Arc::new(AtomicBool::new(false)),
                created: Instant::now(),
            },
        );
        Ok(ConnectStart {
            auth_url,
            state,
            redirect_uri,
        })
    }

    /// Cancel a pending flow (the dialog's Cancel).
    pub fn cancel_connect(&self, state: &str) -> bool {
        match lock(&self.flows).remove(state) {
            Some(f) => {
                f.cancel.store(true, Ordering::SeqCst);
                true
            }
            None => false,
        }
    }

    /// `complete_connect` (06 §4.1): wait for the browser (≤ 5 min), check
    /// `state`, exchange the code, identify, upsert the account by
    /// `(provider, external_id)`, write the keyring entry, seed sources.
    /// Blocking; the app runs it on the blocking pool.
    /// The provider of a connect still in flight (the app names it in the
    /// notch when the sign-in fails).
    pub fn flow_provider(&self, state: &str) -> Option<Provider> {
        lock(&self.flows).get(state).map(|f| f.provider)
    }

    pub fn finish_connect(&self, state: &str) -> Result<AccountRow, OAuthError> {
        let (loopback, cancel, created, flow_state) = {
            let mut flows = lock(&self.flows);
            let f = flows.get_mut(state).ok_or(OAuthError::UnknownFlow)?;
            (
                f.loopback.take().ok_or(OAuthError::UnknownFlow)?,
                Arc::clone(&f.cancel),
                f.created,
                f.state.clone(),
            )
        };
        let deadline = created + oauth::FLOW_TIMEOUT;
        let cb = loopback.wait(&flow_state, deadline, &cancel);
        let flow = lock(&self.flows).remove(state);
        let flow = flow.ok_or(OAuthError::Cancelled)?;
        match cb? {
            oauth::Callback::Denied => Err(OAuthError::AccessDenied),
            oauth::Callback::Error(e) if e.contains("scope") || e.contains("approval") => {
                Err(OAuthError::AdminApproval)
            }
            oauth::Callback::Error(e) => Err(OAuthError::Exchange(e)),
            oauth::Callback::Code(code) => self.complete_with_code(&flow, &code),
        }
    }

    fn complete_with_code(&self, flow: &PendingFlow, code: &str) -> Result<AccountRow, OAuthError> {
        let now = self.now();
        let ex = oauth::exchange_code(
            self.http.as_ref(),
            flow.provider,
            &flow.client,
            code,
            &flow.verifier,
            &flow.redirect_uri,
            now,
        )?;
        let token = StaticToken(ex.tokens.access_token.clone());
        let api = Api::new(self.http.as_ref(), &token, 50);
        let adapter = self.adapter(None, flow.provider);
        let mut identity = adapter.identify(&api, &ex.scopes)?;
        if let (Some(sub), Provider::Google) = (&ex.id_sub, flow.provider)
            && *sub != identity.external_id
        {
            return Err(OAuthError::Exchange("the identity did not match".into()));
        }
        if identity.scopes.is_empty() {
            identity.scopes = ex.scopes.clone();
        }
        // Store the token first: a refused keyring means no row (06 §5 "Failure").
        let provider = flow.provider;
        let (account, created) = self
            .db
            .write({
                let identity = identity.clone();
                move |c| repo::tx(c, |tx| accounts::upsert(tx, provider, &identity, now))
            })
            .map_err(|e| OAuthError::Connector(e.to_string()))?;
        if let Err(e) = self.keyring.set_json(&account.keyring_ref, &ex.tokens) {
            // a new account leaves no row; a re-authorized one keeps its
            // row and its previous tokens
            if created {
                let id = account.id.clone();
                let device = self.db.device_id().to_string();
                let _ = self.db.write(move |c| {
                    repo::tx(c, |tx| accounts::delete_rows(tx, &device, &id).map(|_| ()))
                });
            }
            return Err(OAuthError::Connector(format!(
                "Mode could not store the sign-in in the keyring: {e}"
            )));
        }
        // Seed sources (06 §3.4).
        let mut cands = adapter.list_sources(&api, &account).unwrap_or_default();
        if flow.provider == Provider::Google {
            cands.retain(|c| c.kind != SourceKind::GmailLabel);
            cands.extend(self.gcal.list_sources(&api, &account).unwrap_or_default());
        }
        let (aid, now_s) = (account.id.clone(), format_timestamp(now));
        self.db
            .write(move |c| accounts::seed_sources(c, &aid, &cands, &now_s))
            .map_err(|e| OAuthError::Connector(e.to_string()))?;
        lock(&self.scheduler).wake();
        self.changed(&[Event::Accounts, Event::Health]);
        let id = account.id.clone();
        self.db
            .read(|c| {
                let a = repo::must_get::<Account>(c, &id)?;
                accounts::row(c, &a, now)
            })
            .map_err(|e| OAuthError::Connector(e.to_string()))
    }

    /// Re-authorize (06 §3.3): §4 again with the same services and a
    /// `login_hint`; finishing replaces the tokens in place.
    pub fn begin_reauthorise(&self, account_id: &str) -> Result<ConnectStart, OAuthError> {
        let a = self
            .db
            .read(|c| repo::get_by_id::<Account>(c, account_id))
            .map_err(|e| OAuthError::Connector(e.to_string()))?
            .ok_or(OAuthError::UnknownFlow)?;
        let provider = Provider::parse(&a.provider).unwrap_or(Provider::Google);
        let scopes = super::json_strings(&a.scopes_json);
        // Google asks for both services with write-back again: the card's
        // notices ("Calendar only — re-authorize to add Gmail", "write-back
        // off — re-authorize to enable") promise exactly that; Google's
        // consent screen still lets the user untick either.
        let opts = ConnectOptions {
            gmail: true,
            calendar: true,
            gmail_read_only: false,
            slack_writeback: provider != Provider::Slack
                || scopes.iter().any(|s| s.ends_with(":write")),
            login_hint: a.email.clone(),
            team_id: a.team_id.clone(),
        };
        self.begin_connect(provider, opts)
    }

    // ------------------------------------------------------------- polling

    fn tokens_for(&self, account: &Account) -> AccountTokens {
        let mut t =
            AccountTokens::for_account(account, Arc::clone(&self.keyring), Arc::clone(&self.http));
        if let Some(c) = &self.clock {
            let c = Arc::clone(c);
            t.now = Box::new(move || c());
        }
        let (db, id) = (Arc::clone(&self.db), account.id.clone());
        t.on_scopes = Some(Box::new(move |scopes: Vec<String>| {
            let (id, now) = (id.clone(), crate::db::now_rfc3339());
            let _ = db.write(move |c| accounts::update_scopes(c, &id, &scopes, &now));
        }));
        t
    }

    /// Poll one source now (the scheduler's unit, "Refresh", tests).
    pub fn sync_source(
        &self,
        account_id: &str,
        source_id: &str,
    ) -> Result<SyncOutcome, ConnectorError> {
        let account = self
            .db
            .read(|c| repo::get_by_id::<Account>(c, account_id))?
            .ok_or(ConnectorError::NotFound)?;
        let source = self
            .db
            .read(|c| repo::get_by_id::<AccountSource>(c, source_id))?
            .ok_or(ConnectorError::NotFound)?;
        let settings = self.db.read(ConnectorSettings::load)?;
        let kind = SourceKind::parse(&source.kind);
        let service = kind.map(|k| k.service()).unwrap_or(Service::Gmail);
        let now = self.now();
        let budget = lock(&self.scheduler).budget_for(&account.id, service, &settings, now);
        self.run_planned(
            Planned {
                account,
                source,
                service,
                catch_up: false,
            },
            budget.max(1),
            &settings,
        )
    }

    fn run_planned(
        &self,
        p: Planned,
        budget: u32,
        settings: &ConnectorSettings,
    ) -> Result<SyncOutcome, ConnectorError> {
        let now = self.now();
        let provider = Provider::parse(&p.account.provider).unwrap_or(Provider::Google);
        let tokens = self.tokens_for(&p.account);
        let cx = SyncCx {
            db: &self.db,
            api: Api::new(self.http.as_ref(), &tokens, budget),
            account: &p.account,
            settings,
            now,
            tz: self.tz,
        };
        let adapter = self.adapter(SourceKind::parse(&p.source.kind), provider);
        let res = adapter.sync(&cx, &p.source);
        let used = cx.api.used();
        let failures = {
            let mut s = lock(&self.scheduler);
            s.consume(&p.account.id, p.service, used, settings, now);
            let r = s.record(&p.account.id, res.as_ref().map(|_| ()), now);
            if let Ok(o) = &res
                && let Some(t) = o
                    .latest_message_at
                    .as_deref()
                    .and_then(crate::db::parse_timestamp)
            {
                s.saw_message(&p.source.id, t);
            }
            r
        };
        // last_polled_at moves unless the budget ran out mid-way.
        let polled = !matches!(&res, Ok(o) if o.more)
            && !matches!(
                res,
                Err(ConnectorError::RateLimited { .. }) | Err(ConnectorError::Offline(_))
            );
        if polled {
            let (sid, now_s) = (p.source.id.clone(), format_timestamp(now));
            self.db.write(move |c| {
                c.execute(
                    "UPDATE account_sources SET last_polled_at = ?2 WHERE id = ?1",
                    rusqlite::params![sid, now_s],
                )?;
                Ok(())
            })?;
        }
        let mut publish = Vec::new();
        match &res {
            Ok(o) => {
                let id = p.account.id.clone();
                let before = p.account.status.clone();
                let after = self.db.write(move |c| {
                    health::record_success(c, &id, now)?;
                    Ok(repo::get_by_id::<Account>(c, &id)?.map(|a| a.status))
                })?;
                if after.as_deref() != Some(before.as_str()) {
                    publish.extend([Event::Health, Event::Accounts]);
                }
                if !o.inserted.is_empty() || o.updated > 0 || o.deleted > 0 {
                    publish.extend([Event::Inbox, Event::Pill]);
                }
                if o.events_changed > 0 || (p.service == Service::Calendar && o.deleted > 0) {
                    publish.extend([Event::Agenda, Event::Pill]);
                }
                if !o.inserted.is_empty() {
                    *lock(&self.score_due) = Some(Instant::now() + scoring::DEBOUNCE);
                }
                self.dispatch_alerts(o.alerts.clone());
            }
            Err(e) => {
                if let Some(t) = health::transition_for(e, failures) {
                    let id = p.account.id.clone();
                    if self
                        .db
                        .write(move |c| health::transition(c, &id, &t, now))?
                        .is_some()
                    {
                        publish.extend([Event::Health, Event::Accounts]);
                    }
                }
                if matches!(e, ConnectorError::Offline(_)) {
                    tracing::info!(target: "mode::connectors", "network unreachable; pollers pause");
                }
                tracing::warn!(target: "mode::connectors", account = %p.account.id, source = %p.source.kind, error = %e, "sync failed");
            }
        }
        publish.dedup();
        self.changed(&publish);
        res
    }

    /// One scheduler pass: plan and run every due source in order.
    pub fn poll_once(&self) -> crate::db::Result<PollReport> {
        if self.polling.swap(true, Ordering::SeqCst) {
            return Ok(PollReport::default());
        }
        let out = self.poll_inner();
        self.polling.store(false, Ordering::SeqCst);
        out
    }

    fn poll_inner(&self) -> crate::db::Result<PollReport> {
        let settings = self.db.read(ConnectorSettings::load)?;
        let accounts = self
            .db
            .read(|c| repo::query::<Account>(c, "1 ORDER BY position", &[]))?;
        let sources = self
            .db
            .read(|c| repo::query::<AccountSource>(c, "enabled = 1", &[]))?;
        let now = self.now();
        let plan = lock(&self.scheduler).plan(now, &accounts, &sources, &settings);
        let mut rep = PollReport::default();
        for p in plan {
            let budget =
                lock(&self.scheduler).budget_for(&p.account.id, p.service, &settings, self.now());
            if budget == 0 {
                continue;
            }
            // a 429 earlier in this pass pauses the rest of the account
            if lock(&self.scheduler)
                .paused_until(&p.account.id)
                .is_some_and(|t| t > self.now())
            {
                continue;
            }
            let (aid, kind) = (p.account.id.clone(), p.source.kind.clone());
            rep.polled += 1;
            match self.run_planned(p, budget, &settings) {
                Ok(o) => {
                    rep.inserted += o.inserted.len();
                    rep.updated += o.updated;
                    rep.events_changed += o.events_changed;
                }
                Err(e) => rep.errors.push((format!("{aid}/{kind}"), e.to_string())),
            }
        }
        Ok(rep)
    }

    /// Slack: re-list DMs and group DMs (every 10 min) so new ones appear.
    /// Discover new sources of an account (06 §2.1 discovery): Slack DMs
    /// and group DMs (every 10 min from `run`), Google labels and calendars
    /// (on demand, `sources_refresh`). Returns the rows added.
    pub fn discover(&self, account_id: &str) -> Result<usize, ConnectorError> {
        let account = self
            .db
            .read(|c| repo::get_by_id::<Account>(c, account_id))?
            .ok_or(ConnectorError::NotFound)?;
        if account.status != "active" {
            return Ok(0);
        }
        let slack = account.provider == "slack";
        let service = if slack {
            Service::Slack
        } else {
            Service::Gmail
        };
        let tokens = self.tokens_for(&account);
        let settings = self.db.read(ConnectorSettings::load)?;
        let now = self.now();
        let budget = lock(&self.scheduler).budget_for(&account.id, service, &settings, now);
        let api = Api::new(self.http.as_ref(), &tokens, budget);
        // Slack: new DMs and group DMs; Google: labels and calendars (the
        // Integrations card's pickers). Existing rows keep the user's toggles.
        let cands = if slack {
            self.slack.direct_sources(&api)
        } else {
            self.gmail.list_sources(&api, &account).and_then(|mut c| {
                c.extend(self.gcal.list_sources(&api, &account)?);
                Ok(c)
            })
        };
        lock(&self.scheduler).consume(&account.id, service, api.used(), &settings, now);
        let cands = cands?;
        let (id, now_s) = (account.id.clone(), format_timestamp(now));
        let n = self
            .db
            .write(move |c| accounts::seed_sources(c, &id, &cands, &now_s))?;
        lock(&self.discovered).insert(account.id.clone(), Instant::now());
        Ok(n)
    }

    /// Drain due write-backs (06 §2.5).
    pub fn drain_writebacks(&self) -> crate::db::Result<writeback::DrainReport> {
        let settings = self.db.read(ConnectorSettings::load)?;
        let now = self.now();
        let rep = writeback::drain(&self.db, now, |account, item| {
            let provider = Provider::parse(&account.provider).unwrap_or(Provider::Google);
            let tokens = self.tokens_for(account);
            let service = if provider == Provider::Slack {
                Service::Slack
            } else {
                Service::Gmail
            };
            let budget = lock(&self.scheduler).budget_for(&account.id, service, &settings, now);
            if budget == 0 {
                return Err(ConnectorError::RateLimited {
                    retry_after_secs: 60,
                });
            }
            let cx = SyncCx {
                db: &self.db,
                api: Api::new(self.http.as_ref(), &tokens, budget),
                account,
                settings: &settings,
                now,
                tz: self.tz,
            };
            let adapter = self.adapter(None, provider);
            let r = adapter.write_back(&cx, item);
            lock(&self.scheduler).consume(&account.id, service, cx.api.used(), &settings, now);
            if let Err(e) = &r
                && e.is_auth()
                && let Some(t) = health::transition_for(e, 0)
            {
                let id = account.id.clone();
                let _ = self.db.write(move |c| health::transition(c, &id, &t, now));
            }
            r
        })?;
        if rep.sent + rep.dropped + rep.unsupported > 0 {
            self.changed(&[Event::Inbox]);
        }
        Ok(rep)
    }

    // ------------------------------------------------------------- scoring

    /// One scoring batch (07 §3.4) and its alerts.
    pub fn score_once(&self) -> crate::db::Result<BatchReport> {
        let config = self.db.read(AiConfig::load)?;
        let now = self.now();
        let rep = {
            let mut st = lock(&self.scorer);
            let ai_cx = AiCtx {
                config: &config,
                keyring: &self.keyring,
                factory: self.ai_factory.as_ref(),
            };
            scoring::run_batch(&self.db, &ai_cx, now, self.tz, &mut st)?
        };
        if rep.changed() {
            self.changed(&[Event::Inbox, Event::Pill]);
            self.dispatch_alerts(rep.events.clone());
        }
        if rep.more {
            *lock(&self.score_due) = Some(Instant::now() + scoring::DEBOUNCE);
        }
        Ok(rep)
    }

    /// Ask for a scoring batch after the debounce (a poll, Re-score, the
    /// backend becoming ready, the local-only switch flipping).
    pub fn kick_scoring(&self) {
        *lock(&self.score_due) = Some(Instant::now() + scoring::DEBOUNCE);
    }

    pub fn scorer_counters(&self) -> scoring::Counters {
        lock(&self.scorer).counters.clone()
    }

    // -------------------------------------------------------------- alerts

    /// Run events through the alert pipeline (07 §8.7) and hand the
    /// result to the app. Never blocks a poll write or a hook decision.
    pub fn dispatch_alerts(&self, events: Vec<AlertEvent>) {
        if events.is_empty() {
            return;
        }
        let env = match self.environment() {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!(target: "mode::alerts", error = %e, "alert environment unavailable");
                return;
            }
        };
        let now = self.now();
        let tz = self.tz;
        for ev in events {
            let env = env.clone();
            let r = self.db.write(move |c| {
                repo::tx(c, |tx| {
                    with_tz(tx, tz, |z| alerts::evaluate(tx, &ev, &env, now, z, false))
                })
            });
            match r {
                Ok(mut out) => {
                    if out.overlay_only || (out.sound.is_none() && out.notification.is_none()) {
                        continue;
                    }
                    if out.notification.is_some() && !lock(&self.os_rate).admit(now) {
                        // coalesced into "n more urgent items" (flush_notifications)
                        out.notification = None;
                        if out.sound.is_none() {
                            continue;
                        }
                    }
                    tracing::info!(target: "mode::alerts", event = %out.event, rules = out.rules.len(), sound = out.sound.is_some(), os = out.notification.is_some(), "alert");
                    self.send_alert(AlertCommand::Fired(out));
                }
                Err(e) => {
                    tracing::warn!(target: "mode::alerts", error = %e, "alert evaluation failed")
                }
            }
        }
    }

    /// Post the "n more urgent items" summary of notifications coalesced by
    /// the one-per-second limit (07 §8.6 "Rate"); `run` calls it every tick.
    pub fn flush_notifications(&self) {
        let now = self.now();
        let summary = lock(&self.os_rate).flush(now);
        if let Some(n) = summary {
            self.send_alert(AlertCommand::Fired(AlertOutcome {
                event: "message.received".into(),
                item_key: n.identifier.clone(),
                rules: Vec::new(),
                sound: None,
                notification: Some(n),
                overlay_only: false,
            }));
        }
    }

    /// An event from elsewhere (sessions, notes): the app forwards it.
    pub fn alert(&self, event: AlertEvent) {
        self.dispatch_alerts(vec![event]);
    }

    /// Withdraw the OS notifications of a resolved item (07 §8.6).
    pub fn withdraw(&self, target_kind: &str, target_id: &str) {
        if let Ok(ids) = self
            .db
            .read(|c| alerts::withdraw_keys(c, target_kind, target_id))
            && !ids.is_empty()
        {
            self.send_alert(AlertCommand::Withdraw { identifiers: ids });
        }
    }

    fn calendar_alerts(&self) {
        let now = self.now();
        match self.db.read(|c| alerts::due_calendar_events(c, now)) {
            Ok(ev) => self.dispatch_alerts(ev),
            Err(e) => tracing::warn!(target: "mode::alerts", error = %e, "calendar alerts"),
        }
    }

    fn usage_alerts(&self) {
        let now = self.now();
        let tz = self.tz;
        let r = self.db.read(|c| {
            let s = AiSettings::load(c)?;
            let b = with_tz(c, tz, |z| ai::budget(c, now, z, s.budget_tokens_month))?;
            let month = with_tz(c, tz, |z| {
                let d = z.local_date(now);
                format!("{:04}-{:02}", d.year(), u8::from(d.month()))
            });
            alerts::usage_events(c, b.used_month, b.limit_month, &month)
        });
        if let Ok(ev) = r {
            self.dispatch_alerts(ev);
        }
    }

    /// The sweep S2 (07 §6): fire due snoozes; resurfaced items re-enter Q1.
    pub fn sweep_snoozes(&self) -> crate::db::Result<usize> {
        let now = self.now();
        let device = self.db.device_id().to_string();
        let fired = self
            .db
            .write(move |c| repo::tx(c, |tx| urgency::snooze::sweep(tx, &device, now)))?;
        if !fired.is_empty() {
            self.changed(&[Event::Inbox, Event::Pill, Event::Agenda]);
        }
        Ok(fired.len())
    }

    /// At launch: refresh Slack tokens older than 20 days (06 §8.9).
    pub fn refresh_aging_tokens(&self) -> usize {
        let now = self.now();
        let accounts = self
            .db
            .read(|c| repo::query::<Account>(c, "provider = 'slack' AND status = 'active'", &[]))
            .unwrap_or_default();
        let mut n = 0;
        for a in accounts {
            let t = self.tokens_for(&a);
            let aging = self
                .keyring
                .get_json::<oauth::TokenSet>(&a.keyring_ref)
                .ok()
                .flatten()
                .is_some_and(|s| s.refresh_token_aging(now));
            if aging {
                match t.refresh_now(true) {
                    Ok(_) => n += 1,
                    Err(e) => {
                        if let Some(tr) = health::transition_for(&e, 0) {
                            let id = a.id.clone();
                            let _ = self.db.write(move |c| health::transition(c, &id, &tr, now));
                        }
                    }
                }
            }
        }
        n
    }

    /// Cached Slack channel list for the picker (10 min).
    pub fn slack_channels(
        &self,
        account_id: &str,
        refresh: bool,
    ) -> Result<Vec<super::SourceCandidate>, ConnectorError> {
        if !refresh
            && let Some((at, list)) = lock(&self.channel_cache).get(account_id)
            && at.elapsed() < DISCOVERY_EVERY
        {
            return Ok(list.clone());
        }
        let account = self
            .db
            .read(|c| repo::get_by_id::<Account>(c, account_id))?
            .ok_or(ConnectorError::NotFound)?;
        if account.provider != "slack" {
            return Err(ConnectorError::Unsupported);
        }
        let tokens = self.tokens_for(&account);
        let api = Api::new(self.http.as_ref(), &tokens, 20);
        let list = self.slack.channels(&api)?;
        lock(&self.channel_cache).insert(account_id.to_string(), (Instant::now(), list.clone()));
        Ok(list)
    }

    /// Fetch a body on demand (06 §2.1 `fetch_body`); re-persisted only
    /// inside the retention window.
    pub fn fetch_body(&self, message_id: &str) -> Result<super::FetchedBody, ConnectorError> {
        let m = self
            .db
            .read(|c| repo::get_by_id::<repo::Message>(c, message_id))?
            .ok_or(ConnectorError::NotFound)?;
        let account = self
            .db
            .read(|c| repo::get_by_id::<Account>(c, &m.account_id))?
            .ok_or(ConnectorError::NotFound)?;
        let provider = Provider::parse(&account.provider).unwrap_or(Provider::Google);
        let settings = self.db.read(ConnectorSettings::load)?;
        let tokens = self.tokens_for(&account);
        let cx = SyncCx {
            db: &self.db,
            api: Api::new(self.http.as_ref(), &tokens, 5),
            account: &account,
            settings: &settings,
            now: self.now(),
            tz: self.tz,
        };
        let body = self.adapter(None, provider).fetch_body(&cx, &m)?;
        let keep = crate::db::parse_timestamp(&m.received_at)
            .is_some_and(|r| self.now() - r <= time::Duration::days(settings.message_body_days));
        if keep {
            let (id, b, until) = (
                m.id.clone(),
                body.clone(),
                format_timestamp(self.now() + time::Duration::days(settings.message_body_days)),
            );
            self.db.write(move |c| {
                c.execute(
                    "UPDATE messages SET body_text = ?2, body_truncated = ?3, body_retained_until = ?4 WHERE id = ?1",
                    rusqlite::params![id, b.text, b.truncated, until],
                )?;
                Ok(())
            })?;
        }
        Ok(body)
    }

    // ---------------------------------------------------------------- loop

    /// The background loop. Runs until the task is dropped.
    pub async fn run(self: Arc<Self>) {
        let me = Arc::clone(&self);
        let _ = tokio::task::spawn_blocking(move || {
            me.refresh_aging_tokens();
            let _ = me.sweep_snoozes();
            me.wake();
        })
        .await;
        let mut tick = tokio::time::interval(super::scheduler::TICK);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut last_drain = Instant::now() - DRAIN_EVERY;
        let mut last_minute = Instant::now();
        let mut last_cal = Instant::now() - CALENDAR_ALERTS_EVERY;
        let mut last_users = Instant::now();
        let mut today = String::new();
        loop {
            tick.tick().await;
            self.flush_notifications();
            let me = Arc::clone(&self);
            let _ = tokio::task::spawn_blocking(move || {
                if let Err(e) = me.poll_once() {
                    tracing::warn!(target: "mode::connectors", error = %e, "poll pass failed");
                }
            })
            .await;
            let due = { lock(&self.score_due).is_some_and(|t| Instant::now() >= t) };
            if due {
                *lock(&self.score_due) = None;
                let me = Arc::clone(&self);
                let _ = tokio::task::spawn_blocking(move || me.score_once()).await;
            }
            if last_drain.elapsed() >= DRAIN_EVERY {
                last_drain = Instant::now();
                let me = Arc::clone(&self);
                let _ = tokio::task::spawn_blocking(move || me.drain_writebacks()).await;
            }
            if last_cal.elapsed() >= CALENDAR_ALERTS_EVERY {
                last_cal = Instant::now();
                let me = Arc::clone(&self);
                let _ = tokio::task::spawn_blocking(move || me.calendar_alerts()).await;
            }
            if last_minute.elapsed() >= MINUTE {
                last_minute = Instant::now();
                let me = Arc::clone(&self);
                let _ = tokio::task::spawn_blocking(move || {
                    let _ = me.sweep_snoozes();
                    me.usage_alerts();
                    // Slack DM discovery
                    let slack: Vec<String> = me
                        .db
                        .read(|c| {
                            let mut st = c.prepare("SELECT id FROM accounts WHERE provider = 'slack' AND status = 'active'")?;
                            Ok(st.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<Vec<String>>>()?)
                        })
                        .unwrap_or_default();
                    for id in slack {
                        let stale = lock(&me.discovered)
                            .get(&id)
                            .is_none_or(|t| t.elapsed() >= DISCOVERY_EVERY);
                        if stale && let Err(e) = me.discover(&id) {
                            tracing::debug!(target: "mode::connectors", error = %e, "slack discovery");
                        }
                    }
                })
                .await;
                // agenda sections and the quiet state move with the clock
                let now = self.now();
                let day = format_timestamp(now)[..10].to_string();
                if day != today {
                    today = day;
                }
                self.changed(&[Event::Agenda, Event::Quiet]);
            }
            if last_users.elapsed() >= USERS_CACHE_TTL {
                last_users = Instant::now();
                self.slack.clear_cache();
            }
        }
    }
}

/// Re-exported for tests that drive a flow without a browser.
pub fn callback_url(redirect_uri: &str, state: &str, code: &str) -> String {
    format!("{redirect_uri}?code={code}&state={state}")
}
