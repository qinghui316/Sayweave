CREATE TABLE shortcut_settings (
  singleton INTEGER PRIMARY KEY NOT NULL CHECK(singleton=1),
  binding TEXT NOT NULL CHECK(length(binding) BETWEEN 1 AND 128),
  mode TEXT NOT NULL CHECK(mode IN ('toggle','hold')),
  revision INTEGER NOT NULL CHECK(revision>=1)
) STRICT;
INSERT INTO shortcut_settings VALUES(1,'AltRight','toggle',1);
CREATE TABLE delivery_attempts (
  task_id TEXT PRIMARY KEY NOT NULL,
  attempt_id TEXT NOT NULL REFERENCES speech_attempts(id) ON DELETE CASCADE,
  state TEXT NOT NULL CHECK(state IN ('pending','delivering','submitted','confirmed','failed','unconfirmed','copy')),
  error_code TEXT,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
) STRICT;
