CREATE TABLE processing_settings (
 singleton INTEGER PRIMARY KEY NOT NULL CHECK(singleton=1),
 enabled INTEGER NOT NULL DEFAULT 1 CHECK(enabled IN (0,1)),
 revision INTEGER NOT NULL DEFAULT 1 CHECK(revision>=1)
) STRICT;
INSERT INTO processing_settings(singleton,enabled,revision) VALUES(1,1,1);
CREATE TABLE processing_runs (
 id TEXT PRIMARY KEY NOT NULL,
 attempt_id TEXT NOT NULL UNIQUE REFERENCES speech_attempts(id) ON DELETE CASCADE,
 task_id TEXT NOT NULL,
 config_id TEXT NOT NULL, config_name TEXT NOT NULL, model_id TEXT NOT NULL,
 config_revision INTEGER NOT NULL CHECK(config_revision>=1),
 prompt_version TEXT NOT NULL, settings_revision INTEGER NOT NULL CHECK(settings_revision>=1),
 dictionary_version TEXT NOT NULL,
 state TEXT NOT NULL CHECK(state IN ('running','success','partial','failed','cancelled','interrupted')),
 text TEXT NOT NULL DEFAULT '', search_key TEXT NOT NULL DEFAULT '',
 revision INTEGER NOT NULL DEFAULT 1 CHECK(revision>=1),
 created_at INTEGER NOT NULL, ended_at INTEGER,
 error_code TEXT
) STRICT;
CREATE TABLE processing_units (
 id TEXT PRIMARY KEY NOT NULL,
 run_id TEXT NOT NULL REFERENCES processing_runs(id) ON DELETE CASCADE,
 turn INTEGER NOT NULL CHECK(turn>0), source_start INTEGER NOT NULL CHECK(source_start>=0),
 source_end INTEGER NOT NULL CHECK(source_end>source_start),
 source_refs TEXT NOT NULL, source_text TEXT NOT NULL,
 candidate_text TEXT,
 generation_state TEXT NOT NULL CHECK(generation_state IN ('pending','success','failed','skipped','interrupted')),
 application_state TEXT NOT NULL CHECK(application_state IN ('pending','applied','handedOff','notApplied','uncertain','cancelled')),
 source_committed_end INTEGER CHECK(source_committed_end IS NULL OR source_committed_end BETWEEN source_start AND source_end),
 error_code TEXT, prompt_tokens INTEGER CHECK(prompt_tokens>=0),
 completion_tokens INTEGER CHECK(completion_tokens>=0),
 elapsed_ms INTEGER NOT NULL DEFAULT 0 CHECK(elapsed_ms>=0),
 created_at INTEGER NOT NULL, ended_at INTEGER,
 CHECK(json_valid(source_refs))
) STRICT;
CREATE INDEX processing_units_ordered ON processing_units(run_id,turn,source_start);
CREATE INDEX processing_runs_attempt ON processing_runs(attempt_id);
CREATE TABLE discarded_audio (
 id TEXT PRIMARY KEY NOT NULL REFERENCES audio_assets(id) ON DELETE CASCADE
) STRICT;
