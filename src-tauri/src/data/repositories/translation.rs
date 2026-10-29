use crate::contracts::translation::*;
use crate::data::{
    model::DataError,
    repository::{Repository, db_error},
};
use rusqlite::params;

impl Repository {
    pub fn translation_operation(&mut self, op: DbOperation) -> Result<DbOutput, DataError> {
        match op {
            DbOperation::Load => {
                let (targets, active_target, revision): (String, String, i64) = self.connection.query_row(
                    "SELECT targets,active_target,revision FROM translation_settings WHERE singleton=1", [],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                ).map_err(db_error)?;
                let settings = Settings {
                    targets: serde_json::from_str(&targets)
                        .map_err(|_| DataError::new("unavailable"))?,
                    active_target,
                    revision,
                };
                settings.validate()?;
                Ok(DbOutput::Settings(settings))
            }
            DbOperation::Save(settings) => {
                settings.validate()?;
                let targets = serde_json::to_string(&settings.targets)
                    .map_err(|_| DataError::new("invalid"))?;
                let changed = self.connection.execute(
                    "UPDATE translation_settings SET targets=?1,active_target=?2,revision=revision+1 WHERE singleton=1 AND revision=?3",
                    params![targets, settings.active_target, settings.revision],
                ).map_err(db_error)?;
                if changed != 1 {
                    return Err(DataError::new("conflict"));
                }
                self.translation_operation(DbOperation::Load)
            }
        }
    }
}
