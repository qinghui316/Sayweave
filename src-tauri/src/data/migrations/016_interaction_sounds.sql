ALTER TABLE speech_preferences ADD COLUMN interaction_sounds_enabled INTEGER NOT NULL
    DEFAULT 1 CHECK (interaction_sounds_enabled IN (0, 1));
