//! SRV-13: one event stream for every workspace, the job registry and background sync.
//!
//! `GET /api/v1/events` sends each workspace's freshness on connect, then every event as it
//! happens; there is no replay log because the current state is always sent first. While at least
//! one stream is open, a ticker looks at every workspace once a second: a new index generation
//! becomes `index.changed`, a changed freshness becomes `freshness`, and a workspace that is behind
//! gets a background sync (`ensure_fresh`, D1). With no stream open nothing polls.

use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{Path, State};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use cartograph_api::events::ServerEvent;
use cartograph_api::types::{
    Actor, ApiError, ErrorCode, Freshness, IndexState, Job, JobKind, JobRequest, JobState, Problem,
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, broadcast};

use crate::routes::{App, body, error, ok};
use crate::sync;

/// At most 16 streams (SRV-13).
pub const MAX_STREAMS: usize = 16;
/// A heartbeat comment this often.
const HEARTBEAT: Duration = Duration::from_secs(15);
/// The ticker's period while a stream is open.
const TICK: Duration = Duration::from_secs(1);
/// Progress events per job: at most 2 a second.
const PROGRESS_EVERY: Duration = Duration::from_millis(500);
/// One background sync attempt per workspace per 2 s, so a burst of reads can't fork-bomb (IDX-15).
const SPAWN_EVERY: Duration = Duration::from_secs(2);

pub struct Live {
    tx: broadcast::Sender<(u64, ServerEvent)>,
    seq: AtomicU64,
    streams: Arc<Semaphore>,
    ticking: AtomicBool,
    jobs: Mutex<Jobs>,
}

#[derive(Default)]
struct Jobs {
    all: HashMap<String, Entry>,
    /// The running job of each workspace.
    running: HashMap<String, String>,
    /// The last background start per workspace (spawn rate limit).
    spawned: HashMap<String, Instant>,
    /// What the ticker last saw per workspace: generation and freshness.
    seen: HashMap<String, (u64, String)>,
}

/// D1: the states a background sync brings current. Rewritten history is one of them: a sync
/// replays from the merge base (IDX-19), so it ends current like any other. A repo force-pushed in
/// a loop is bounded by the spawn rate, the dirty-round cap (IDX-15) and the fact cache.
fn wants_sync(state: IndexState) -> bool {
    matches!(state, IndexState::Behind | IndexState::Rewritten)
}

struct Entry {
    job: Job,
    cancel: Arc<AtomicBool>,
    /// Cancelling a first index (POST /workspaces) also forgets the workspace.
    first_index: bool,
    /// The cartograph.yaml its request wrote where there was none: a cancelled first index removes
    /// it too. One that was already there (committed) is never removed.
    created: Option<std::path::PathBuf>,
}

impl Default for Live {
    fn default() -> Self {
        Self {
            tx: broadcast::channel(256).0,
            seq: AtomicU64::new(0),
            streams: Arc::new(Semaphore::new(MAX_STREAMS)),
            ticking: AtomicBool::new(false),
            jobs: Mutex::default(),
        }
    }
}

impl Live {
    pub fn emit(&self, e: ServerEvent) {
        let id = self.seq.fetch_add(1, Ordering::Relaxed) + 1;
        // No receiver means no stream is open: nothing to do.
        let _ = self.tx.send((id, e));
    }

    fn jobs(&self) -> std::sync::MutexGuard<'_, Jobs> {
        self.jobs.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn job(&self, id: &str) -> Option<Job> {
        self.jobs().all.get(id).map(|e| e.job.clone())
    }
}

fn now() -> String {
    let s = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    cartograph_query::rfc3339(s)
}

fn job_id() -> String {
    let mut b = [0u8; 8];
    getrandom::fill(&mut b).expect("the OS random source");
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Starts a job of `kind` on `ws`, or, when one is already running there, asks it for one more
/// round (D2) and returns that one. The work runs on the blocking pool.
/// At start (`serve`, `open`): every registered index in an older format is migrated under the
/// writer lock, and one with no migration path gets a sync, which rebuilds it (B3). Until then the
/// workspace is listed with its state, not dropped.
pub(crate) fn upgrade_all(app: &Arc<App>) {
    for ws in sync::workspace_ids(&app.data_dir) {
        let w = cartograph_index::sync::Workspace {
            dir: cartograph_core::paths::workspace_dir(&app.data_dir, &ws),
            ..Default::default()
        };
        match cartograph_index::sync::migrate_if_needed(&w) {
            Ok(cartograph_index::sync::Upgrade::Rebuild) => {
                start(app, &ws, JobKind::Sync, Actor::Background, false);
            }
            Ok(_) => {}
            Err(e) => {
                use cartograph_core::Explain;
                tracing::warn!(ws, error = %e.what(), why = %e.why(), "can't check the index format");
            }
        }
    }
}

pub fn start(app: &Arc<App>, ws: &str, kind: JobKind, by: Actor, first_index: bool) -> Job {
    start_with(app, ws, kind, by, first_index, None)
}

/// [`start`], recording the manifest the first index's request created.
pub fn start_with(
    app: &Arc<App>,
    ws: &str,
    kind: JobKind,
    by: Actor,
    first_index: bool,
    created: Option<std::path::PathBuf>,
) -> Job {
    let (job, cancel) = {
        let mut jobs = app.live.jobs();
        if let Some(running) = jobs.running.get(ws).and_then(|id| jobs.all.get(id)) {
            // Asked again while a cancelled job is still stopping: the job is taken back and runs to
            // its end for this request. Handed out still cancelled, it would end this request as
            // cancelled too, and a first index would then forget the workspace just asked for again.
            if running.cancel.swap(false, Ordering::Relaxed) {
                sync::withdraw_cancel(&app.data_dir, ws);
            }
            sync::request_round(&app.data_dir, ws);
            return running.job.clone();
        }
        let job = Job {
            id: job_id(),
            ws: ws.to_owned(),
            kind,
            by,
            state: JobState::Running,
            started_at: now(),
            took_ms: None,
            error: None,
        };
        let cancel = Arc::new(AtomicBool::new(false));
        jobs.running.insert(ws.to_owned(), job.id.clone());
        jobs.all.insert(
            job.id.clone(),
            Entry {
                job: job.clone(),
                cancel: cancel.clone(),
                first_index,
                created,
            },
        );
        (job, cancel)
    };
    app.live.emit(ServerEvent::JobStarted {
        job_id: job.id.clone(),
        ws: job.ws.clone(),
        kind,
        by,
    });
    let app2 = app.clone();
    let id = job.id.clone();
    let ws = ws.to_owned();
    tokio::task::spawn_blocking(move || run(&app2, &id, &ws, kind, &cancel));
    job
}

fn run(app: &Arc<App>, id: &str, ws: &str, kind: JobKind, cancel: &AtomicBool) {
    let t0 = Instant::now();
    let mut last = Instant::now() - PROGRESS_EVERY;
    let progress = |stage: &str, done: u64, total: u64| {
        if last.elapsed() >= PROGRESS_EVERY || done == total {
            last = Instant::now();
            app.live.emit(ServerEvent::JobProgress {
                job_id: id.to_owned(),
                stage: stage.to_owned(),
                done,
                total,
                repo: None,
                eta_s: None,
            });
        }
    };
    let result = guarded(|| sync::run(&app.data_dir, ws, kind, progress, cancel));
    let cancelled = cancel.load(Ordering::Relaxed);
    let took_ms = t0.elapsed().as_millis() as u64;
    let (state, problem) = match (&result, cancelled) {
        (_, true) => (
            JobState::Cancelled,
            Some(Problem {
                what: "the job was cancelled".into(),
                why: "someone stopped it; the index keeps what was synced before the stop".into(),
                action: "start it again when you're ready; it carries on from there".into(),
            }),
        ),
        (Ok(_), false) => (JobState::Done, None),
        (Err(p), false) => (JobState::Failed, Some(p.clone())),
    };
    let (first_index, created) = {
        let mut jobs = app.live.jobs();
        jobs.running.remove(ws);
        jobs.all
            .get_mut(id)
            .map(|e| {
                e.job.state = state;
                e.job.took_ms = Some(took_ms);
                e.job.error = problem.clone();
                (Some(e.first_index), e.created.take())
            })
            .unwrap_or((None, None))
    };
    if cancelled && first_index == Some(true) {
        // A cancelled first index leaves nothing behind: no registry entry, no index folder, no
        // manifest its request wrote. The cached store goes first: its open connections would keep
        // Windows from deleting the file.
        app.forget_store(ws);
        sync::forget(&app.data_dir, ws);
        if let Some(manifest) = created {
            let _ = std::fs::remove_file(manifest);
        }
        app.live.emit(ServerEvent::WorkspacesChanged {});
    }
    let generation = result.as_ref().ok().copied();
    app.live.emit(ServerEvent::JobFinished {
        job_id: id.to_owned(),
        ok: state == JobState::Done,
        generation,
        took_ms,
        error: problem,
    });
    if state == JobState::Done && first_index == Some(true) {
        app.live.emit(ServerEvent::WorkspacesChanged {});
    }
    // The ticker would notice within a second; say it now so the map refetches at once.
    app.tick_one(ws);
}

// ---------------------------------------------------------------- the ticker

impl App {
    /// Looks at one workspace: a new generation or a changed freshness becomes an event, and a
    /// workspace that is behind gets a background sync.
    pub(crate) fn tick_one(self: &Arc<Self>, ws: &str) {
        let Some((generation, fact_hash, fresh)) = self.observe(ws) else {
            return;
        };
        let key = serde_json::to_string(&fresh).unwrap_or_default();
        let (gen_changed, fresh_changed) = {
            let mut jobs = self.live.jobs();
            let prev = jobs.seen.insert(ws.to_owned(), (generation, key.clone()));
            match prev {
                // The first look sends what it sees. A stream got a state on connect, but a sync
                // that ended before this tick changed it, and job.finished alone can't tell a tab
                // that never saw the job start. Repeating an unchanged state costs one event.
                None => (false, true),
                Some((g, k)) => (g != generation, k != key),
            }
        };
        if gen_changed {
            self.live.emit(ServerEvent::IndexChanged {
                ws: ws.to_owned(),
                generation,
                fact_hash: fact_hash.unwrap_or_default(),
            });
        }
        if fresh_changed {
            self.live.emit(ServerEvent::Freshness {
                ws: ws.to_owned(),
                freshness: fresh.clone(),
            });
        }
        self.ensure_fresh(ws, &fresh);
    }

    /// D1: a workspace that is behind or rewritten, with `auto_sync = background` and no job
    /// running, gets a sync, at most one start per 2 s.
    pub(crate) fn ensure_fresh(self: &Arc<Self>, ws: &str, fresh: &Freshness) {
        if !wants_sync(fresh.state) || !sync::auto_sync(&self.data_dir) {
            return;
        }
        {
            let mut jobs = self.live.jobs();
            if jobs.running.contains_key(ws)
                || jobs
                    .spawned
                    .get(ws)
                    .is_some_and(|t| t.elapsed() < SPAWN_EVERY)
            {
                return;
            }
            jobs.spawned.insert(ws.to_owned(), Instant::now());
        }
        start(self, ws, JobKind::Sync, Actor::Background, false);
    }

    /// The workspace's generation, fact hash and freshness, read on the blocking thread it's
    /// called from.
    fn observe(&self, ws: &str) -> Option<(u64, Option<String>, Freshness)> {
        let store = self.store(ws).ok()?;
        let snap = store.snapshot(cartograph_query::Limits::default()).ok()?;
        let fresh = sync::freshness(&self.data_dir, ws, &snap).ok()?;
        Some((snap.generation, snap.fact_hash.clone(), fresh))
    }

    /// Every workspace's current freshness, for a new stream.
    fn current(&self) -> Vec<ServerEvent> {
        self.workspace_ids()
            .into_iter()
            .filter_map(|ws| {
                let (_, _, freshness) = self.observe(&ws)?;
                Some(ServerEvent::Freshness { ws, freshness })
            })
            .collect()
    }
}

/// Runs once a second while any stream is open; stops when the last one closes.
fn ensure_ticker(app: &Arc<App>) {
    if app.live.ticking.swap(true, Ordering::AcqRel) {
        return;
    }
    let app = app.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(TICK).await;
            if app.live.streams.available_permits() == MAX_STREAMS {
                app.live.ticking.store(false, Ordering::Release);
                // A stream that opened between the check and the store starts its own ticker.
                if app.live.streams.available_permits() == MAX_STREAMS
                    || app.live.ticking.swap(true, Ordering::AcqRel)
                {
                    return;
                }
            }
            let a = app.clone();
            let _ = tokio::task::spawn_blocking(move || {
                for ws in a.workspace_ids() {
                    a.tick_one(&ws);
                }
            })
            .await;
        }
    });
}

// ---------------------------------------------------------------- routes

fn sse_event(id: u64, e: &ServerEvent) -> Event {
    let v = serde_json::to_value(e).unwrap_or_default();
    Event::default()
        .id(id.to_string())
        .event(v["event"].as_str().unwrap_or("message"))
        .data(v["data"].to_string())
}

/// `GET /api/v1/events`.
pub async fn events(State(app): State<Arc<App>>) -> Response {
    let Ok(permit) = app.live.streams.clone().try_acquire_owned() else {
        return error(&ApiError::new(
            ErrorCode::TooManyRequests,
            "too many event streams",
            "the server keeps at most 16 streams open",
            "close other Cartograph tabs; one tab shares its stream with the rest",
        ));
    };
    // Subscribe before reading the current state, so nothing between the two is lost.
    let rx = app.live.tx.subscribe();
    let a = app.clone();
    let first = tokio::task::spawn_blocking(move || a.current())
        .await
        .unwrap_or_default();
    ensure_ticker(&app);
    let seq = app.live.seq.load(Ordering::Relaxed);
    let first: Vec<Event> = first.iter().map(|e| sse_event(seq, e)).collect();
    struct S {
        first: std::vec::IntoIter<Event>,
        rx: broadcast::Receiver<(u64, ServerEvent)>,
        _permit: OwnedSemaphorePermit,
    }
    let stream = futures_util::stream::unfold(
        S {
            first: first.into_iter(),
            rx,
            _permit: permit,
        },
        |mut s| async move {
            if let Some(e) = s.first.next() {
                return Some((Ok::<_, Infallible>(e), s));
            }
            match s.rx.recv().await {
                Ok((id, e)) => Some((Ok(sse_event(id, &e)), s)),
                // A client too slow to keep 256 events is dropped; it reconnects and gets the
                // current state again.
                // ponytail: a client that stops reading entirely holds its slot until the TCP
                // connection dies; the 30 s stall timeout needs a write deadline on the socket.
                Err(_) => None,
            }
        },
    );
    Sse::new(stream)
        .keep_alive(KeepAlive::new().interval(HEARTBEAT).text(""))
        .into_response()
}

/// `POST /api/v1/w/{ws}/jobs`.
pub async fn post_job(
    State(app): State<Arc<App>>,
    Path(ws): Path<String>,
    bytes: axum::body::Bytes,
) -> Response {
    let req: JobRequest = match body(&bytes) {
        Ok(r) => r,
        Err(e) => return e,
    };
    if !app.workspace_ids().contains(&ws) {
        return error(&ApiError::new(
            ErrorCode::NotFound,
            "no such workspace",
            "the registry has no workspace with this id",
            "list workspaces with GET /api/v1/workspaces",
        ));
    }
    // ponytail: `repo` narrows nothing yet; a sync covers the workspace (IDX-14 syncs per repo).
    let _ = req.repo;
    if other_kind_running(&app, &ws, req.kind) {
        return error(&ApiError::new(
            ErrorCode::Conflict,
            "a job of another kind is running on this workspace",
            "one job runs per workspace, and a sync and a rebuild don't merge",
            "wait for it to finish (GET /api/v1/jobs/{id}), then ask again",
        ));
    }
    ok(0, start(&app, &ws, req.kind, Actor::App, false))
}

/// Whether a job of another kind runs on `ws`. One of the same kind takes a request as one more
/// round (D2); a rebuild asked for during a sync would otherwise quietly become a sync round.
fn other_kind_running(app: &App, ws: &str, kind: JobKind) -> bool {
    let jobs = app.live.jobs();
    jobs.running
        .get(ws)
        .and_then(|id| jobs.all.get(id))
        .is_some_and(|e| std::mem::discriminant(&e.job.kind) != std::mem::discriminant(&kind))
}

/// `GET /api/v1/jobs/{id}`.
pub async fn get_job(State(app): State<Arc<App>>, Path(id): Path<String>) -> Response {
    match app.live.job(&id) {
        Some(j) => ok(0, j),
        None => no_job(),
    }
}

/// `DELETE /api/v1/jobs/{id}`: asks the job to stop (E5). The previous index stays readable.
pub async fn cancel_job(State(app): State<Arc<App>>, Path(id): Path<String>) -> Response {
    let found = {
        let jobs = app.live.jobs();
        jobs.all.get(&id).map(|e| (e.job.clone(), e.cancel.clone()))
    };
    let Some((job, cancel)) = found else {
        return no_job();
    };
    if job.state == JobState::Running {
        cancel.store(true, Ordering::Relaxed);
        sync::request_cancel(&app.data_dir, &job.ws);
    }
    ok(0, job)
}

/// Runs a job's body and turns a panic into a failed job. The cleanup after it (the registry's
/// running entry, job.finished) must run whatever happens, or the job stays "running" and the
/// workspace never syncs again until the server restarts.
fn guarded<T>(body: impl FnOnce() -> Result<T, Problem>) -> Result<T, Problem> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(body)).unwrap_or_else(|_| {
        Err(Problem {
            what: "the job stopped on an internal error".into(),
            why: "the indexer panicked; the index keeps what was synced before".into(),
            action: "run `cartograph sync -v` in a terminal to see the error, and report it".into(),
        })
    })
}

/// Shutdown: asks every running job to stop after its current batch, so the process can exit
/// instead of waiting for a long sync (the blocking pool outlives the listener). Shutting down
/// isn't the user cancelling, so a first index keeps its workspace and resumes on the next sync.
pub(crate) fn stop_all(app: &App) {
    let mut jobs = app.live.jobs();
    let running: Vec<(String, String)> = jobs
        .running
        .iter()
        .map(|(ws, id)| (ws.clone(), id.clone()))
        .collect();
    for (ws, id) in running {
        if let Some(e) = jobs.all.get_mut(&id) {
            e.first_index = false;
            e.cancel.store(true, Ordering::Relaxed);
        }
        sync::request_cancel(&app.data_dir, &ws);
    }
}

fn no_job() -> Response {
    error(&ApiError::new(
        ErrorCode::NotFound,
        "no such job",
        "jobs are kept in memory while the server runs",
        "start a sync with POST /api/v1/w/{ws}/jobs",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(data: &std::path::Path) -> App {
        let info = crate::ServerInfo {
            pid: 1,
            host: std::net::Ipv4Addr::LOCALHOST.into(),
            port: 1,
            token: "t".into(),
            version: "0".into(),
        };
        App::new(data.to_path_buf(), None, &info)
    }

    fn running(app: &App, ws: &str, id: &str, first_index: bool) -> Arc<AtomicBool> {
        let cancel = Arc::new(AtomicBool::new(false));
        let job = Job {
            id: id.into(),
            ws: ws.into(),
            kind: JobKind::Sync,
            by: Actor::App,
            state: JobState::Running,
            started_at: now(),
            took_ms: None,
            error: None,
        };
        let mut jobs = app.live.jobs();
        jobs.running.insert(ws.into(), id.into());
        let cancel2 = cancel.clone();
        jobs.all.insert(
            id.into(),
            Entry {
                job,
                cancel: cancel2,
                first_index,
                created: None,
            },
        );
        cancel
    }

    #[test]
    fn a_panic_in_a_job_ends_it_as_failed() {
        let r: Result<(), Problem> = guarded(|| panic!("an invariant broke"));
        assert!(r.unwrap_err().what.contains("internal error"));
        assert_eq!(guarded(|| Ok::<_, Problem>(7)).unwrap(), 7);
    }

    #[test]
    fn a_rebuild_doesnt_join_a_running_sync() {
        let d = tempfile::tempdir().unwrap();
        let app = app(d.path());
        running(&app, "0000000a", "j1", false);
        assert!(other_kind_running(&app, "0000000a", JobKind::Rebuild));
        assert!(!other_kind_running(&app, "0000000a", JobKind::Sync));
        assert!(!other_kind_running(&app, "0000000b", JobKind::Rebuild));
    }

    #[test]
    fn the_host_allow_list_names_the_bound_address() {
        let d = tempfile::tempdir().unwrap();
        let info = crate::ServerInfo {
            pid: 1,
            host: "127.0.0.2".parse().unwrap(),
            port: 4000,
            token: "t".into(),
            version: "0".into(),
        };
        let app = App::new(d.path().to_path_buf(), None, &info);
        // `serve --host 127.0.0.2` prints http://127.0.0.2:4000, so that Host must pass the guard.
        assert_eq!(app.origin, "http://127.0.0.2:4000");
        assert!(
            app.hosts.iter().any(|h| h == "127.0.0.2:4000"),
            "{:?}",
            app.hosts
        );
    }

    #[test]
    fn shutdown_stops_every_running_job_and_keeps_a_first_index() {
        let d = tempfile::tempdir().unwrap();
        let app = app(d.path());
        let ws = cartograph_core::paths::workspace_dir(d.path(), "0000000a");
        std::fs::create_dir_all(&ws).unwrap();
        let cancel = running(&app, "0000000a", "j1", true);
        stop_all(&app);
        assert!(cancel.load(Ordering::Relaxed), "the job is asked to stop");
        assert!(
            ws.join("cancel").exists(),
            "so is its writer, between batches"
        );
        assert!(
            !app.live.jobs().all["j1"].first_index,
            "a shutdown doesn't forget a first index"
        );
    }

    #[test]
    fn rewritten_history_gets_a_background_sync_like_behind() {
        assert!(super::wants_sync(IndexState::Behind));
        assert!(
            super::wants_sync(IndexState::Rewritten),
            "replayed from the merge base, not left for Sync now"
        );
        assert!(!super::wants_sync(IndexState::Current));
    }
}
