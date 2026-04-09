-- Add display metadata so the UI can show human-readable names alongside
-- transcripts instead of bare pseudo_ids.

ALTER TABLE sessions ADD COLUMN title TEXT;
ALTER TABLE session_participants ADD COLUMN display_name TEXT;
ALTER TABLE session_participants ADD COLUMN character_name TEXT;
