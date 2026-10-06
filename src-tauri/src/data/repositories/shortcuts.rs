use crate::contracts::shortcuts::*;
use crate::data::{
    model::DataError,
    repository::{Repository, db_error},
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
        }
    }
}
