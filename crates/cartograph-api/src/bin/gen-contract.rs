//! Writes the OpenAPI file and the JSON Schema the TypeScript types are generated from.
//! Run from the repository root: `cargo run -p cartograph-api --bin gen-contract`, then `pnpm gen:types`.

use std::path::Path;

fn main() -> std::io::Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    std::fs::write(
        root.join(cartograph_api::OPENAPI_PATH),
        cartograph_api::render(&cartograph_api::openapi()),
    )?;
    std::fs::write(
        root.join(cartograph_api::SCHEMA_PATH),
        cartograph_api::render(&cartograph_api::json_schema()),
    )?;
    println!(
        "wrote {} and {}",
        cartograph_api::OPENAPI_PATH,
        cartograph_api::SCHEMA_PATH
    );
    Ok(())
}
