ALTER TABLE speech_preferences ADD COLUMN input_gain_db INTEGER NOT NULL DEFAULT 0
 CHECK(input_gain_db IN (-6,-3,0,3,6,9,12));
