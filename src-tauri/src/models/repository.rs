use super::types::*;
use crate::data::repository::{Repository, db_error, now};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
fn error(e: rusqlite::Error) -> ModelError {
    db_error(e).into()
}
fn overview(c: &Connection) -> Result<StoredOverview, ModelError> {
    let configs=c.prepare("SELECT id,name,protocol,base_url,model_id,speech,text,credential_ref,revision,created_at,updated_at FROM model_configs ORDER BY created_at ASC,id ASC").map_err(error)?
 .query_map([],|r|{let mut usages=vec![];if r.get::<_,bool>(5)? {usages.push(Usage::Speech);}
if r.get::<_,bool>(6)? {usages.push(Usage::Text);}
 Ok(StoredConfig{id:r.get(0)?,input:ConfigInput{name:r.get(1)?,protocol:r.get(2)?,base_url:r.get(3)?,model_id:r.get(4)?,usages},credential_ref:r.get(7)?,revision:r.get(8)?,created_at:r.get(9)?,updated_at:r.get(10)?})}).map_err(error)?
 .collect::<Result<Vec<_>,_>>().map_err(error)?;
    let defaults = c
        .prepare("SELECT usage,config_id,revision FROM model_defaults ORDER BY usage ASC")
        .map_err(error)?
        .query_map([], |r| {
            Ok(DefaultRecord {
                usage: if r.get::<_, String>(0)? == "speech" {
                    Usage::Speech
                } else {
                    Usage::Text
                },
                config_id: r.get(1)?,
                revision: r.get(2)?,
            })
        })
        .map_err(error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(error)?;
    Ok(StoredOverview { configs, defaults })
}
fn current<'a>(
    view: &'a StoredOverview,
    id: &str,
    rev: i64,
) -> Result<&'a StoredConfig, ModelError> {
    let c = view
        .configs
        .iter()
        .find(|c| c.id == id)
        .ok_or(ModelError::new("notFound"))?;
    if c.revision != rev {
        return Err(ModelError::new("conflict"));
    }
    Ok(c)
}
fn expectation(
    view: &StoredOverview,
    id: &str,
    usages: &[Usage],
    expected: &Expectation,
) -> Result<Vec<Replacement>, ModelError> {
    let replace = replacements(view, id, usages);
    if view.defaults != expected.defaults || replace != expected.replacements {
        return Err(ModelError::new("conflict"));
    }
    Ok(replace)
}
fn apply(c: &Connection, changes: &[Replacement]) -> Result<(), ModelError> {
    for d in changes {
        c.execute(
            "UPDATE model_defaults SET config_id=?2,revision=revision+1 WHERE usage=?1",
            params![d.usage.key(), d.config_id],
        )
        .map_err(error)?;
    }
    Ok(())
}
fn cleanup(c: &Connection, reference: &str) -> Result<(), ModelError> {
    c.execute(
        "INSERT OR IGNORE INTO credential_operations VALUES(?1,'cleanup',?2)",
        params![reference, now()],
    )
    .map_err(error)?;
    Ok(())
}
impl Repository {
    pub fn model_operation(&mut self, op: DbOperation) -> Result<DbOutput, ModelError> {
        match op {
            DbOperation::Overview => {
                let tx = self.connection.transaction().map_err(error)?;
                let view = overview(&tx)?;
                tx.commit().map_err(error)?;
                Ok(DbOutput::Overview(view))
            }
            DbOperation::Prepare(reference) => {
                self.connection
                    .execute(
                        "INSERT INTO credential_operations VALUES(?1,'prepared',?2)",
                        params![reference, now()],
                    )
                    .map_err(error)?;
                Ok(DbOutput::Done)
            }
            DbOperation::Pending => {
                let rows=self.connection.prepare("SELECT credential_ref FROM credential_operations ORDER BY created_at,credential_ref").map_err(error)?
    .query_map([],|r|r.get(0)).map_err(error)?.collect::<Result<Vec<String>,_>>().map_err(error)?;
                Ok(DbOutput::Pending(rows))
            }
            DbOperation::Referenced(reference) => {
                let exists = self
                    .connection
                    .query_row(
                        "SELECT EXISTS(SELECT 1 FROM model_configs WHERE credential_ref=?1)",
                        [reference],
                        |r| r.get(0),
                    )
                    .map_err(error)?;
                Ok(DbOutput::Referenced(exists))
            }
            DbOperation::Clear(reference) => {
                self.connection
                    .execute(
                        "DELETE FROM credential_operations WHERE credential_ref=?1",
                        [reference],
                    )
                    .map_err(error)?;
                Ok(DbOutput::Done)
            }
            DbOperation::Commit {
                input,
                id,
                revision,
                expectation: expected,
                credential_ref,
            } => {
                let input = input.validate()?;
                let tx = self
                    .connection
                    .transaction_with_behavior(TransactionBehavior::Immediate)
                    .map_err(error)?;
                let view = overview(&tx)?;
                let time = now();
                let id = if let Some(id) = id {
                    let old = current(&view, &id, revision.ok_or(ModelError::new("invalid"))?)?;
                    let changes = expectation(
                        &view,
                        &id,
                        &input.usages,
                        &expected.ok_or(ModelError::new("invalid"))?,
                    )?;
                    // Replace affected defaults BEFORE removing their usage so trigger checks are always true.
                    apply(&tx, &changes)?;
                    tx.execute("UPDATE model_configs SET name=?2,protocol=?3,base_url=?4,model_id=?5,speech=?6,text=?7,credential_ref=?8,revision=revision+1,updated_at=?9 WHERE id=?1",
      params![id,input.name,input.protocol,input.base_url,input.model_id,input.usages.contains(&Usage::Speech),input.usages.contains(&Usage::Text),credential_ref,time]).map_err(error)?;
                    if old.credential_ref != credential_ref {
                        cleanup(&tx, &old.credential_ref)?;
                    }
                    id
                } else {
                    if revision.is_some() || expected.is_some() {
                        return Err(ModelError::new("invalid"));
                    }
                    let id = uuid::Uuid::new_v4().to_string();
                    tx.execute(
                        "INSERT INTO model_configs VALUES(?1,?2,?3,?4,?5,?6,?7,?8,1,?9,?9)",
                        params![
                            id,
                            input.name,
                            input.protocol,
                            input.base_url,
                            input.model_id,
                            input.usages.contains(&Usage::Speech),
                            input.usages.contains(&Usage::Text),
                            credential_ref,
                            time
                        ],
                    )
                    .map_err(error)?;
                    id
                };
                // Fill only vacant applicable defaults, including a newly added use.
                for usage in &input.usages {
                    if !executable(&input, usage) {
                        continue;
                    }
                    tx.execute("UPDATE model_defaults SET config_id=?2,revision=revision+1 WHERE usage=?1 AND config_id IS NULL",params![usage.key(),id]).map_err(error)?;
                }
                tx.execute(
                    "DELETE FROM credential_operations WHERE credential_ref=?1",
                    [credential_ref],
                )
                .map_err(error)?;
                let result = overview(&tx)?;
                tx.commit().map_err(error)?;
                Ok(DbOutput::Overview(result))
            }
            DbOperation::Remove(req) => {
                let tx = self
                    .connection
                    .transaction_with_behavior(TransactionBehavior::Immediate)
                    .map_err(error)?;
                let view = overview(&tx)?;
                let old = current(&view, &req.id, req.revision)?;
                let changes = expectation(&view, &req.id, &[], &req.expectation)?;
                apply(&tx, &changes)?;
                tx.execute("DELETE FROM model_configs WHERE id=?1", [req.id])
                    .map_err(error)?;
                cleanup(&tx, &old.credential_ref)?;
                let result = overview(&tx)?;
                tx.commit().map_err(error)?;
                Ok(DbOutput::Overview(result))
            }
            DbOperation::SetDefault(req) => {
                let tx = self
                    .connection
                    .transaction_with_behavior(TransactionBehavior::Immediate)
                    .map_err(error)?;
                let applicable:Option<bool>=tx.query_row("SELECT CASE WHEN ?2='speech' THEN speech ELSE text END FROM model_configs WHERE id=?1",params![req.config_id,req.usage.key()],|r|r.get(0)).optional().map_err(error)?;
                let protocol: Option<String> = tx
                    .query_row(
                        "SELECT protocol FROM model_configs WHERE id=?1",
                        [&req.config_id],
                        |r| r.get(0),
                    )
                    .optional()
                    .map_err(error)?;
                if applicable != Some(true)
                    || !protocol
                        .as_deref()
                        .and_then(capabilities)
                        .is_some_and(|c| c.usages.contains(&req.usage))
                {
                    return Err(ModelError::new("invalid"));
                }
                let changed=tx.execute("UPDATE model_defaults SET config_id=?2,revision=revision+1 WHERE usage=?1 AND revision=?3",params![req.usage.key(),req.config_id,req.revision]).map_err(error)?;
                if changed != 1 {
                    return Err(ModelError::new("conflict"));
                }
                let result = overview(&tx)?;
                tx.commit().map_err(error)?;
                Ok(DbOutput::Overview(result))
            }
        }
    }
}
