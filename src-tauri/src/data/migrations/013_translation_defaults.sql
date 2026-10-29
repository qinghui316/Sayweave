-- Preserve custom choices and immutable schema12 checksums.
UPDATE translation_settings SET targets='["en-US","ja","ko","fr","zh-Hans"]',revision=revision+1
 WHERE revision=1 AND targets='["en-US"]' AND active_target='en-US';
UPDATE shortcut_settings SET translation_binding='AltRight+ShiftRight',revision=revision+1
 WHERE revision=1 AND translation_binding='F8' AND mode='toggle'
 AND binding='AltRight';
