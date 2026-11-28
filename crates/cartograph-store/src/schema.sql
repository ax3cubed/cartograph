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
-- indexed lookup (SRV-08). detected_by is the C6 rule that found a service or library (the
-- detect_rule of board S04) and detect_source the file that rule read (XS-02). fan_in counts live inbound links at the last run (IDX-21).
CREATE TABLE node (
  id INTEGER PRIMARY KEY,
  kind TEXT NOT NULL,
  file INTEGER REFERENCES file (id),
  key TEXT,
  name TEXT,
  parent INTEGER,
  service INTEGER,
  detected_by TEXT,
  detect_source TEXT,
  fan_in INTEGER NOT NULL DEFAULT 0,
  -- XS-11: the generator that made this file (orval, oapi-codegen, ...) at the last run, else NULL.
  generated TEXT
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
  -- XS-05 confidence (begin): set when the link is written; NULL for kinds that carry no
  -- confidence (imports). level is exact, likely, guessed or weak; score is inside its band.
  level TEXT,
  score REAL,
  -- XS-05 confidence (end)
  -- XS-09 config (begin): the environments the link holds in, bit i for environment id i; NULL
  -- when it doesn't depend on one.
  envs INTEGER,
  -- XS-09 config (end)
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

-- XS-02 host aliases (begin)
-- Host names that reach a service (C6, C7): compose service and container names, network aliases and
-- k8s Service names in their four forms. port is the one the host listens on (NULL: any). env_id
-- names the environment the file belongs to (NULL: every one; environments arrive with XS-09).
-- source is the file's path and blob_id the blob that says so; valid_from and valid_to are the
-- interval of that version of the file at that path, like every other row's.
CREATE TABLE host_alias (
  host TEXT NOT NULL,
  port INTEGER,
  env_id INTEGER,
  service_id INTEGER NOT NULL REFERENCES node (id),
  source TEXT NOT NULL,
  blob_id BLOB NOT NULL,
  valid_from INTEGER NOT NULL,
  valid_to INTEGER
) STRICT;
CREATE INDEX host_alias_host ON host_alias (host, valid_from);
CREATE INDEX host_alias_service ON host_alias (service_id);
-- XS-02 host aliases (end)

-- XS-09 config (begin)
-- Environments: the manifest's `environments:` and the ones file names imply (values-prod.yaml,
-- overlays/staging/, compose and .env.example are `local`). id is the bit the `link.envs` mask
-- uses, so at most 32; is_default marks the one the map shows when none is asked for.
CREATE TABLE environment (
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL UNIQUE,
  is_default INTEGER NOT NULL
) STRICT;
-- What a key is set to in an environment while (env_id NULL: every one), with the file and blob
-- that say so. value is stored only when it classifies as a locator (XS-10): a URL without
-- userinfo, query or fragment, host[:port] or a short identifier; value_kind is `locator`, or
-- `opaque` with a NULL value (a key whose value isn't kept, a Secret's keys).
CREATE TABLE config_value (
  key TEXT NOT NULL,
  env_id INTEGER REFERENCES environment (id),
  value TEXT,
  value_kind TEXT NOT NULL,
  source TEXT NOT NULL,
  blob_id BLOB NOT NULL,
  valid_from INTEGER NOT NULL,
  valid_to INTEGER
) STRICT;
CREATE INDEX config_value_key ON config_value (key, valid_from);
-- The key and value an HTTP link was resolved through, per environment, while that held.
CREATE TABLE link_config (
  link INTEGER NOT NULL REFERENCES link (id),
  env_id INTEGER NOT NULL REFERENCES environment (id),
  key TEXT NOT NULL,
  value TEXT NOT NULL,
  source TEXT NOT NULL,
  from_seq INTEGER NOT NULL,
  to_seq INTEGER
) STRICT;
CREATE INDEX link_config_link ON link_config (link);
-- XS-09 config (end)

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

-- XS-04 http links (begin)
-- The HTTP pass reads the interface facts (extractor `adapters`) with the file versions that hold
-- them: both lookups go by the index. Its links are `link` rows of kind http, its evidence rows
-- are the call site and the route site, its unresolved calls carry fact_kind http.call.
CREATE INDEX fact_extractor ON fact (extractor);
CREATE INDEX file_version_blob ON file_version (blob);
-- XS-04 http links (end)

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
  extractor TEXT NOT NULL,
  -- XS-06 link detail (begin): `get /orders/{}`, the call or route this site stands for.
  operation TEXT
  -- XS-06 link detail (end)
) STRICT;
CREATE INDEX evidence_link ON evidence (link);
CREATE INDEX evidence_file ON evidence (file);
-- <<< XS-01

-- XS-08 contracts (begin)
-- OpenAPI and Swagger specs (the `manifest-openapi` facts of the spec files) with intervals. One
-- contract per spec file lineage; service is the node that owns it (the manifest's contracts[].service,
-- else the service whose root holds the file). An operation is a method on a path (base path
-- included, as written: path; with parameters as `{}`: template) over the time that version of the
-- spec had it. summary is cut to 200 characters and is untrusted text. fields is a JSON array of
-- {name, ty, required}. A binding joins an operation to code: handled_by (file is the route's),
-- consumes (file is the call's), or, with no operation, undocumented (a route of a service that has
-- a spec and is in none). Unresolved `$ref`s are `unresolved` rows of fact_kind openapi.ref.
CREATE TABLE contract (
  id INTEGER PRIMARY KEY,
  file INTEGER NOT NULL REFERENCES file (id),
  service INTEGER REFERENCES node (id),
  title TEXT NOT NULL,
  version TEXT NOT NULL
) STRICT;
CREATE UNIQUE INDEX contract_file ON contract (file);
CREATE TABLE operation (
  id INTEGER PRIMARY KEY,
  contract INTEGER NOT NULL REFERENCES contract (id),
  method TEXT NOT NULL,
  path TEXT NOT NULL,
  template TEXT NOT NULL,
  operation_id TEXT,
  summary TEXT,
  line INTEGER NOT NULL,
  from_seq INTEGER NOT NULL,
  to_seq INTEGER
) STRICT;
CREATE INDEX operation_contract ON operation (contract);
CREATE TABLE contract_schema (
  contract INTEGER NOT NULL REFERENCES contract (id),
  name TEXT NOT NULL,
  fields TEXT NOT NULL,
  from_seq INTEGER NOT NULL,
  to_seq INTEGER
) STRICT;
CREATE INDEX contract_schema_contract ON contract_schema (contract);
CREATE TABLE binding (
  id INTEGER PRIMARY KEY,
  kind TEXT NOT NULL,
  operation INTEGER REFERENCES operation (id),
  file INTEGER NOT NULL REFERENCES file (id),
  line INTEGER NOT NULL,
  template TEXT NOT NULL,
  from_seq INTEGER NOT NULL,
  to_seq INTEGER
) STRICT;
CREATE INDEX binding_operation ON binding (operation);
CREATE INDEX binding_kind ON binding (kind, from_seq);
-- XS-08 contracts (end)

-- XS-19 spec versions (begin)
-- One row per distinct spec of a contract along first-parent history (the file lineage follows
-- renames): n counts from 1, seq is the commit where that spec came to be, spec_hash is the hash
-- of its canonical operations and schemas, blob is the root spec's blob (for a diff on request).
-- Only the breaking changes are kept (JSON array of {kind, subject, breaking, summary}), with the
-- number of all changes and a one-line summary built from the diff; full diffs are recomputed.
CREATE TABLE contract_version (
  contract INTEGER NOT NULL REFERENCES contract (id),
  n INTEGER NOT NULL,
  seq INTEGER NOT NULL,
  blob BLOB NOT NULL,
  spec_hash BLOB NOT NULL,
  changes INTEGER NOT NULL,
  breaking_count INTEGER NOT NULL,
  breaking TEXT NOT NULL,
  summary TEXT NOT NULL,
  PRIMARY KEY (contract, n)
) STRICT, WITHOUT ROWID;
-- XS-19 spec versions (end)
-- XS-11 generated clients (begin)
-- A function of a generated client that is an operation of a spec: found by the method and path
-- the function spells out, not by its name, so a regenerated client keeps its alias row. file is the
-- generated file, line the function's.
CREATE TABLE client_alias (
  operation INTEGER NOT NULL REFERENCES operation (id),
  file INTEGER NOT NULL REFERENCES file (id),
  symbol TEXT NOT NULL,
  line INTEGER NOT NULL,
  from_seq INTEGER NOT NULL,
  to_seq INTEGER
) STRICT;
CREATE INDEX client_alias_operation ON client_alias (operation);
-- The names the files that import a generated client call, per blob (the `refs` adapters), made
-- when the client is found. A cache of the derived pass, not a fact: it is not in the fact hash.
CREATE TABLE blob_ref (
  blob BLOB PRIMARY KEY,
  version TEXT NOT NULL,
  payload BLOB NOT NULL
) STRICT, WITHOUT ROWID;
-- XS-11 generated clients (end)
