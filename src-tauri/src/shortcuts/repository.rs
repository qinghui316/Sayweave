use super::types::*;
use crate::data::{
    model::DataError,
    repository::{Repository, db_error, now},
};
use rusqlite::params;
impl Repository {
    pub fn shortcut_operation(&mut self, op: DbOperation) -> Result<DbOutput, DataError> {
        match op {
            DbOperation::Load => self
                .connection
                .query_row(
                    "SELECT binding,mode,revision FROM shortcut_settings WHERE singleton=1",
                    [],
                    |r| {
                        Ok(DbOutput::Config(Config {
                            binding: r.get(0)?,
                            mode: if r.get::<_, String>(1)? == "hold" {
                                Mode::Hold
                            } else {
                                Mode::Toggle
                            },
                            revision: r.get(2)?,
                        }))
                    },
                )
                .map_err(db_error),
            DbOperation::Save(config) => {
                let config = config.validate().map_err(|_| DataError::new("invalid"))?;
                let changed=self.connection.execute("UPDATE shortcut_settings SET binding=?1,mode=?2,revision=revision+1 WHERE singleton=1 AND revision=?3",params![config.binding,if config.mode==Mode::Hold{"hold"}else{"toggle"},config.revision]).map_err(db_error)?;
                if changed != 1 {
                    return Err(DataError::new("conflict"));
                }
                self.shortcut_operation(DbOperation::Load)
            }
            DbOperation::Recover => {
                self.connection.execute("UPDATE delivery_attempts SET state='unconfirmed',error_code='interrupted',updated_at=?1 WHERE state IN ('pending','delivering','submitted')",[now()]).map_err(db_error)?;
                Ok(DbOutput::Done)
            }
            DbOperation::Deliveries(id) => {
                let mut query=self.connection.prepare("SELECT d.attempt_id,d.state,d.error_code,d.updated_at FROM delivery_attempts d JOIN speech_attempts a ON a.id=d.attempt_id WHERE a.record_id=?1 ORDER BY d.updated_at DESC,d.task_id ASC").map_err(db_error)?;
                let rows = query
                    .query_map([id], |r| {
                        Ok(DeliveryReport {
                            attempt_id: r.get(0)?,
                            state: r.get(1)?,
                            error: r.get(2)?,
                            updated_at: r.get(3)?,
                        })
                    })
                    .map_err(db_error)?;
                Ok(DbOutput::Deliveries(
                    rows.collect::<Result<Vec<_>, _>>().map_err(db_error)?,
                ))
            }
            DbOperation::Delivery {
                task,
                attempt,
                state,
                error,
            } => {
                let changed=self.connection.execute("INSERT INTO delivery_attempts(task_id,attempt_id,state,error_code,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?5) ON CONFLICT(task_id) DO UPDATE SET state=excluded.state,error_code=excluded.error_code,updated_at=excluded.updated_at WHERE delivery_attempts.attempt_id=excluded.attempt_id AND (delivery_attempts.state=excluded.state OR delivery_attempts.state='pending' AND excluded.state IN ('delivering','copy','failed','unconfirmed') OR delivery_attempts.state='delivering' AND excluded.state IN ('submitted','confirmed','copy','failed','unconfirmed') OR delivery_attempts.state='submitted' AND excluded.state IN ('confirmed','failed','unconfirmed'))",params![task,attempt,state,error,now()]).map_err(db_error)?;
                if changed != 1 {
                    return Err(DataError::new("conflict"));
                }
                Ok(DbOutput::Done)
            }
        }
    }
}
