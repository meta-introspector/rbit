ALTER TABLE branch_protections ADD COLUMN require_code_owner_reviews INTEGER NOT NULL DEFAULT 0 CHECK (require_code_owner_reviews IN (0, 1));
