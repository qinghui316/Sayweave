ALTER TABLE processing_runs ADD COLUMN mode TEXT NOT NULL DEFAULT 'model' CHECK(mode IN ('model','native'));
CREATE TABLE processing_draft_blocks (
 id TEXT PRIMARY KEY NOT NULL,
 run_id TEXT NOT NULL REFERENCES processing_runs(id) ON DELETE CASCADE,
 ordinal INTEGER NOT NULL CHECK(ordinal>0),
 source_snapshot TEXT CHECK(source_snapshot IS NULL OR json_valid(source_snapshot)),
 visible_revision INTEGER NOT NULL DEFAULT 0 CHECK(visible_revision>=0),
 visible_text TEXT NOT NULL DEFAULT '',
 state TEXT NOT NULL DEFAULT 'preview' CHECK(state IN ('preview','delivered','stopped','uncertain')),
 UNIQUE(run_id,ordinal)
) STRICT;
CREATE TABLE processing_draft_versions (
 unit_id TEXT PRIMARY KEY NOT NULL REFERENCES processing_units(id) ON DELETE CASCADE,
 block_id TEXT NOT NULL REFERENCES processing_draft_blocks(id) ON DELETE CASCADE,
 content_version INTEGER NOT NULL CHECK(content_version>0),
 source_epoch INTEGER NOT NULL CHECK(source_epoch>0),
 baseline_kind TEXT NOT NULL CHECK(baseline_kind IN ('asr','editorSnapshot')),
 is_delivery INTEGER NOT NULL CHECK(is_delivery IN (0,1)),
 visible_revision INTEGER CHECK(visible_revision>=0)
) STRICT;
CREATE INDEX processing_draft_versions_block ON processing_draft_versions(block_id,content_version);
CREATE TRIGGER processing_draft_association AFTER INSERT ON processing_units
WHEN json_type(NEW.source_refs)='object'
BEGIN
 SELECT CASE WHEN json_extract(NEW.source_refs,'$.format') IS NOT 'revisable-v1'
 THEN RAISE(ABORT,'unsupported draft source') END;
 INSERT INTO processing_draft_blocks(id,run_id,ordinal)
 VALUES(json_extract(NEW.source_refs,'$.source.block'),NEW.run_id,json_extract(NEW.source_refs,'$.order'))
 ON CONFLICT(id) DO NOTHING;
 SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM processing_draft_blocks
 WHERE id=json_extract(NEW.source_refs,'$.source.block') AND run_id=NEW.run_id AND ordinal=json_extract(NEW.source_refs,'$.order'))
 THEN RAISE(ABORT,'draft identity conflict') END;
 INSERT INTO processing_draft_versions(unit_id,block_id,content_version,source_epoch,baseline_kind,is_delivery,visible_revision)
 VALUES(NEW.id,json_extract(NEW.source_refs,'$.source.block'),json_extract(NEW.source_refs,'$.source.version'),
 json_extract(NEW.source_refs,'$.source.sourceEpoch'),json_extract(NEW.source_refs,'$.source.baselineKind'),
 json_extract(NEW.source_refs,'$.delivery'),json_extract(NEW.source_refs,'$.visibleRevision'));
END;
