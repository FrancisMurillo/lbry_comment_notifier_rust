CREATE TABLE comments (
  id VARCHAR PRIMARY KEY NOT NULL,
  account_id VARCHAR NOT NULL,
  claim_id VARCHAR NOT NULL,
  claim_name VARCHAR NOT NULL,
  commenter_id VARCHAR NOT NULL,
  commenter_name VARCHAR NOT NULL,
  commenter_url VARCHAR NOT NULL,
  comment TEXT NOT NULL,
  is_hidden BOOLEAN NOT NULL DEFAULT 'f',
  timestamp TIMESTAMP NOT NULL
);
