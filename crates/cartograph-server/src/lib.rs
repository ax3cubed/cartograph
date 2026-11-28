//! The local server behind `cartograph open` (SRV-05).
//!
//! It binds a loopback address only and serves one origin, `http://127.0.0.1:<port>`. Every
//! request passes the Host allow list, the Origin check and `Sec-Fetch-Site`; every `/api/*`
//! request also needs `Authorization: Bearer <token>`. The token reaches the page in the URL
//! fragment, which no browser sends to a server or in a Referer. There are no cookies and no CORS.
//! Nothing here opens a connection to another machine (G3).
//!
//! The CLI calls [`serve`]; tests call [`bind`] and [`Server::run`].

// Handlers return their error as a ready Response; it is built once per failed request.
#![allow(clippy::result_large_err)]

pub mod activity;
pub mod agents;
mod corrections;
mod guard;
mod live;
pub mod mcp;
mod reads;
mod routes;
mod sync;
pub mod ui;
mod writes;

use std::future::Future;
use std::io::Write;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use cartograph_core::Explain;

use serde::{Deserialize, Serialize};
/// A registered workspace as sync and the freshness check see it (registry plus manifest); the
/// CLI's `status` uses the same lookup.
pub use sync::{workspace, workspace_checked, workspace_ids};

/// What `cartograph open` passes in.
#[derive(Debug, Clone)]
pub struct ServeOptions {
    /// The data folder (B1): workspaces are read from `<data>/workspaces/<id>/`, and `server.json`
    /// is written here.
    pub data_dir: PathBuf,
    /// `--host`: a loopback address or `localhost`. Anything else is refused.
    pub host: Option<String>,
    /// `--port`. Without it the last port in `server.json` is reused when free, else any port.
    pub port: Option<u16>,
    /// The folder `fs/list` may browse (SRV-14). `None`: the user's home from the environment.
    pub home: Option<PathBuf>,
}

/// `server.json` (0600): how a second `cartograph open` finds the running server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerInfo {
    pub pid: u32,
    pub host: IpAddr,
    pub port: u16,
    pub token: String,
    pub version: String,
}

impl ServerInfo {
    /// `http://127.0.0.1:<port>`: the one origin the server answers.
    pub fn origin(&self) -> String {
        format!("http://{}", SocketAddr::new(self.host, self.port))
    }

    /// The address to open: the token travels in the fragment, never the query string.
    pub fn url(&self) -> String {
        format!("{}/#k={}", self.origin(), self.token)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ServeError {
    #[error("{host} is not a loopback address")]
    NotLoopback { host: String },
    #[error("can't listen on port {port}")]
    Bind {
        port: u16,
        #[source]
        source: std::io::Error,
    },
    #[error("can't write server.json in the data folder")]
    Io(#[source] std::io::Error),
}

impl Explain for ServeError {
    fn why(&self) -> String {
        match self {
            Self::NotLoopback { .. } => "Cartograph serves this machine only; a LAN or public \
                address would expose the index to other machines"
                .into(),
            Self::Bind { source, .. } | Self::Io(source) => source.to_string(),
        }
    }
    fn action(&self) -> String {
        match self {
            Self::NotLoopback { .. } => "leave --host out, or pass 127.0.0.1".into(),
            Self::Bind { .. } => "leave --port out to use any free port".into(),
            Self::Io(_) => "check the data folder's permissions".into(),
        }
    }
}

/// Parses `--host`: `127.0.0.1` (the default), any `127.x` address, `::1` or `localhost`.
pub fn loopback(host: Option<&str>) -> Result<IpAddr, ServeError> {
    let refused = || ServeError::NotLoopback {
        host: host.unwrap_or_default().into(),
    };
    let ip = match host {
        None | Some("localhost") => IpAddr::V4(Ipv4Addr::LOCALHOST),
        Some(h) => h
            .trim_start_matches('[')
            .trim_end_matches(']')
            .parse::<IpAddr>()
            .map_err(|_| refused())?,
    };
    if !ip.is_loopback() {
        return Err(refused());
    }
    Ok(ip)
}

const SERVER_JSON: &str = "server.json";

/// The server recorded in `server.json`, if it still accepts connections.
// ponytail: a connect proves something listens on the port, not that it's ours; a GET
// /api/version with the token would. Enough while only `cartograph open` writes the file.
pub fn running(data_dir: &Path) -> Option<ServerInfo> {
    let info: ServerInfo =
        serde_json::from_slice(&std::fs::read(data_dir.join(SERVER_JSON)).ok()?).ok()?;
    let addr = SocketAddr::new(info.host, info.port);
    TcpStream::connect_timeout(&addr, Duration::from_millis(300)).ok()?;
    Some(info)
}

/// A bound listener, not yet serving.
pub struct Server {
    listener: tokio::net::TcpListener,
    info: ServerInfo,
    data_dir: PathBuf,
    home: Option<PathBuf>,
}

/// Binds the listener and writes `server.json`. Refuses a non-loopback host before binding.
pub async fn bind(opts: ServeOptions) -> Result<Server, ServeError> {
    let host = loopback(opts.host.as_deref())?;
    let last = std::fs::read(opts.data_dir.join(SERVER_JSON))
        .ok()
        .and_then(|b| serde_json::from_slice::<ServerInfo>(&b).ok())
        .map(|i| i.port);
    let listener = match (opts.port, last) {
        (Some(port), _) => tokio::net::TcpListener::bind((host, port))
            .await
            .map_err(|source| ServeError::Bind { port, source })?,
        // Port ruling: reuse the last port when it's free, so an open tab keeps working.
        (None, Some(port)) => match tokio::net::TcpListener::bind((host, port)).await {
            Ok(l) => l,
            Err(_) => any_port(host).await?,
        },
        (None, None) => any_port(host).await?,
    };
    let port = listener.local_addr().map_err(ServeError::Io)?.port();
    let info = ServerInfo {
        pid: std::process::id(),
        host,
        port,
        token: token(),
        version: env!("CARGO_PKG_VERSION").into(),
    };
    write_private(
        &opts.data_dir,
        SERVER_JSON,
        &serde_json::to_vec_pretty(&info).expect("plain data"),
    )
    .map_err(ServeError::Io)?;
    let home = opts.home.or_else(|| {
        std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .map(PathBuf::from)
    });
    Ok(Server {
        listener,
        info,
        data_dir: opts.data_dir,
        home,
    })
}

async fn any_port(host: IpAddr) -> Result<tokio::net::TcpListener, ServeError> {
    tokio::net::TcpListener::bind((host, 0))
        .await
        .map_err(|source| ServeError::Bind { port: 0, source })
}

impl Server {
    pub fn info(&self) -> &ServerInfo {
        &self.info
    }

    /// Serves until `shutdown` resolves, then removes `server.json`.
    pub async fn run(
        self,
        shutdown: impl Future<Output = ()> + Send + 'static,
    ) -> std::io::Result<()> {
        let state = Arc::new(routes::App::new(
            self.data_dir.clone(),
            self.home.clone(),
            &self.info,
        ));
        {
            let state = state.clone();
            tokio::task::spawn_blocking(move || live::upgrade_all(&state));
        }
        let app = routes::router(state.clone());
        // Our own accept loop instead of `axum::serve`, which has no header-read timeout: a client
        // that never finishes its headers is dropped after 10 s (HD-11 slow-loris).
        let svc = hyper_util::service::TowerToHyperService::new(app);
        let mut http = hyper::server::conn::http1::Builder::new();
        http.timer(hyper_util::rt::TokioTimer::new())
            .header_read_timeout(Duration::from_secs(10));
        let graceful = hyper_util::server::graceful::GracefulShutdown::new();
        tokio::pin!(shutdown);
        loop {
            tokio::select! {
                accepted = self.listener.accept() => match accepted {
                    Ok((stream, _)) => {
                        let conn = http.serve_connection(hyper_util::rt::TokioIo::new(stream), svc.clone());
                        let conn = graceful.watch(conn);
                        tokio::spawn(async move { let _ = conn.await; });
                    }
                    // Out of file descriptors and the like: back off instead of spinning.
                    Err(_) => tokio::time::sleep(Duration::from_millis(50)).await,
                },
                () = &mut shutdown => break,
            }
        }
        // Event streams never end on their own, so in-flight requests get one second.
        let _ = tokio::time::timeout(Duration::from_secs(1), graceful.shutdown()).await;
        let _ = std::fs::remove_file(self.data_dir.join(SERVER_JSON));
        live::stop_all(&state);
        Ok(())
    }
}

/// `cartograph open`: binds, calls `ready` with the server's address (the CLI opens the browser
/// or prints the URL), and serves until Ctrl-C. Blocks the calling thread.
pub fn serve(opts: ServeOptions, ready: impl FnOnce(&ServerInfo)) -> Result<(), ServeError> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(ServeError::Io)?;
    let out = rt.block_on(async {
        let server = bind(opts).await?;
        ready(server.info());
        server
            .run(async {
                let _ = tokio::signal::ctrl_c().await;
            })
            .await
            .map_err(ServeError::Io)
    });
    // Dropping the runtime would wait for every blocking job with no limit, and ctrl_c() has
    // taken over SIGINT, so a second Ctrl-C couldn't end it. Running jobs were asked to stop after
    // their current batch; one that hasn't by then is left behind (batches commit whole, IDX-13).
    rt.shutdown_timeout(SHUTDOWN_GRACE);
    out
}

/// How long `serve` waits for running jobs to stop after Ctrl-C.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(5);

/// 32 random bytes from the OS, base64url without padding.
///
/// Decided (phase 2 review): a new token on every start, never stored for reuse. A leaked URL dies
/// with the process; the cost is reopening the app (`cartograph open`) after a restart.
fn token() -> String {
    let mut b = [0u8; 32];
    getrandom::fill(&mut b).expect("the OS random source");
    base64url(&b)
}

fn base64url(bytes: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut s = String::with_capacity(bytes.len() * 4 / 3 + 2);
    for c in bytes.chunks(3) {
        let n = c
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, b)| n | u32::from(*b) << (16 - 8 * i));
        for i in 0..=c.len() {
            s.push(A[(n >> (18 - 6 * i) & 63) as usize] as char);
        }
    }
    s
}

/// Writes `name` in `dir` as 0600 through a temporary file and a rename. The folder is created
/// 0700 when missing.
// ponytail: Windows relies on the profile folder's ACL until SRV-26 sets one.
fn write_private(dir: &Path, name: &str, bytes: &[u8]) -> std::io::Result<()> {
    let mut b = std::fs::DirBuilder::new();
    b.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut b, 0o700);
    b.create(dir)?;
    let tmp = dir.join(format!("{name}.{}.tmp", std::process::id()));
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
    let mut f = o.open(&tmp)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    drop(f);
    std::fs::rename(&tmp, dir.join(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_loopback_hosts_are_accepted() {
        for ok in [
            None,
            Some("localhost"),
            Some("127.0.0.1"),
            Some("127.0.0.2"),
            Some("::1"),
            Some("[::1]"),
        ] {
            assert!(loopback(ok).unwrap().is_loopback(), "{ok:?}");
        }
        for bad in [
            "0.0.0.0",
            "::",
            "192.168.1.5",
            "10.0.0.1",
            "example.com",
            "",
            "127.0.0.1:80",
        ] {
            let e = loopback(Some(bad)).unwrap_err();
            assert!(matches!(e, ServeError::NotLoopback { .. }), "{bad}");
            assert!(e.why().contains("this machine only"));
        }
    }

    #[test]
    fn tokens_are_43_url_safe_characters_and_differ() {
        let (a, b) = (token(), token());
        assert_eq!(a.len(), 43);
        assert!(
            a.bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
        );
        assert_ne!(a, b);
        assert_eq!(base64url(b"\xfb\xff"), "-_8");
        assert_eq!(base64url(b"Man"), "TWFu");
    }

    /// An index in an older format is listed with its state, not dropped, and `serve` migrates
    /// it at start when a migration is registered.
    #[test]
    fn an_old_format_index_is_listed_then_migrated_at_start() {
        use cartograph_api::types::IndexState;
        let d = tempfile::tempdir().unwrap();
        let data = d.path().to_owned();
        let (old, none) = ("0000000a", "0000000b");
        for (ws, format) in [(old, 1), (none, 0)] {
            let dir = cartograph_core::paths::workspace_dir(&data, ws);
            std::fs::create_dir_all(&dir).unwrap();
            let ix = cartograph_store::Index::create(&dir.join("index.cartograph")).unwrap();
            ix.conn
                .execute_batch(&format!(
                    "{undo}
                     ALTER TABLE repo DROP COLUMN indexed_commit;
                     ALTER TABLE repo DROP COLUMN indexed_seq; ALTER TABLE repo DROP COLUMN removed;
                     DROP INDEX commits_repo_sha; DROP INDEX commits_sha;
                     CREATE UNIQUE INDEX commits_sha ON commits (sha);
                     UPDATE meta SET value = {format} WHERE key = 'format_version';
                     PRAGMA user_version = {format};",
                    undo = cartograph_store::migrate::undo_to_v2()
                ))
                .unwrap();
        }
        let info = ServerInfo {
            pid: 1,
            host: std::net::Ipv4Addr::LOCALHOST.into(),
            port: 1,
            token: token(),
            version: "t".into(),
        };
        let app = std::sync::Arc::new(routes::App::new(data.clone(), None, &info));
        assert_eq!(app.workspace_ids(), [old, none]);
        let err = cartograph_query::QueryError::Unavailable {
            why: String::new(),
            action: String::new(),
        };
        for ws in [old, none] {
            let (summary, fresh) = app.unopened(ws, &err).unwrap();
            assert_eq!(
                (summary.state, fresh.state),
                (IndexState::NeedsMigrate, IndexState::NeedsMigrate)
            );
            assert!(fresh.error.is_some());
        }
        // `serve` at start: the migratable one is brought current. (The other gets a sync job, which
        // rebuilds it; the indexer's old_format test covers that path.)
        let w = cartograph_index::sync::Workspace {
            dir: cartograph_core::paths::workspace_dir(&data, old),
            ..Default::default()
        };
        assert_eq!(
            cartograph_index::sync::migrate_if_needed(&w).unwrap(),
            cartograph_index::sync::Upgrade::Migrated
        );
        let ix = cartograph_store::Index::open_read(&w.index_path()).unwrap();
        assert_eq!(ix.state, cartograph_store::State::Current);
    }

    #[test]
    fn a_timeout_is_503_with_retry_after_and_nothing_else_has_one() {
        use cartograph_api::types::{ApiError, ErrorCode};
        let res = routes::error(&ApiError::new(ErrorCode::Timeout, "t", "w", "a"));
        assert_eq!(res.status().as_u16(), 503);
        assert_eq!(res.headers()["retry-after"], "5");
        for code in [
            ErrorCode::Ambiguous,
            ErrorCode::TooLarge,
            ErrorCode::IndexUnavailable,
        ] {
            let res = routes::error(&ApiError::new(code, "t", "w", "a"));
            assert!(!res.headers().contains_key("retry-after"), "{code:?}");
        }
        assert_eq!(ErrorCode::Ambiguous.status(), 400);
        assert_eq!(ErrorCode::TooLarge.status(), 422);
    }
}
