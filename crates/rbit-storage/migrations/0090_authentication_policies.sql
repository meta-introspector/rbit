-- Optional organization enrollment policy. Site policy uses instance_settings.
CREATE TABLE org_authentication_policies (
    org_id INTEGER PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
    require_two_factor INTEGER NOT NULL DEFAULT 0 CHECK (require_two_factor IN (0, 1))
) STRICT;
