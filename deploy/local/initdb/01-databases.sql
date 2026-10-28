-- One Postgres, two databases: the arena's tables and the runner's (credit ledger + jobs) never share a schema.
CREATE DATABASE arena;
CREATE DATABASE runner;
