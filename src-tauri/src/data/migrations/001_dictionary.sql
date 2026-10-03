CREATE TABLE dictionary_terms (
 id TEXT PRIMARY KEY NOT NULL,
 term TEXT NOT NULL CHECK(length(term) BETWEEN 1 AND 128),
 normalized_term TEXT NOT NULL UNIQUE,
 category TEXT CHECK(category IS NULL OR length(category) BETWEEN 1 AND 64),
 category_key TEXT,
 revision INTEGER NOT NULL CHECK(revision > 0),
 created_at INTEGER NOT NULL,
 updated_at INTEGER NOT NULL,
 CHECK((category IS NULL) = (category_key IS NULL))
) STRICT;
CREATE INDEX terms_updated ON dictionary_terms(updated_at DESC, id ASC);
CREATE TABLE correction_rules (
 id TEXT PRIMARY KEY NOT NULL,
 heard TEXT NOT NULL CHECK(length(heard) BETWEEN 1 AND 128),
 corrected TEXT NOT NULL CHECK(length(corrected) BETWEEN 1 AND 128),
 normalized_heard TEXT NOT NULL UNIQUE,
 corrected_key TEXT NOT NULL,
 revision INTEGER NOT NULL CHECK(revision > 0),
 created_at INTEGER NOT NULL,
 updated_at INTEGER NOT NULL
) STRICT;
CREATE INDEX corrections_updated ON correction_rules(updated_at DESC, id ASC);
