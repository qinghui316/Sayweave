CREATE TABLE model_configs (
 id TEXT PRIMARY KEY NOT NULL,
 name TEXT NOT NULL CHECK(length(name) BETWEEN 1 AND 64),
 protocol TEXT NOT NULL CHECK(protocol='openai-chat'),
 base_url TEXT NOT NULL CHECK(length(base_url) BETWEEN 1 AND 2048),
 model_id TEXT NOT NULL CHECK(length(model_id) BETWEEN 1 AND 256),
 speech INTEGER NOT NULL CHECK(speech IN (0,1)),
 text INTEGER NOT NULL CHECK(text IN (0,1)),
 credential_ref TEXT NOT NULL UNIQUE,
 revision INTEGER NOT NULL CHECK(revision>=1),
 created_at INTEGER NOT NULL,
 updated_at INTEGER NOT NULL,
 CHECK(speech=1 OR text=1)
) STRICT;
CREATE TABLE model_defaults (
 usage TEXT PRIMARY KEY NOT NULL CHECK(usage IN ('speech','text')),
 config_id TEXT REFERENCES model_configs(id) ON DELETE RESTRICT,
 revision INTEGER NOT NULL CHECK(revision>=1)
) STRICT;
INSERT INTO model_defaults VALUES ('speech',NULL,1),('text',NULL,1);
CREATE TABLE credential_operations (
 credential_ref TEXT PRIMARY KEY NOT NULL,
 stage TEXT NOT NULL CHECK(stage IN ('prepared','cleanup')),
 created_at INTEGER NOT NULL
) STRICT;
CREATE TRIGGER model_default_insert BEFORE INSERT ON model_defaults
WHEN NEW.config_id IS NOT NULL AND NOT EXISTS(
 SELECT 1 FROM model_configs WHERE id=NEW.config_id AND
 ((NEW.usage='speech' AND speech=1) OR (NEW.usage='text' AND text=1)))
BEGIN SELECT RAISE(ABORT,'invalid default usage'); END;
CREATE TRIGGER model_usage_update BEFORE UPDATE OF speech,text ON model_configs
WHEN EXISTS(SELECT 1 FROM model_defaults WHERE config_id=NEW.id AND
 ((usage='speech' AND NEW.speech=0) OR (usage='text' AND NEW.text=0)))
BEGIN SELECT RAISE(ABORT,'referenced usage'); END;
CREATE TRIGGER model_default_update BEFORE UPDATE OF config_id ON model_defaults
WHEN NEW.config_id IS NOT NULL AND NOT EXISTS(
 SELECT 1 FROM model_configs WHERE id=NEW.config_id AND
 ((NEW.usage='speech' AND speech=1) OR (NEW.usage='text' AND text=1)))
BEGIN SELECT RAISE(ABORT,'invalid default usage'); END;

