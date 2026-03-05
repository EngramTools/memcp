-- Rename workspace → project for clarity (workspace confused with IDE workspaces).
-- Backwards-compatible: application code accepts both field names via serde aliases.

ALTER TABLE memories RENAME COLUMN workspace TO project;

-- Rename the partial index created in 018_plugin_primitives.sql.
ALTER INDEX memories_workspace_idx RENAME TO memories_project_idx;
