-- Local/cloud share default ownership, while only cloud rows carry endpoints/secrets.
CREATE TABLE model_configs_new (
 id TEXT PRIMARY KEY NOT NULL,
 name TEXT NOT NULL CHECK(length(name) BETWEEN 1 AND 64),
 protocol TEXT CHECK(protocol IN ('openai-chat','qwen-audio-streaming','doubao-asr-streaming')),
 base_url TEXT CHECK(length(base_url) BETWEEN 1 AND 2048),
 model_id TEXT CHECK(length(model_id) BETWEEN 1 AND 256),
 speech INTEGER NOT NULL CHECK(speech IN (0,1)),
 text INTEGER NOT NULL CHECK(text IN (0,1)),
 credential_ref TEXT UNIQUE,
 revision INTEGER NOT NULL CHECK(revision>=1),
 created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL,
 source_kind TEXT NOT NULL DEFAULT 'cloud' CHECK(source_kind IN ('cloud','local')),
 asset_id TEXT, engine TEXT, device TEXT CHECK(device IN ('auto','cpu')),
 CHECK(speech=1 OR text=1),
 CHECK(protocol NOT IN ('qwen-audio-streaming','doubao-asr-streaming') OR (speech=1 AND text=0)),
 CHECK((source_kind='cloud' AND protocol IS NOT NULL AND base_url IS NOT NULL
   AND model_id IS NOT NULL AND credential_ref IS NOT NULL
   AND asset_id IS NULL AND engine IS NULL AND device IS NULL)
 OR (source_kind='local' AND protocol IS NULL AND base_url IS NULL AND model_id IS NULL
   AND credential_ref IS NULL AND speech=1 AND text=0
   AND asset_id IS NOT NULL AND engine IS NOT NULL AND device IS NOT NULL))
) STRICT;
CREATE TABLE model_defaults_new (
 usage TEXT PRIMARY KEY NOT NULL CHECK(usage IN ('speech','text')),
 config_id TEXT REFERENCES model_configs_new(id) ON DELETE RESTRICT,
 revision INTEGER NOT NULL CHECK(revision>=1)
) STRICT;
INSERT INTO model_configs_new(id,name,protocol,base_url,model_id,speech,text,credential_ref,revision,created_at,updated_at)
 SELECT id,name,protocol,base_url,model_id,speech,text,credential_ref,revision,created_at,updated_at FROM model_configs;
INSERT INTO model_defaults_new SELECT * FROM model_defaults;
DROP TABLE model_defaults;
DROP TABLE model_configs;
ALTER TABLE model_configs_new RENAME TO model_configs;
ALTER TABLE model_defaults_new RENAME TO model_defaults;
CREATE TRIGGER model_default_insert BEFORE INSERT ON model_defaults
WHEN NEW.config_id IS NOT NULL AND NOT EXISTS (
 SELECT 1 FROM model_configs WHERE id=NEW.config_id AND
 ((NEW.usage='speech' AND speech=1) OR (NEW.usage='text' AND text=1)))
BEGIN SELECT RAISE(ABORT,'invalid default usage'); END;
CREATE TRIGGER model_default_update BEFORE UPDATE OF config_id ON model_defaults
WHEN NEW.config_id IS NOT NULL AND NOT EXISTS (
 SELECT 1 FROM model_configs WHERE id=NEW.config_id AND
 ((NEW.usage='speech' AND speech=1) OR (NEW.usage='text' AND text=1)))
BEGIN SELECT RAISE(ABORT,'invalid default usage'); END;
CREATE TRIGGER model_usage_update BEFORE UPDATE OF speech,text ON model_configs
WHEN EXISTS (SELECT 1 FROM model_defaults WHERE config_id=NEW.id AND
 ((usage='speech' AND NEW.speech=0) OR (usage='text' AND NEW.text=0)))
BEGIN SELECT RAISE(ABORT,'default still uses configuration'); END;
ALTER TABLE speech_attempts ADD COLUMN source_kind TEXT NOT NULL DEFAULT 'cloud' CHECK(source_kind IN ('cloud','local'));
ALTER TABLE speech_attempts ADD COLUMN asset_id TEXT;
ALTER TABLE speech_attempts ADD COLUMN engine TEXT;
