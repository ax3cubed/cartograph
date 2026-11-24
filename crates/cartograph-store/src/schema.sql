-- Index schema, format version 4 (IDX-03; v2 adds the repo's indexed commit, IDX-13; v3 adds lineage
-- across renames, rewrites, structure nodes, derived rows, coverage, layout and search, phase 3; v4
-- adds the cross-service tables, phase 4: one block per item, each marked XS-nn).
-- Every table is STRICT. Everything that changes over time
-- is an interval [from_seq, to_seq) on the first-parent timeline; to_seq NULL means still open.
-- No triggers and no views: an index file is untrusted input (board G01).

CREATE TABLE meta (
  key TEXT PRIMARY KEY,
  value ANY NOT NULL
) STRICT, WITHOUT ROWID;

CREATE TABLE extractor (
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL,
  version TEXT NOT NULL,
  UNIQUE (name, version)
) STRICT;

-- indexed_commit and indexed_seq move in the same transaction as the rows they cover (IDX-13), so a
-- crash leaves a complete, earlier index. removed: dropped from the workspace (IDX-14).
CREATE TABLE repo (
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL,
  path TEXT NOT NULL,
  head BLOB,
  shallow INTEGER NOT NULL,
  indexed_commit BLOB,
  indexed_seq INTEGER,
  removed INTEGER NOT NULL DEFAULT 0
) STRICT;

-- The coverage totals at the run's end (IDX-22): live tracked files, those parsed (not skipped),
-- live import references and those that resolved (to a file, a package, a dependency or a built-in).
CREATE TABLE run (
  id INTEGER PRIMARY KEY,
  started_at INTEGER NOT NULL,
  finished_at INTEGER,
  writer_version TEXT NOT NULL,
  head BLOB,
  stats TEXT,
  files_total INTEGER,
  files_parsed INTEGER,
  imports_total INTEGER,
  imports_resolved INTEGER
) STRICT;

-- One row per first-parent step. time is the committer time; corrected_time is strictly increasing.
CREATE TABLE commits (
  seq INTEGER PRIMARY KEY,
  repo INTEGER NOT NULL REFERENCES repo (id),
  sha BLOB NOT NULL,
  time INTEGER NOT NULL,
  corrected_time INTEGER NOT NULL,
  author TEXT NOT NULL,
  summary TEXT NOT NULL
) STRICT;
-- One commit can sit in two repos (a fork, vendored history), so a sha is unique per repo only.
CREATE UNIQUE INDEX commits_repo_sha ON commits (repo, sha);
CREATE INDEX commits_sha ON commits (sha);
CREATE INDEX commits_time ON commits (corrected_time);

-- A file's lineage (IDX-18): a rename moves it to a new path, so one file can have several paths.
CREATE TABLE file (
  id INTEGER PRIMARY KEY,
  repo INTEGER NOT NULL REFERENCES repo (id)
) STRICT;

-- via: add, rename-exact or rename-similar; similarity in percent for a rename-similar row.
CREATE TABLE file_path (
  file INTEGER NOT NULL REFERENCES file (id),
  path TEXT NOT NULL,
  from_seq INTEGER NOT NULL,
  to_seq INTEGER,
  via TEXT NOT NULL DEFAULT 'add',
  similarity INTEGER
) STRICT;
CREATE INDEX file_path_file ON file_path (file, from_seq);
CREATE INDEX file_path_path ON file_path (path);

CREATE TABLE file_version (
  file INTEGER NOT NULL REFERENCES file (id),
  blob BLOB NOT NULL,
  from_seq INTEGER NOT NULL,
  to_seq INTEGER
) STRICT;
CREATE INDEX file_version_file ON file_version (file, from_seq);

-- Facts per blob and extractor, postcard-encoded and sorted. Specifier strings are untrusted text (F8).
CREATE TABLE fact (
  blob BLOB NOT NULL,
  extractor INTEGER NOT NULL REFERENCES extractor (id),
  payload BLOB NOT NULL,
  PRIMARY KEY (blob, extractor)
) STRICT, WITHOUT ROWID;

-- kind: file, service, library or module (IDX-20). key names a structure node across runs and
-- rebuilds (`service:<repo>:<root>`, `module:<path>`). parent is the node one level up (a file's
-- module, a module's service); service is the top-level ancestor, so a scope's descendants are one
-- indexed lookup (SRV-08). fan_in counts live inbound links at the last run (IDX-21).
CREATE TABLE node (
  id INTEGER PRIMARY KEY,
  kind TEXT NOT NULL,
  file INTEGER REFERENCES file (id),
  key TEXT,
  name TEXT,
  parent INTEGER,
  service INTEGER,
  detected_by TEXT,
  fan_in INTEGER NOT NULL DEFAULT 0
) STRICT;
CREATE INDEX node_file ON node (file);
CREATE UNIQUE INDEX node_key ON node (key);
CREATE INDEX node_parent ON node (parent);
CREATE INDEX node_service ON node (service);

-- When a service or module node exists: the union of its files' path intervals (IDX-20).
CREATE TABLE node_span (
  node INTEGER NOT NULL REFERENCES node (id),
  from_seq INTEGER NOT NULL,
  to_seq INTEGER
) STRICT;
CREATE INDEX node_span_node ON node_span (node);

CREATE TABLE link (
  id INTEGER PRIMARY KEY,
  kind TEXT NOT NULL,
  src INTEGER NOT NULL REFERENCES node (id),
  dst INTEGER NOT NULL REFERENCES node (id),
  UNIQUE (kind, src, dst)
) STRICT;

-- A link is live at step s when from_seq <= s AND (to_seq IS NULL OR to_seq > s).
CREATE TABLE interval (
  link INTEGER NOT NULL REFERENCES link (id),
  from_seq INTEGER NOT NULL,
  to_seq INTEGER
) STRICT;
CREATE INDEX interval_at ON interval (from_seq, to_seq, link);
CREATE INDEX interval_link ON interval (link);

-- Something noted on a commit, such as rename-limit when rename detection gave up (IDX-18).
CREATE TABLE note (
  seq INTEGER NOT NULL,
  kind TEXT NOT NULL,
  detail TEXT NOT NULL
) STRICT;
CREATE INDEX note_seq ON note (seq);

-- A rewritten history (IDX-19): the repo's old and new head, their merge base (NULL when none) and
-- the base's corrected time (`at`, where the timeline marks it). archived counts the intervals kept
-- in archived_interval, dropped the ones over the cap.
CREATE TABLE rewrite (
  id INTEGER PRIMARY KEY,
  repo INTEGER NOT NULL REFERENCES repo (id),
  old_head BLOB NOT NULL,
  new_head BLOB NOT NULL,
  base BLOB,
  at INTEGER,
  recorded_at INTEGER NOT NULL,
  archived INTEGER NOT NULL,
  dropped INTEGER NOT NULL
) STRICT;

-- Links that existed only in a history that was rewritten, keyed by the commits of that history.
CREATE TABLE archived_interval (
  rewrite INTEGER NOT NULL REFERENCES rewrite (id),
  link INTEGER NOT NULL REFERENCES link (id),
  from_sha BLOB NOT NULL,
  to_sha BLOB
) STRICT;
CREATE INDEX archived_interval_rewrite ON archived_interval (rewrite);

-- Host names that reach a service: compose service names and k8s names (C6), for the links area.
CREATE TABLE alias (
  host TEXT NOT NULL,
  node INTEGER NOT NULL REFERENCES node (id),
  source TEXT NOT NULL
) STRICT;

-- One row per derived artefact (B6, IDX-21): what it was built from. A run rebuilds an artefact only
-- when its key changes; AI output is marked stale instead.
CREATE TABLE derived (
  kind TEXT NOT NULL,
  scope TEXT NOT NULL,
  inputs_key BLOB NOT NULL,
  stale INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (kind, scope)
) STRICT, WITHOUT ROWID;

-- Per service, library and module at the last run (IDX-21): live modules and files below it, and
-- live links that leave it, enter it or stay inside.
CREATE TABLE scope_count (
  node INTEGER PRIMARY KEY REFERENCES node (id),
  modules INTEGER NOT NULL,
  files INTEGER NOT NULL,
  links_out INTEGER NOT NULL,
  links_in INTEGER NOT NULL,
  links_inside INTEGER NOT NULL
) STRICT;

-- An import that didn't resolve (IDX-22), while it was there. spec is untrusted text (F8), cut to
-- 512 bytes, with anything that looks like a secret masked (XS-10). XS-01 extends it to every
-- reference an adapter couldn't resolve: fact_kind is the kind of the fact (NULL for an import), raw
-- the reference as the source spells it, masked and cut the same way.
CREATE TABLE unresolved (
  file INTEGER NOT NULL REFERENCES file (id),
  spec TEXT NOT NULL,
  line INTEGER NOT NULL,
  stage TEXT NOT NULL,
  reason TEXT NOT NULL,
  from_seq INTEGER NOT NULL,
  to_seq INTEGER,
  fact_kind TEXT,
  raw TEXT
) STRICT;
CREATE INDEX unresolved_at ON unresolved (from_seq, to_seq);
CREATE INDEX unresolved_file ON unresolved (file, spec);

-- Positions (IDX-25): a node's circle in the workspace's plane, from the run that placed it.
CREATE TABLE layout (
  node INTEGER NOT NULL REFERENCES node (id),
  x REAL NOT NULL,
  y REAL NOT NULL,
  r REAL NOT NULL,
  from_seq INTEGER NOT NULL,
  to_seq INTEGER
) STRICT;
CREATE INDEX layout_node ON layout (node, from_seq);

-- Search (AI-15): names, split identifiers and paths, contentless (the text lives in node and
-- file_path), rowid = node id.
CREATE VIRTUAL TABLE search USING fts5 (
  name, split, path, docstring, summary,
  content = '', contentless_delete = 1, prefix = '2 3', tokenize = 'unicode61'
);
-- Substring matches of three or more characters over the same names and paths (SRV-09), what the
-- palette's `includes` did; rowid = node id.
CREATE VIRTUAL TABLE search_sub USING fts5 (
  name, path, content = '', contentless_delete = 1, tokenize = 'trigram'
);

-- ---- XS-23 owners: begin ----
-- The CODEOWNERS blob of a repo (the first of .github/, root, docs/, .gitlab/) over history, one
-- row per version: present from from_seq until to_seq (NULL: still).
CREATE TABLE codeowners (
  repo INTEGER NOT NULL REFERENCES repo (id),
  blob BLOB NOT NULL,
  from_seq INTEGER NOT NULL,
  to_seq INTEGER
) STRICT;
CREATE INDEX codeowners_repo ON codeowners (repo, from_seq);

-- Who owns a file lineage while (last matching pattern wins). owner is untrusted text (F8): an
-- `@user`, an `@org/team` or an email, at most 256 bytes of [A-Za-z0-9._%+@/-]; pattern is the
-- CODEOWNERS pattern that matched, cut to 512 bytes. kind: team, person or email. Emails are
-- personal data: they stay in the index and are left out of reports.
CREATE TABLE ownership (
  file INTEGER NOT NULL REFERENCES file (id),
  owner TEXT NOT NULL,
  kind TEXT NOT NULL,
  pattern TEXT NOT NULL,
  from_seq INTEGER NOT NULL,
  to_seq INTEGER
) STRICT;
CREATE INDEX ownership_file ON ownership (file, from_seq);
CREATE INDEX ownership_owner ON ownership (owner);
-- ---- XS-23 owners: end ----

-- >>> XS-01 (format 4): what a link stands on. One row per piece of source text, for a link (NULL
-- while it is only a fact). text is the source cut to 200 characters with secrets masked (XS-10);
-- kind is literal, template or inferred; extractor is `name@version`.
CREATE TABLE evidence (
  id INTEGER PRIMARY KEY,
  link INTEGER REFERENCES link (id),
  file INTEGER NOT NULL REFERENCES file (id),
  kind TEXT NOT NULL,
  text TEXT NOT NULL,
  start_line INTEGER NOT NULL,
  start_col INTEGER NOT NULL,
  end_line INTEGER NOT NULL,
  end_col INTEGER NOT NULL,
  extractor TEXT NOT NULL
) STRICT;
CREATE INDEX evidence_link ON evidence (link);
CREATE INDEX evidence_file ON evidence (file);
-- <<< XS-01
