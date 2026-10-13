use crate::contracts::{
    dictionary::{DataError, Kind},
    processing::{DbOperation, DbOutput, RunRecord, Settings, UnitRecord},
};
use crate::data::repository::{Repository, db_error, now};
use rusqlite::{OptionalExtension, params};
impl Repository {
    pub fn processing_operation(&mut self, operation: DbOperation) -> Result<DbOutput, DataError> {
        match operation {
            DbOperation::SaveDraft(checkpoint) => {
                if !checkpoint.valid() {
                    return Err(DataError::new("invalid"));
                }
                let source = serde_json::to_string(&checkpoint.source)
                    .map_err(|_| DataError::new("invalid"))?;
                let revision = i64::try_from(checkpoint.visible_revision)
                    .map_err(|_| DataError::new("invalid"))?;
                let order =
                    i64::try_from(checkpoint.order).map_err(|_| DataError::new("invalid"))?;
                let changed=self.connection.execute("INSERT INTO processing_draft_blocks(id,run_id,ordinal,source_snapshot,visible_revision,visible_text,state) SELECT ?1,?2,?3,?4,?5,?6,?7 WHERE EXISTS(SELECT 1 FROM processing_runs WHERE id=?2 AND (state='running' OR ?7!='preview')) ON CONFLICT(id) DO UPDATE SET source_snapshot=excluded.source_snapshot,visible_revision=excluded.visible_revision,visible_text=excluded.visible_text,state=excluded.state WHERE processing_draft_blocks.run_id=excluded.run_id AND processing_draft_blocks.ordinal=excluded.ordinal AND (processing_draft_blocks.state='preview' AND processing_draft_blocks.visible_revision<excluded.visible_revision OR processing_draft_blocks.visible_revision=excluded.visible_revision AND (processing_draft_blocks.visible_text=excluded.visible_text AND processing_draft_blocks.state='preview' OR processing_draft_blocks.state=excluded.state AND processing_draft_blocks.source_snapshot=excluded.source_snapshot AND processing_draft_blocks.visible_text=excluded.visible_text))",params![checkpoint.source.block,checkpoint.run_id,order,source,revision,checkpoint.visible_text,checkpoint.state]).map_err(db_error)?;
                if changed != 1 {
                    return Err(DataError::new("conflict"));
                }
                Ok(DbOutput::Done)
            }
            DbOperation::Prepare(run) => {
                if !["model", "native"].contains(&run.mode.as_str())
                    || run.mode == "native" && !run.config_id.is_empty()
                {
                    return Err(DataError::new("invalid"));
                }
                for id in [&run.id, &run.attempt_id, &run.task_id]
                    .into_iter()
                    .chain((run.mode == "model").then_some(&run.config_id))
                {
                    if uuid::Uuid::parse_str(id).is_err() {
                        return Err(DataError::new("invalid"));
                    }
                }
                if run.config_revision < 1 || run.settings_revision < 1 {
                    return Err(DataError::new("invalid"));
                }
                self.connection.execute("INSERT INTO processing_runs(id,attempt_id,task_id,config_id,config_name,model_id,config_revision,prompt_version,settings_revision,dictionary_version,state,created_at,mode) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,'running',?11,?12)",params![run.id,run.attempt_id,run.task_id,run.config_id,run.config_name,run.model_id,run.config_revision,run.prompt_version,run.settings_revision,run.dictionary_version,now(),run.mode]).map_err(db_error)?;
                Ok(DbOutput::Done)
            }
            DbOperation::SaveUnit(unit) => {
                validate_unit(&unit)?;
                let live = self
                    .connection
                    .query_row(
                        "SELECT state FROM processing_runs WHERE id=?1",
                        [&unit.run_id],
                        |r| r.get::<_, String>(0),
                    )
                    .optional()
                    .map_err(db_error)?
                    .ok_or(DataError::new("notFound"))?;
                if live != "running" {
                    return Err(DataError::new("conflict"));
                }
                let integer = |v: u64| i64::try_from(v).map_err(|_| DataError::new("invalid"));
                let turn = integer(unit.turn)?;
                let source_start = integer(unit.source_start as u64)?;
                let source_end = integer(unit.source_end as u64)?;
                let committed = unit
                    .source_committed_end
                    .map(|v| integer(v as u64))
                    .transpose()?;
                let prompt_tokens = unit.prompt_tokens.map(integer).transpose()?;
                let completion_tokens = unit.completion_tokens.map(integer).transpose()?;
                let elapsed = integer(unit.elapsed_ms)?;
                let changed=self.connection.execute("INSERT INTO processing_units(id,run_id,turn,source_start,source_end,source_refs,source_text,candidate_text,generation_state,application_state,source_committed_end,error_code,prompt_tokens,completion_tokens,elapsed_ms,created_at,ended_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17) ON CONFLICT(id) DO UPDATE SET candidate_text=excluded.candidate_text,generation_state=excluded.generation_state,application_state=excluded.application_state,source_committed_end=excluded.source_committed_end,error_code=excluded.error_code,prompt_tokens=excluded.prompt_tokens,completion_tokens=excluded.completion_tokens,elapsed_ms=excluded.elapsed_ms,ended_at=excluded.ended_at WHERE processing_units.run_id=excluded.run_id AND processing_units.turn=excluded.turn AND processing_units.source_start=excluded.source_start AND processing_units.source_end=excluded.source_end AND processing_units.source_refs=excluded.source_refs AND processing_units.source_text=excluded.source_text",params![unit.id,unit.run_id,turn,source_start,source_end,unit.source_refs,unit.source_text,unit.candidate_text,unit.generation_state,unit.application_state,committed,unit.error_code,prompt_tokens,completion_tokens,elapsed,now(),(unit.generation_state!="pending").then(now)]).map_err(db_error)?;
                if changed != 1 {
                    return Err(DataError::new("conflict"));
                }
                Ok(DbOutput::Done)
            }
            DbOperation::Cancel {
                id,
                text,
                raw,
                rollback_failed,
                units,
            } => {
                if text.len() > 2 * 1024 * 1024 || raw.len() > 1024 * 1024 {
                    return Err(DataError::new("invalid"));
                }
                let tx = self.connection.transaction().map_err(db_error)?;
                let (attempt,record):(String,String)=tx.query_row("SELECT p.attempt_id,a.record_id FROM processing_runs p JOIN speech_attempts a ON a.id=p.attempt_id WHERE p.id=?1 AND p.state='running'",[&id],|r|Ok((r.get(0)?,r.get(1)?))).map_err(db_error)?;
                let revisable: bool = tx
                    .query_row(
                        "SELECT EXISTS(SELECT 1 FROM processing_draft_blocks WHERE run_id=?1)",
                        [&id],
                        |r| r.get(0),
                    )
                    .map_err(db_error)?;
                if revisable {
                    tx.execute("UPDATE processing_units SET application_state=CASE WHEN application_state='pending' THEN 'notApplied' ELSE application_state END,generation_state=CASE WHEN generation_state='pending' THEN 'interrupted' ELSE generation_state END WHERE run_id=?1",[&id]).map_err(db_error)?;
                } else {
                    tx.execute("DELETE FROM processing_units WHERE run_id=?1", [&id])
                        .map_err(db_error)?;
                }
                for unit in units {
                    validate_unit(&unit)?;
                    if unit.run_id != id
                        || crate::contracts::draft::read(&unit.source_refs)
                            .map_err(|_| DataError::new("invalid"))?
                            .is_none()
                            && !["applied", "handedOff", "uncertain"]
                                .contains(&unit.application_state.as_str())
                    {
                        return Err(DataError::new("invalid"));
                    }
                    let turn = i64::try_from(unit.turn).map_err(|_| DataError::new("invalid"))?;
                    let integer = |v: u64| i64::try_from(v).map_err(|_| DataError::new("invalid"));
                    let committed = unit
                        .source_committed_end
                        .map(|v| integer(v as u64))
                        .transpose()?;
                    let elapsed = integer(unit.elapsed_ms)?;
                    let prompt_tokens = unit.prompt_tokens.map(integer).transpose()?;
                    let completion_tokens = unit.completion_tokens.map(integer).transpose()?;
                    let start =
                        i64::try_from(unit.source_start).map_err(|_| DataError::new("invalid"))?;
                    let end =
                        i64::try_from(unit.source_end).map_err(|_| DataError::new("invalid"))?;
                    tx.execute("INSERT INTO processing_units(id,run_id,turn,source_start,source_end,source_refs,source_text,candidate_text,generation_state,application_state,source_committed_end,error_code,prompt_tokens,completion_tokens,elapsed_ms,created_at,ended_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?16) ON CONFLICT(id) DO UPDATE SET candidate_text=excluded.candidate_text,generation_state=excluded.generation_state,application_state=excluded.application_state,source_committed_end=excluded.source_committed_end,error_code=excluded.error_code,prompt_tokens=excluded.prompt_tokens,completion_tokens=excluded.completion_tokens,elapsed_ms=excluded.elapsed_ms,ended_at=excluded.ended_at WHERE processing_units.run_id=excluded.run_id AND processing_units.turn=excluded.turn AND processing_units.source_start=excluded.source_start AND processing_units.source_end=excluded.source_end AND processing_units.source_refs=excluded.source_refs AND processing_units.source_text=excluded.source_text",params![unit.id,id,turn,start,end,unit.source_refs,unit.source_text,unit.candidate_text,unit.generation_state,unit.application_state,committed,unit.error_code,prompt_tokens,completion_tokens,elapsed,now()]).map_err(db_error)?;
                }
                tx.execute(
                    "UPDATE speech_attempts SET text=?2,search_key=?3 WHERE id=?1",
                    params![attempt, raw, crate::contracts::dictionary::key(&raw)],
                )
                .map_err(db_error)?;
                tx.execute("INSERT OR IGNORE INTO discarded_audio(id) SELECT id FROM audio_assets WHERE record_id=?1",[&record]).map_err(db_error)?;
                tx.execute(
                    "UPDATE audio_assets SET status='expire_pending' WHERE record_id=?1",
                    [&record],
                )
                .map_err(db_error)?;
                tx.execute("UPDATE speech_records SET status='interrupted',revision=revision+1 WHERE id=?1",[record]).map_err(db_error)?;
                tx.execute("UPDATE processing_runs SET state='cancelled',text=?2,search_key=?3,ended_at=?4,error_code=?5,revision=revision+1 WHERE id=?1",params![id,text,crate::contracts::dictionary::key(&text),now(),if rollback_failed {"compositionRollback"} else {"cancelled"}]).map_err(db_error)?;
                tx.commit().map_err(db_error)?;
                Ok(DbOutput::Done)
            }
            DbOperation::Finish {
                id,
                text,
                state,
                error,
            } => {
                if text.len() > 2 * 1024 * 1024
                    || !["success", "partial", "failed", "cancelled", "interrupted"]
                        .contains(&state.as_str())
                {
                    return Err(DataError::new("invalid"));
                }
                let tx = self.connection.transaction().map_err(db_error)?;
                let changed=tx.execute("UPDATE processing_runs SET state=?2,text=?3,search_key=?4,revision=revision+1,ended_at=?5,error_code=?6 WHERE id=?1 AND state='running'",params![id,state,text,crate::contracts::dictionary::key(&text),now(),error]).map_err(db_error)?;
                if changed != 1 {
                    return Err(DataError::new("conflict"));
                }
                tx.execute("UPDATE speech_records SET revision=revision+1 WHERE id=(SELECT a.record_id FROM processing_runs p JOIN speech_attempts a ON a.id=p.attempt_id WHERE p.id=?1)",[id]).map_err(db_error)?;
                tx.commit().map_err(db_error)?;
                Ok(DbOutput::Done)
            }
            DbOperation::Recover => {
                let tx = self.connection.transaction().map_err(db_error)?;
                let runs: Vec<String> = {
                    let mut query = tx
                        .prepare("SELECT id FROM processing_runs WHERE state='running'")
                        .map_err(db_error)?;
                    query
                        .query_map([], |r| r.get(0))
                        .map_err(db_error)?
                        .collect::<Result<_, _>>()
                        .map_err(db_error)?
                };
                for run in runs {
                    let bodies: Vec<String> = {
                        let mut query=tx.prepare("SELECT visible_text FROM processing_draft_blocks WHERE run_id=?1 AND state!='uncertain' ORDER BY ordinal").map_err(db_error)?;
                        query
                            .query_map([&run], |r| r.get(0))
                            .map_err(db_error)?
                            .collect::<Result<_, _>>()
                            .map_err(db_error)?
                    };
                    if !bodies.is_empty() {
                        let text = bodies.concat();
                        tx.execute(
                            "UPDATE processing_runs SET text=?2,search_key=?3 WHERE id=?1",
                            params![run, text, crate::contracts::dictionary::key(&text)],
                        )
                        .map_err(db_error)?;
                    }
                }
                tx.execute("UPDATE processing_units SET application_state=CASE WHEN application_state='pending' THEN 'uncertain' ELSE application_state END,generation_state=CASE WHEN generation_state='pending' THEN 'interrupted' ELSE generation_state END,error_code=coalesce(error_code,'interrupted'),ended_at=coalesce(ended_at,?1) WHERE run_id IN (SELECT id FROM processing_runs WHERE state='running')",[now()]).map_err(db_error)?;
                tx.execute("UPDATE processing_runs SET state='interrupted',error_code='interrupted',ended_at=?1,revision=revision+1 WHERE state='running'",[now()]).map_err(db_error)?;
                tx.commit().map_err(db_error)?;
                Ok(DbOutput::Done)
            }
            DbOperation::Settings => Ok(DbOutput::Settings(
                self.connection
                    .query_row(
                        "SELECT enabled,revision FROM processing_settings WHERE singleton=1",
                        [],
                        |r| {
                            Ok(Settings {
                                enabled: r.get(0)?,
                                revision: r.get(1)?,
                            })
                        },
                    )
                    .map_err(db_error)?,
            )),
            DbOperation::SaveSettings(settings) => {
                if settings.revision < 1 {
                    return Err(DataError::new("invalid"));
                }
                if self.connection.execute("UPDATE processing_settings SET enabled=?1,revision=revision+1 WHERE singleton=1 AND revision=?2",params![settings.enabled,settings.revision]).map_err(db_error)?!=1 {return Err(DataError::new("conflict"));}
                self.processing_operation(DbOperation::Settings)
            }
            DbOperation::DictionarySnapshot => {
                let tx = self.connection.transaction().map_err(db_error)?;
                let mut records = Vec::new();
                for kind in [Kind::Term, Kind::Correction] {
                    let mut query = tx
                        .prepare(&format!(
                            "SELECT {} FROM {} ORDER BY id",
                            kind.columns(),
                            kind.table()
                        ))
                        .map_err(db_error)?;
                    for row in query
                        .query_map([], |r| Self::row(kind.clone(), r))
                        .map_err(db_error)?
                    {
                        if records.len() >= 50000 {
                            return Err(DataError::new("dictionaryTooLarge"));
                        }
                        records.push(row.map_err(db_error)?);
                    }
                }
                tx.commit().map_err(db_error)?;
                Ok(DbOutput::Dictionary(records))
            }
        }
    }
}

pub(super) fn read_runs(
    connection: &rusqlite::Connection,
    record: &str,
) -> Result<Vec<RunRecord>, DataError> {
    let mut query=connection.prepare("SELECT p.id,p.attempt_id,p.config_name,p.model_id,p.state,p.text,p.revision,p.error_code,p.mode FROM processing_runs p JOIN speech_attempts a ON a.id=p.attempt_id WHERE a.record_id=?1 ORDER BY p.created_at DESC,p.id ASC").map_err(db_error)?;
    let mut runs = query
        .query_map([record], |r| {
            Ok(RunRecord {
                mode: r.get(8)?,
                id: r.get(0)?,
                attempt_id: r.get(1)?,
                config_name: r.get(2)?,
                model_id: r.get(3)?,
                state: r.get(4)?,
                text: r.get(5)?,
                revision: r.get(6)?,
                error_code: r.get(7)?,
                units: Vec::new(),
            })
        })
        .map_err(db_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_error)?;
    drop(query);
    for run in &mut runs {
        let mut query=connection.prepare("SELECT id,run_id,turn,source_start,source_end,source_refs,source_text,candidate_text,generation_state,application_state,source_committed_end,error_code,prompt_tokens,completion_tokens,elapsed_ms FROM processing_units WHERE run_id=?1 ORDER BY turn,source_start,id").map_err(db_error)?;
        run.units = query
            .query_map([&run.id], |r| {
                Ok(UnitRecord {
                    id: r.get(0)?,
                    run_id: r.get(1)?,
                    turn: r.get::<_, i64>(2)? as u64,
                    source_start: r.get::<_, i64>(3)? as usize,
                    source_end: r.get::<_, i64>(4)? as usize,
                    source_refs: r.get(5)?,
                    source_text: r.get(6)?,
                    candidate_text: r.get(7)?,
                    generation_state: r.get(8)?,
                    application_state: r.get(9)?,
                    source_committed_end: r.get::<_, Option<i64>>(10)?.map(|v| v as usize),
                    error_code: r.get(11)?,
                    prompt_tokens: r.get::<_, Option<i64>>(12)?.map(|v| v as u64),
                    completion_tokens: r.get::<_, Option<i64>>(13)?.map(|v| v as u64),
                    elapsed_ms: r.get::<_, i64>(14)? as u64,
                })
            })
            .map_err(db_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(db_error)?;
    }

    Ok(runs)
}

fn validate_unit(unit: &UnitRecord) -> Result<(), DataError> {
    if let Some(draft) =
        crate::contracts::draft::read(&unit.source_refs).map_err(|_| DataError::new("invalid"))?
        && draft.source.text != unit.source_text
    {
        return Err(DataError::new("invalid"));
    }
    if uuid::Uuid::parse_str(&unit.id).is_err()
        || uuid::Uuid::parse_str(&unit.run_id).is_err()
        || unit.turn == 0
        || unit.source_end <= unit.source_start
        || unit.source_text.len() > 1024 * 1024
        || unit
            .candidate_text
            .as_ref()
            .is_some_and(|s| s.len() > 16 * 1024)
        || unit.source_refs.len() > 64 * 1024
        || unit
            .source_committed_end
            .is_some_and(|end| end < unit.source_start || end > unit.source_end)
        || !["pending", "success", "failed", "skipped", "interrupted"]
            .contains(&unit.generation_state.as_str())
        || ![
            "pending",
            "applied",
            "handedOff",
            "notApplied",
            "uncertain",
            "cancelled",
        ]
        .contains(&unit.application_state.as_str())
    {
        return Err(DataError::new("invalid"));
    }
    Ok(())
}
