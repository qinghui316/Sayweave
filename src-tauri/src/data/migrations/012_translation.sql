CREATE TABLE translation_settings (
 singleton INTEGER PRIMARY KEY NOT NULL CHECK(singleton=1),
 targets TEXT NOT NULL DEFAULT '["en-US"]',
 active_target TEXT NOT NULL DEFAULT 'en-US',
 revision INTEGER NOT NULL DEFAULT 1 CHECK(revision>=1)
) STRICT;
INSERT INTO translation_settings(singleton) VALUES(1);
ALTER TABLE shortcut_settings ADD COLUMN translation_binding TEXT NOT NULL DEFAULT 'F8';
ALTER TABLE speech_attempts ADD COLUMN purpose TEXT NOT NULL DEFAULT 'dictation'
 CHECK(purpose IN ('dictation','translation'));
ALTER TABLE speech_attempts ADD COLUMN target_language TEXT;

-- Keep an existing F8 shortcut usable on upgrade.
UPDATE shortcut_settings SET translation_binding='F9' WHERE binding='F8' OR binding LIKE '%+F8';
