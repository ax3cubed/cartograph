//! HD-04: a data folder holding the golden indexes, for front-end work against a real server.
//!
//!     cargo run -p cartograph-fixtures --example fixture-home -- /tmp/cartograph-fixture
//!     cargo run -p cartograph-cli -- serve --fixture /tmp/cartograph-fixture
//!
//! (`CARTOGRAPH_HOME=/tmp/cartograph-fixture cargo run -p cartograph-cli -- serve` is the same.)
//! Workspaces `northwind` and `history`; existing indexes are replaced.

fn main() {
    let dir = std::env::args_os()
        .nth(1)
        .map(std::path::PathBuf::from)
        .expect("usage: fixture-home <dir>");
    for (name, repo) in [
        ("northwind", cartograph_fixtures::northwind()),
        ("history", cartograph_fixtures::history()),
    ] {
        let ws = dir.join("workspaces").join(name);
        std::fs::create_dir_all(&ws).unwrap();
        let built = tempfile::tempdir().unwrap();
        let ix = cartograph_fixtures::index(&repo, built.path());
        std::fs::copy(ix, ws.join("index.cartograph")).unwrap();
        println!("{}", ws.join("index.cartograph").display());
    }
}
