//! An index in an older format is brought current under the writer lock (IDX-16, B3): migrated
//! when a migration is registered, rebuilt from git when none is or it fails. Nothing drops it.

use cartograph_facts::FactCache;
use cartograph_git::fixture::Fixture;
use cartograph_index::sync::{
    SyncOptions, SyncOutcome, SyncSummary, Upgrade, Workspace, migrate_if_needed, sync,
};
use cartograph_index::timeline::RepoSpec;
use cartograph_store::rusqlite::Connection;
use cartograph_store::{Index, State, fact_hash};

fn run(w: &Workspace, cache: &mut FactCache) -> SyncSummary {
    match sync(w, &SyncOptions::default(), cache).unwrap() {
        SyncOutcome::Synced(s) => s,
        SyncOutcome::AlreadyRunning => panic!("nothing else is running"),
    }
}

/// Takes the index back to format 1, as `writer.rs` does; `format` other than 1 has no migration.
/// Formats 4 and 3's tables and columns go first, then format 2's.
fn downgrade(w: &Workspace, format: i64) {
    Connection::open(w.index_path())
        .unwrap()
        .execute_batch(&format!(
            "{undo4} {undo}
             ALTER TABLE repo DROP COLUMN indexed_commit; ALTER TABLE repo DROP COLUMN indexed_seq;
             ALTER TABLE repo DROP COLUMN removed;
             DROP INDEX commits_repo_sha; DROP INDEX commits_sha;
             CREATE UNIQUE INDEX commits_sha ON commits (sha);
             UPDATE meta SET value = {format} WHERE key = 'format_version';
             PRAGMA user_version = {format};",
            undo4 = cartograph_store::migrate::UNDO_V4,
            undo = cartograph_store::migrate::UNDO_V3
        ))
        .unwrap();
    assert_eq!(
        Index::open_read(&w.index_path()).unwrap().state,
        State::NeedsMigrate
    );
}

fn hash(w: &Workspace) -> String {
    let ix = Index::open_read(&w.index_path()).unwrap();
    assert_eq!(ix.state, State::Current);
    fact_hash(&ix.conn).unwrap().to_string()
}

#[test]
fn an_old_index_is_migrated_at_serve_and_at_sync_or_else_rebuilt() {
    let d = tempfile::tempdir().unwrap();
    let mut f = Fixture::init(&d.path().join("repo"));
    for i in 0..3 {
        f.write("a.ts", &format!("import './b';\nexport const a = {i};\n"));
        f.write("b.ts", "export {};\n");
        f.commit("step");
    }
    let w = Workspace {
        dir: d.path().join("ws"),
        repos: vec![RepoSpec {
            name: "repo".into(),
            path: f.dir.clone(),
            ..Default::default()
        }],
        ..Default::default()
    };
    let mut cache = FactCache::open(&d.path().join("facts.db")).unwrap();
    run(&w, &mut cache);
    let want = hash(&w);

    // At serve: migrated in place, same facts.
    downgrade(&w, 1);
    assert_eq!(migrate_if_needed(&w).unwrap(), Upgrade::Migrated);
    assert_eq!(hash(&w), want);
    assert_eq!(migrate_if_needed(&w).unwrap(), Upgrade::Current);

    // At sync: migrated, then synced as usual.
    downgrade(&w, 1);
    assert_eq!(run(&w, &mut cache).steps, 0);
    assert_eq!(hash(&w), want);

    // No migration path: serve leaves it for a sync, which rebuilds from git.
    downgrade(&w, 0);
    assert_eq!(migrate_if_needed(&w).unwrap(), Upgrade::Rebuild);
    let s = run(&w, &mut cache);
    assert_eq!(s.steps, 3, "built again from nothing");
    assert_eq!(hash(&w), want);
}
