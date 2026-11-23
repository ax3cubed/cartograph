//! `cartograph open` and `cartograph serve` (SRV-05, SRV-06): one local server per user, on
//! loopback, found again through `server.json`.

use std::io::IsTerminal;

use cartograph_core::{Report, logging};
use cartograph_server::{ServeOptions, ServerInfo, running};
use serde_json::json;

use crate::output::Out;
use crate::{Global, data_dir};

/// Opens the app: reuses the running server when there is one, else starts one and serves until
/// Ctrl-C.
// ponytail: the cartograph:// link argument (I7) is parsed and ignored until share links land.
pub fn open(out: &Out, g: &Global) -> Result<(), Report> {
    let data = data_dir()?;
    if let Some(info) = running(&data) {
        announce(out, &info, true);
        return Ok(());
    }
    run(out, g, None, None, None, true)
}

/// Serves without opening a browser, for development and tests.
/// `fixture` (debug builds, HD-04) serves that data folder instead of the user's.
pub fn serve(
    out: &Out,
    g: &Global,
    host: Option<String>,
    port: Option<u16>,
    fixture: Option<std::path::PathBuf>,
) -> Result<(), Report> {
    run(out, g, host, port, fixture, false)
}

fn run(
    out: &Out,
    g: &Global,
    host: Option<String>,
    port: Option<u16>,
    fixture: Option<std::path::PathBuf>,
    browser: bool,
) -> Result<(), Report> {
    let data = match fixture {
        Some(d) => d,
        None => data_dir()?,
    };
    let _log = logging::init(&data.join("logs"), g.verbose).ok();
    cartograph_server::serve(
        ServeOptions {
            data_dir: data,
            host,
            port,
            home: None,
        },
        |info| announce(out, info, browser),
    )
    .map_err(|e| Report::from_error(&e))
}

/// Prints the address with its token (stdout, so only this user sees it) and, at a terminal,
/// opens the browser on it.
fn announce(out: &Out, info: &ServerInfo, browser: bool) {
    let url = info.url();
    out.print(
        &format!("Cartograph is at {url}"),
        json!({ "url": url, "port": info.port, "pid": info.pid }),
    );
    if browser && std::io::stdout().is_terminal() {
        launch(&url);
    }
}

/// Hands the URL to the platform's opener by absolute path, never a PATH lookup. Best effort: the
/// address is already printed.
fn launch(url: &str) {
    #[cfg(target_os = "macos")]
    let cmd = std::process::Command::new("/usr/bin/open").arg(url).spawn();
    #[cfg(windows)]
    let cmd = std::process::Command::new(
        std::path::Path::new(
            &std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into()),
        )
        .join("System32\\rundll32.exe"),
    )
    .args(["url.dll,FileProtocolHandler", url])
    .spawn();
    #[cfg(not(any(target_os = "macos", windows)))]
    let cmd = std::process::Command::new("/usr/bin/xdg-open")
        .arg(url)
        .spawn();
    let _ = cmd;
}
