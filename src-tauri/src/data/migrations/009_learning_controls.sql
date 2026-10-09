CREATE TABLE automatic_term_settings (
 singleton INTEGER PRIMARY KEY NOT NULL CHECK(singleton=1),
 enabled INTEGER NOT NULL DEFAULT 1 CHECK(enabled IN (0,1)),
 revision INTEGER NOT NULL DEFAULT 1 CHECK(revision>=1)
) STRICT;
INSERT INTO automatic_term_settings(singleton,enabled,revision) VALUES(1,1,1);
ALTER TABLE language_edits ADD COLUMN automatic_terms_revision INTEGER;
ALTER TABLE language_edits ADD COLUMN personalization_revision INTEGER;
ALTER TABLE language_edits ADD COLUMN automatic_terms_state TEXT NOT NULL DEFAULT 'inactive'
 CHECK(automatic_terms_state IN ('inactive','pending','settled','cancelled'));
ALTER TABLE language_edits ADD COLUMN personalization_state TEXT NOT NULL DEFAULT 'inactive'
 CHECK(personalization_state IN ('inactive','pending','settled','cancelled'));
UPDATE language_edits SET state='expired',evidence='{}' WHERE state='pending';
