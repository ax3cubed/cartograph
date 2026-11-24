//! `cargo run -p cartograph-eval` prints the precision table; `--write` also rewrites
//! `docs/precision.md` and `docs/precision.json`.

fn main() {
    let r = cartograph_eval::evaluate_set_a();
    print!("{}", cartograph_eval::render(&r));
    if std::env::args().any(|a| a == "--write") {
        cartograph_eval::write_docs(&r).expect("write docs");
    }
}
