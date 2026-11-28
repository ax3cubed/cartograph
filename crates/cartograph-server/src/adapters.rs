//! `GET /api/v1/w/{ws}/adapters` (board S5): the adapters a sync can run, and whether each may.
//! The built-in ones always run. A subprocess adapter named in the manifest runs only when this
//! user approved its exact folder hash (`cartograph adapters allow`); the app only reads that.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::response::Response;
use cartograph_api::types::{Adapter, AdapterKind, ApiError, ErrorCode};
use cartograph_discovery::{Registry, manifest};
use cartograph_facts::adapters::AdapterSet;
use cartograph_facts::adapters::subprocess::Declared;

use crate::routes::{App, error, ok};
use crate::writes::blocking;

pub async fn get(State(app): State<Arc<App>>, Path(ws): Path<String>) -> Response {
    blocking(move || {
        let unknown = || {
            error(&ApiError::new(
                ErrorCode::NotFound,
                "no such workspace",
                "the registry has no workspace with this id",
                "list workspaces with GET /api/v1/workspaces",
            ))
        };
        let Ok((reg, _)) = Registry::load(&app.data_dir) else {
            return unknown();
        };
        let entry = reg.workspaces.iter().find(|w| w.id == ws);
        // An index nobody registered still has the built-in adapters; it has no manifest to add more.
        let indexed = cartograph_core::paths::workspace_dir(&app.data_dir, &ws)
            .join("index.cartograph")
            .is_file();
        if entry.is_none() && !indexed {
            return unknown();
        }
        let mut out: Vec<Adapter> = AdapterSet::builtin()
            .adapters()
            .iter()
            .map(|a| Adapter {
                name: a.name.clone(),
                version: a.version.clone(),
                kind: AdapterKind::BuiltIn,
                allowed: true,
            })
            .collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        if let Some(entry) = entry
            && let Ok(parsed) = manifest::read(&entry.manifest)
        {
            let base = entry.manifest.parent().unwrap_or(std::path::Path::new("/"));
            // A folder that doesn't load is left out: its reason is `cartograph adapters list`'s.
            out.extend(parsed.manifest.adapters.iter().filter_map(|f| {
                let d = Declared::load(base, f).ok()?;
                Some(Adapter {
                    allowed: reg.adapter_allowed(&ws, &d.name, &d.hash),
                    name: d.name,
                    version: d.version,
                    kind: AdapterKind::Subprocess,
                })
            }));
        }
        ok(0, out)
    })
    .await
}
