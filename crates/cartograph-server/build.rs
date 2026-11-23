//! SRV-06: embeds the app's static export. `pnpm build` writes `out/` at the repository root (or
//! wherever `CARTOGRAPH_UI_DIR` points); each file is brotli-compressed into `OUT_DIR` and listed in
//! `assets.rs`, which `src/ui.rs` includes. The binary then serves the app without reading the disk.
//!
//! Without `out/` (the CI Rust job doesn't build the web app, nor does `cargo install` without
//! Node) a one-page placeholder is embedded instead, which says how to get the app.

use std::fmt::Write as _;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::Write as _;
use std::path::{Path, PathBuf};

const PLACEHOLDER: &str = "<!doctype html><html lang=\"en\"><meta charset=\"utf-8\">\
<title>Cartograph</title><p>Cartograph is running, but this build has no app in it: it was \
compiled without <code>out/</code>. Run <code>pnpm build</code>, then build the binary again. \
The API is under <code>/api/v1</code>.</p></html>\n";

fn main() {
    println!("cargo:rerun-if-env-changed=CARTOGRAPH_UI_DIR");
    let manifest = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let ui = std::env::var_os("CARTOGRAPH_UI_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| manifest.join("../../out"));
    println!("cargo:rerun-if-changed={}", ui.display());

    let mut files = Vec::new();
    if ui.join("index.html").is_file() {
        walk(&ui, &ui, &mut files);
    } else {
        println!(
            "cargo:warning=no app export at {}; embedding a placeholder page (run `pnpm build` for the app)",
            ui.display()
        );
        files.push(("index.html".to_owned(), PLACEHOLDER.as_bytes().to_vec()));
        files.push(("404.html".to_owned(), PLACEHOLDER.as_bytes().to_vec()));
    }
    files.sort();

    let out = PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    let dir = out.join("ui");
    std::fs::create_dir_all(&dir).unwrap();
    let mut rs = String::from("pub static ASSETS: &[(&str, &[u8], u64)] = &[\n");
    for (i, (key, bytes)) in files.iter().enumerate() {
        let file = dir.join(format!("{i}.br"));
        let mut c = Vec::new();
        {
            let mut w = brotli::CompressorWriter::new(&mut c, 1 << 16, 11, 22);
            w.write_all(bytes).unwrap();
        }
        std::fs::write(&file, &c).unwrap();
        let mut h = DefaultHasher::new();
        bytes.hash(&mut h);
        writeln!(
            rs,
            "    ({key:?}, include_bytes!({:?}), {:#x}),",
            file.display().to_string(),
            h.finish()
        )
        .unwrap();
    }
    rs.push_str("];\n");
    std::fs::write(out.join("assets.rs"), rs).unwrap();
}

/// Every file under `dir`, keyed by its `/`-separated path under `root`. Symlinks are skipped.
fn walk(root: &Path, dir: &Path, files: &mut Vec<(String, Vec<u8>)>) {
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let t = e.file_type().unwrap();
        let p = e.path();
        if t.is_dir() {
            walk(root, &p, files);
        } else if t.is_file() {
            let key = p
                .strip_prefix(root)
                .unwrap()
                .components()
                .map(|c| c.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/");
            files.push((key, std::fs::read(&p).unwrap()));
        }
    }
}
