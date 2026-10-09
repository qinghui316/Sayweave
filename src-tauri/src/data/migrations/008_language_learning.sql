CREATE TABLE language_settings (
 singleton INTEGER PRIMARY KEY NOT NULL CHECK(singleton=1),
 enabled INTEGER NOT NULL DEFAULT 1 CHECK(enabled IN (0,1)),
 revision INTEGER NOT NULL DEFAULT 1 CHECK(revision>=1)
) STRICT;
INSERT INTO language_settings(singleton,enabled,revision) VALUES(1,1,1);
CREATE TABLE language_term_sources (
 term_id TEXT PRIMARY KEY NOT NULL REFERENCES dictionary_terms(id) ON DELETE CASCADE,
 source TEXT NOT NULL CHECK(source IN ('manual','automatic')),
 evidence_id TEXT, settings_revision INTEGER NOT NULL, created_at INTEGER NOT NULL
) STRICT;
CREATE TABLE language_suppression (
 term_key TEXT PRIMARY KEY NOT NULL, updated_at INTEGER NOT NULL
) STRICT;
CREATE TABLE language_edits (
 id TEXT PRIMARY KEY NOT NULL, task_id TEXT NOT NULL, scope TEXT NOT NULL,
 settings_revision INTEGER NOT NULL, evidence TEXT NOT NULL CHECK(json_valid(evidence)),
 decision TEXT CHECK(decision IS NULL OR json_valid(decision)),
 analysis TEXT CHECK(analysis IS NULL OR json_valid(analysis)),
 state TEXT NOT NULL CHECK(state IN ('pending','settled','expired','failed')),
 created_at INTEGER NOT NULL, expires_at INTEGER NOT NULL
) STRICT;
CREATE TABLE language_lexical_candidates (
 id TEXT PRIMARY KEY NOT NULL REFERENCES language_edits(id) ON DELETE CASCADE,
 term_key TEXT NOT NULL, term TEXT NOT NULL, original TEXT NOT NULL,
 scope TEXT NOT NULL, created_at INTEGER NOT NULL
) STRICT;
CREATE INDEX language_lexical_term ON language_lexical_candidates(term_key,created_at);
CREATE TABLE language_observations (
 sequence INTEGER PRIMARY KEY AUTOINCREMENT, id TEXT NOT NULL UNIQUE,
 task_id TEXT NOT NULL, scope TEXT NOT NULL, settings_revision INTEGER NOT NULL,
 kind TEXT NOT NULL CHECK(kind IN ('speech','edit')),
 observation TEXT NOT NULL CHECK(json_valid(observation)), created_at INTEGER NOT NULL
) STRICT;
CREATE INDEX language_scope_observations ON language_observations(scope,sequence);
CREATE INDEX language_current_observations ON language_observations(settings_revision,sequence);
CREATE TABLE language_profiles (
 scope TEXT PRIMARY KEY NOT NULL, version INTEGER NOT NULL DEFAULT 0,
 watermark INTEGER NOT NULL DEFAULT 0, published_at INTEGER NOT NULL DEFAULT 0
 ,attempted_watermark INTEGER NOT NULL DEFAULT 0, attempted_at INTEGER NOT NULL DEFAULT 0
) STRICT;
CREATE TABLE language_profile_revisions (
 scope TEXT NOT NULL, version INTEGER NOT NULL CHECK(version>=1), predecessor INTEGER NOT NULL,
 watermark INTEGER NOT NULL, settings_revision INTEGER NOT NULL,
 preferences TEXT NOT NULL CHECK(json_valid(preferences)), candidates TEXT NOT NULL CHECK(json_valid(candidates)),
 evidence TEXT NOT NULL CHECK(json_valid(evidence)), reason TEXT NOT NULL,
 analysis TEXT NOT NULL CHECK(json_valid(analysis)), created_at INTEGER NOT NULL,
 PRIMARY KEY(scope,version)
) STRICT;
