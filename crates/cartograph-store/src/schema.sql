-- Index schema, format version 1 (IDX-03). Every table is STRICT. Everything that changes over time
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

CREATE TABLE repo (
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL,
  path TEXT NOT NULL,
  head BLOB,
  shallow INTEGER NOT NULL
) STRICT;

CREATE TABLE run (
  id INTEGER PRIMARY KEY,
  started_at INTEGER NOT NULL,
  finished_at INTEGER,
  writer_version TEXT NOT NULL,
  head BLOB,
  stats TEXT
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
CREATE UNIQUE INDEX commits_sha ON commits (sha);
CREATE INDEX commits_time ON commits (corrected_time);

-- A file's lineage. Until rename detection (IDX-18), one lineage per path.
CREATE TABLE file (
  id INTEGER PRIMARY KEY,
  repo INTEGER NOT NULL REFERENCES repo (id)
) STRICT;

CREATE TABLE file_path (
  file INTEGER NOT NULL REFERENCES file (id),
  path TEXT NOT NULL,
  from_seq INTEGER NOT NULL,
  to_seq INTEGER
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

CREATE TABLE node (
  id INTEGER PRIMARY KEY,
  kind TEXT NOT NULL,
  file INTEGER REFERENCES file (id)
) STRICT;
CREATE INDEX node_file ON node (file);

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
