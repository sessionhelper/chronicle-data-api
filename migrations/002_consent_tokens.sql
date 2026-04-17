-- Consent tokens: allow participants to manage consent via a unique URL
-- without requiring Discord OAuth.

CREATE TABLE consent_tokens (
  token UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  session_id UUID NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  participant_id UUID NOT NULL REFERENCES session_participants(id) ON DELETE CASCADE,
  pseudo_id TEXT NOT NULL REFERENCES users(pseudo_id) ON DELETE CASCADE,
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  expires_at TIMESTAMPTZ NOT NULL DEFAULT NOW() + INTERVAL '30 days',
  revoked_at TIMESTAMPTZ
);

CREATE INDEX idx_consent_tokens_participant ON consent_tokens(participant_id);
