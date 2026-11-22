//! manifest-js: package.json and tsconfig/jsconfig files. These facts are also the resolver's
//! inputs: the replay's file system serves a manifest as the object of its resolution fields only,
//! so a manifest change that touches none of them (a version bump) re-resolves nothing (phase 0).

use crate::{BlobFacts, Extractor, Fact};

/// Values keep the file's key order (serde_json's `preserve_order`, enabled for this crate): the
/// order of conditions in `exports` and `imports` decides which one matches, so it is part of what
/// a resolution reads.
pub struct ManifestJs;

const PACKAGE: &str = "manifest-js/package";
const TSCONFIG: &str = "manifest-js/tsconfig";

/// The package.json fields a resolution reads.
pub const PACKAGE_FIELDS: [&str; 9] = [
    "name", "main", "module", "types", "typings", "exports", "imports", "browser", "type",
];
/// The tsconfig fields a resolution reads. `files`, `include` and `exclude` decide which project
/// of a solution-style config applies to a file.
pub const TSCONFIG_FIELDS: [&str; 6] = [
    "extends",
    "compilerOptions",
    "references",
    "files",
    "include",
    "exclude",
];
const DEPENDENCIES: [&str; 4] = [
    "dependencies",
    "devDependencies",
    "peerDependencies",
    "optionalDependencies",
];

impl Extractor for ManifestJs {
    fn name(&self) -> &'static str {
        "manifest-js"
    }
    fn version(&self) -> &'static str {
        "1.0.0"
    }
    fn handles(&self, path: &str) -> Option<&'static str> {
        let base = path.rsplit('/').next().unwrap_or(path);
        if base == "package.json" {
            Some(PACKAGE)
        } else if (base.starts_with("tsconfig") || base.starts_with("jsconfig"))
            && base.ends_with(".json")
        {
            Some(TSCONFIG)
        } else {
            None
        }
    }
    fn extract(&self, kind: &'static str, bytes: &[u8]) -> BlobFacts {
        if let Some(r) = crate::skip_reason(bytes) {
            return BlobFacts::Skipped(r);
        }
        let mut buf = bytes.to_vec();
        // tsconfig is JSONC; package.json never has comments, and stripping is harmless.
        let _ = json_strip_comments::strip_slice(&mut buf);
        let Ok(serde_json::Value::Object(obj)) = serde_json::from_slice::<serde_json::Value>(&buf)
        else {
            return BlobFacts::Facts(vec![]);
        };
        let fields: &[&str] = if kind == PACKAGE {
            &PACKAGE_FIELDS
        } else {
            &TSCONFIG_FIELDS
        };
        let mut facts: Vec<Fact> = fields
            .iter()
            .chain(if kind == PACKAGE {
                &["workspaces"][..]
            } else {
                &[]
            })
            .filter_map(|f| {
                Some(Fact::Manifest {
                    field: (*f).to_owned(),
                    value: obj.get(*f)?.to_string(),
                })
            })
            .collect();
        if kind == PACKAGE {
            let mut names: Vec<&str> = DEPENDENCIES
                .iter()
                .filter_map(|d| obj.get(*d)?.as_object())
                .flat_map(|m| m.keys().map(String::as_str))
                .collect();
            names.sort_unstable();
            names.dedup();
            if !names.is_empty() {
                facts.push(Fact::Manifest {
                    field: "dependency_names".into(),
                    value: serde_json::to_string(&names).expect("strings encode"),
                });
            }
        }
        facts.sort();
        BlobFacts::Facts(facts)
    }
}

/// The compilerOptions oxc_resolver 11.24 reads when it resolves or picks a file's tsconfig; the rest
/// (strict, jsx, target, ...) can't change a resolution.
const RESOLUTION_COMPILER_OPTIONS: [&str; 6] = [
    "allowJs",
    "baseUrl",
    "declarationDir",
    "outDir",
    "paths",
    "rootDirs",
];

/// The manifest as the resolver sees it: a JSON object of its resolution fields only.
pub fn resolution_json(facts: &[Fact], package: bool) -> String {
    let keep: &[&str] = if package {
        &PACKAGE_FIELDS
    } else {
        &TSCONFIG_FIELDS
    };
    let mut out = String::from("{");
    for f in facts {
        if let Fact::Manifest { field, value } = f
            && keep.contains(&field.as_str())
        {
            let narrowed;
            let value = if !package && field == "compilerOptions" {
                let Ok(serde_json::Value::Object(mut o)) = serde_json::from_str(value) else {
                    continue;
                };
                o.retain(|k, _| RESOLUTION_COMPILER_OPTIONS.contains(&k.as_str()));
                narrowed = serde_json::Value::Object(o).to_string();
                &narrowed
            } else {
                value
            };
            if out.len() > 1 {
                out.push(',');
            }
            out.push_str(&serde_json::to_string(field).expect("strings encode"));
            out.push(':');
            out.push_str(value);
        }
    }
    out.push('}');
    out
}

/// True for a path whose base name makes it a package.json.
pub fn is_package_json(path: &str) -> bool {
    path == "package.json" || path.ends_with("/package.json")
}
