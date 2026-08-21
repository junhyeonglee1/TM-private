ALTER TABLE projects
ADD COLUMN system_key TEXT
CHECK (system_key IS NULL OR system_key = 'uncategorized');

-- Adopt the oldest active project already named "기타". Existing duplicates are
-- intentionally preserved; system_key, rather than the display name, is identity.
UPDATE projects
SET name = '기타',
    system_key = 'uncategorized'
WHERE id = (
    SELECT id
    FROM projects
    WHERE deleted_at IS NULL
      AND archived_at IS NULL
      AND trim(name) = '기타'
    ORDER BY created_at ASC, id ASC
    LIMIT 1
);

INSERT INTO projects(
    id, name, description, color, sort_order, created_at, updated_at,
    archived_at, deleted_at, system_key
)
SELECT
    tm_uuid_v7(), '기타', '', '#7386ff', 0, tm_now_utc(), tm_now_utc(),
    NULL, NULL, 'uncategorized'
WHERE NOT EXISTS (
    SELECT 1 FROM projects WHERE system_key = 'uncategorized'
);

CREATE UNIQUE INDEX idx_projects_system_key
    ON projects(system_key)
    WHERE system_key IS NOT NULL;

-- Preserve the user's historical ordering timestamp while recording the domain
-- change through the existing task.changed trigger and optimistic version.
UPDATE tasks
SET project_id = (
        SELECT id FROM projects WHERE system_key = 'uncategorized'
    ),
    version = version + 1
WHERE project_id IS NULL;

CREATE TRIGGER tasks_project_required_insert
BEFORE INSERT ON tasks
WHEN NEW.project_id IS NULL
BEGIN
    SELECT RAISE(ABORT, 'task project is required');
END;

CREATE TRIGGER tasks_project_required_update
BEFORE UPDATE OF project_id ON tasks
WHEN NEW.project_id IS NULL
BEGIN
    SELECT RAISE(ABORT, 'task project is required');
END;

CREATE TRIGGER projects_uncategorized_protect_update
BEFORE UPDATE ON projects
WHEN OLD.system_key = 'uncategorized'
 AND (
    NEW.system_key IS NOT OLD.system_key
    OR NEW.name IS NOT OLD.name
    OR NEW.archived_at IS NOT OLD.archived_at
    OR NEW.deleted_at IS NOT OLD.deleted_at
 )
BEGIN
    SELECT RAISE(ABORT, 'uncategorized project identity and active state are immutable');
END;

CREATE TRIGGER projects_uncategorized_protect_delete
BEFORE DELETE ON projects
WHEN OLD.system_key = 'uncategorized'
BEGIN
    SELECT RAISE(ABORT, 'uncategorized project cannot be deleted');
END;

CREATE TRIGGER projects_uncategorized_name_reserved_insert
BEFORE INSERT ON projects
WHEN NEW.system_key IS NULL AND trim(NEW.name) = '기타'
BEGIN
    SELECT RAISE(ABORT, 'project name 기타 is reserved');
END;

CREATE TRIGGER projects_uncategorized_name_reserved_update
BEFORE UPDATE OF name, system_key ON projects
WHEN NEW.system_key IS NULL
 AND trim(NEW.name) = '기타'
 AND trim(OLD.name) <> '기타'
BEGIN
    SELECT RAISE(ABORT, 'project name 기타 is reserved');
END;
