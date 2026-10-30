ALTER TABLE shortcut_settings ADD COLUMN translation_binding_source TEXT NOT NULL DEFAULT 'legacyUnknown'
    CHECK(translation_binding_source IN ('default','custom','legacyUnknown'));
