CREATE TABLE IF NOT EXISTS games (
  id TEXT PRIMARY KEY, revision INTEGER NOT NULL,
  credentials BLOB NOT NULL, checkpoint BLOB NOT NULL
);
CREATE TABLE IF NOT EXISTS accepted_actions (
  game_id TEXT NOT NULL REFERENCES games(id) ON DELETE CASCADE,
  hero_id TEXT NOT NULL, action_id TEXT NOT NULL,
  command BLOB NOT NULL, reply BLOB NOT NULL,
  PRIMARY KEY (game_id, hero_id, action_id)
);
CREATE TABLE IF NOT EXISTS replay_starts (
  game_id TEXT PRIMARY KEY REFERENCES games(id) ON DELETE CASCADE,
  revision INTEGER NOT NULL, checkpoint BLOB NOT NULL
);
CREATE TABLE IF NOT EXISTS replay_entries (
  game_id TEXT NOT NULL REFERENCES replay_starts(game_id) ON DELETE CASCADE,
  revision INTEGER NOT NULL, entry BLOB NOT NULL,
  PRIMARY KEY (game_id, revision)
);
CREATE TABLE IF NOT EXISTS reported_replays (
  game_id TEXT PRIMARY KEY, revision INTEGER NOT NULL, archive BLOB NOT NULL
);
CREATE TABLE IF NOT EXISTS replay_shares (
  token TEXT PRIMARY KEY, game_id TEXT NOT NULL UNIQUE,
  total INTEGER NOT NULL, metadata BLOB NOT NULL
);
CREATE TABLE IF NOT EXISTS replay_share_groups (
  token TEXT NOT NULL REFERENCES replay_shares(token) ON DELETE CASCADE,
  start INTEGER NOT NULL, gzip BLOB NOT NULL, PRIMARY KEY (token, start)
);
INSERT OR IGNORE INTO replay_starts (game_id, revision, checkpoint)
  SELECT id, revision, checkpoint FROM games;
