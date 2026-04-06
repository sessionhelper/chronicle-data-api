CREATE TABLE session_beats (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    session_id UUID NOT NULL REFERENCES sessions(id),
    beat_index INTEGER NOT NULL,
    start_time DOUBLE PRECISION NOT NULL,
    end_time DOUBLE PRECISION NOT NULL,
    title TEXT NOT NULL,
    summary TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(session_id, beat_index)
);

CREATE TABLE session_scenes (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    session_id UUID NOT NULL REFERENCES sessions(id),
    scene_index INTEGER NOT NULL,
    start_time DOUBLE PRECISION NOT NULL,
    end_time DOUBLE PRECISION NOT NULL,
    title TEXT NOT NULL,
    summary TEXT NOT NULL DEFAULT '',
    beat_start INTEGER NOT NULL,
    beat_end INTEGER NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(session_id, scene_index)
);

CREATE INDEX idx_beats_session ON session_beats(session_id);
CREATE INDEX idx_scenes_session ON session_scenes(session_id);

ALTER TABLE transcript_segments ADD COLUMN beat_id INTEGER;
ALTER TABLE transcript_segments ADD COLUMN chunk_group INTEGER;
ALTER TABLE transcript_segments ADD COLUMN excluded BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE transcript_segments ADD COLUMN exclude_reason TEXT;
