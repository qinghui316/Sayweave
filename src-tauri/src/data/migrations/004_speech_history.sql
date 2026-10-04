CREATE TABLE speech_records (
 id TEXT PRIMARY KEY NOT NULL,
 created_at INTEGER NOT NULL, ended_at INTEGER,
 duration_ms INTEGER NOT NULL DEFAULT 0 CHECK(duration_ms>=0),
 status TEXT NOT NULL CHECK(status IN ('preparing','recording','processing','success','failed','interrupted','delete_pending')),
 current_attempt TEXT, revision INTEGER NOT NULL DEFAULT 1 CHECK(revision>=1)
) STRICT;
CREATE TABLE speech_attempts (
 id TEXT PRIMARY KEY NOT NULL,
 record_id TEXT NOT NULL REFERENCES speech_records(id) ON DELETE CASCADE,
 kind TEXT NOT NULL CHECK(kind IN ('live','retry')),
 config_id TEXT NOT NULL, config_name TEXT NOT NULL, model_id TEXT NOT NULL, service_host TEXT NOT NULL,
 config_revision INTEGER NOT NULL CHECK(config_revision>=1),
 created_at INTEGER NOT NULL, ended_at INTEGER,
 status TEXT NOT NULL CHECK(status IN ('preparing','recording','processing','success','failed','cancelled','interrupted')),
 text TEXT NOT NULL DEFAULT '', error_code TEXT, elapsed_ms INTEGER NOT NULL DEFAULT 0 CHECK(elapsed_ms>=0),
 search_key TEXT NOT NULL DEFAULT ''
) STRICT;
CREATE TABLE audio_assets (
 id TEXT PRIMARY KEY NOT NULL,
 record_id TEXT NOT NULL UNIQUE REFERENCES speech_records(id) ON DELETE CASCADE,
 file_key TEXT NOT NULL UNIQUE,
 status TEXT NOT NULL CHECK(status IN ('prepared','ready','delete_pending','expire_pending','expired','missing','corrupt')),
 sample_rate INTEGER NOT NULL DEFAULT 16000 CHECK(sample_rate=16000),
 channels INTEGER NOT NULL DEFAULT 1 CHECK(channels=1),
 bytes INTEGER NOT NULL DEFAULT 0 CHECK(bytes>=0),
 expires_at INTEGER
) STRICT;
CREATE TABLE speech_preferences (
 singleton INTEGER PRIMARY KEY NOT NULL CHECK(singleton=1),
 device_id TEXT,
 history_enabled INTEGER NOT NULL CHECK(history_enabled IN (0,1)),
 audio_enabled INTEGER NOT NULL CHECK(audio_enabled IN (0,1)),
 retention_days INTEGER CHECK(retention_days IN (7,30,90)),
 revision INTEGER NOT NULL CHECK(revision>=1)
) STRICT;
INSERT INTO speech_preferences VALUES(1,NULL,1,1,30,1);
CREATE INDEX speech_records_recent ON speech_records(created_at DESC,id ASC);
CREATE INDEX speech_attempts_record ON speech_attempts(record_id,created_at DESC,id ASC);
CREATE INDEX audio_assets_cleanup ON audio_assets(status,expires_at);

CREATE TRIGGER speech_current_attempt_guard BEFORE UPDATE OF current_attempt ON speech_records
WHEN NEW.current_attempt IS NOT NULL AND NOT EXISTS(SELECT 1 FROM speech_attempts WHERE id=NEW.current_attempt AND record_id=NEW.id)
BEGIN SELECT RAISE(ABORT,'attempt must belong to record'); END;
