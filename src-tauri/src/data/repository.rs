use super::model::*;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::Path,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub const MIGRATIONS: &[(&str, &str)] = &[
    (
        "dictionary_nfc_lower_v1",
        include_str!("migrations/001_dictionary.sql"),
    ),
    ("models_v1", include_str!("migrations/002_models.sql")),
    (
        "speech_protocol_v1",
        include_str!("migrations/003_speech_protocol.sql"),
    ),
    (
        "speech_history_v1",
        include_str!("migrations/004_speech_history.sql"),
    ),
    (
        "speech_adapters_v1",
        include_str!("migrations/005_speech_adapters.sql"),
    ),
    (
        "windows_dictation_v1",
        include_str!("migrations/006_dictation.sql"),
    ),
    (
        "streaming_processing_v1",
        include_str!("migrations/007_streaming_processing.sql"),
    ),
    (
        "language_learning_v1",
        include_str!("migrations/008_language_learning.sql"),
    ),
    (
        "learning_controls_v1",
        include_str!("migrations/009_learning_controls.sql"),
    ),
];
pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}
pub fn db_error(error: rusqlite::Error) -> DataError {
    log::warn!(target:"sayweave","SAYWEAVE_DATA sqlite_code={:?}", error.sqlite_error_code());
    match error.sqlite_error_code() {
        Some(rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked) => {
            DataError::new("busy")
        }
        _ => DataError::new("unavailable"),
    }
}
pub fn configure(connection: &Connection) -> Result<(), DataError> {
    connection
        .busy_timeout(Duration::from_secs(5))
        .map_err(db_error)?;
    connection
        .execute_batch(
            "PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON;",
        )
        .map_err(db_error)
}
pub fn verify(connection: &Connection) -> Result<(), DataError> {
    let result: String = connection
        .query_row("PRAGMA quick_check", [], |row| row.get(0))
        .map_err(db_error)?;
    if result == "ok" {
        Ok(())
    } else {
        Err(DataError::new("unavailable"))
    }
}
fn migration_checksum(sql: &str) -> String {
    format!("{:x}", Sha256::digest(sql.replace("\r\n", "\n").as_bytes()))
}
fn migration_checksum_matches(sql: &str, checksum: &str) -> bool {
    // Older Windows builds stored CRLF hashes. Accept only these two exact
    // newline representations; all other migration content remains frozen.
    let canonical = sql.replace("\r\n", "\n");
    checksum == migration_checksum(&canonical)
        || checksum
            == format!(
                "{:x}",
                Sha256::digest(canonical.replace('\n', "\r\n").as_bytes())
            )
}
pub fn migrate(
    connection: &mut Connection,
    root: &Path,
    migrations: &[(&str, &str)],
) -> Result<(), DataError> {
    verify(connection)?;
    let ledger: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='schema_migrations')",
            [],
            |r| r.get(0),
        )
        .map_err(db_error)?;
    let mut applied = Vec::<(u32, String, String)>::new();
    if ledger {
        let mut statement = connection
            .prepare("SELECT version,name,checksum FROM schema_migrations ORDER BY version")
            .map_err(db_error)?;
        applied = statement
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .map_err(db_error)?
            .collect::<Result<_, _>>()
            .map_err(db_error)?;
    } else {
        let count: u32 = connection
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE name NOT LIKE 'sqlite_%'",
                [],
                |r| r.get(0),
            )
            .map_err(db_error)?;
        if count != 0 {
            return Err(DataError::new("unavailable"));
        }
    }
    for (i, (version, name, checksum)) in applied.iter().enumerate() {
        let Some((expected_name, sql)) = migrations.get(i) else {
            return Err(DataError::new("unavailable"));
        };
        if *version != i as u32 + 1
            || name != expected_name
            || !migration_checksum_matches(sql, checksum)
        {
            return Err(DataError::new("unavailable"));
        }
    }
    if applied.len() == migrations.len() {
        return Ok(());
    }
    let backup = if !applied.is_empty() {
        let backups = root.join("backups");
        fs::create_dir_all(&backups).map_err(|_| DataError::new("unavailable"))?;
        let path = backups.join(format!(
            "{}-{}.protected.sqlite",
            now(),
            uuid::Uuid::new_v4()
        ));
        let mut destination = Connection::open(&path).map_err(db_error)?;
        rusqlite::backup::Backup::new(connection, &mut destination)
            .map_err(db_error)?
            .run_to_completion(64, Duration::from_millis(10), None)
            .map_err(db_error)?;
        verify(&destination)?;
        drop(destination);
        Some(path)
    } else {
        None
    };
    let tx = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(db_error)?;
    if !ledger {
        tx.execute_batch("CREATE TABLE schema_migrations(version INTEGER PRIMARY KEY NOT NULL,name TEXT NOT NULL,checksum TEXT NOT NULL,applied_at INTEGER NOT NULL) STRICT;").map_err(db_error)?;
    }
    for (i, (name, sql)) in migrations.iter().enumerate().skip(applied.len()) {
        tx.execute_batch(sql).map_err(db_error)?;
        tx.execute(
            "INSERT INTO schema_migrations VALUES(?1,?2,?3,?4)",
            params![i as u32 + 1, name, migration_checksum(sql), now()],
        )
        .map_err(db_error)?;
    }
    let violations: i64 = tx
        .query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| {
            r.get(0)
        })
        .map_err(db_error)?;
    if violations != 0 {
        return Err(DataError::new("unavailable"));
    }
    tx.commit().map_err(db_error)?;
    verify(connection)?;
    if let Some(path) = backup {
        let successful = path.with_file_name(
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .replace(".protected.", ".successful."),
        );
        fs::rename(&path, successful).map_err(|_| DataError::new("unavailable"))?;
        let mut files: Vec<_> = fs::read_dir(root.join("backups"))
            .map_err(|_| DataError::new("unavailable"))?
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .is_some_and(|s| s.to_string_lossy().ends_with(".successful.sqlite"))
            })
            .collect();
        files.sort();
        files.reverse();
        for old in files.into_iter().skip(3) {
            fs::remove_file(old).map_err(|_| DataError::new("unavailable"))?;
        }
    }
    Ok(())
}
pub struct Repository {
    pub connection: Connection,
    pub(super) archive_cursor: [usize; 2],
    pub(super) archive_causal: std::sync::Arc<super::archive_recovery::Causal>,
}
impl Repository {
    #[cfg(test)]
    pub fn open(root: &Path) -> Result<Self, DataError> {
        Self::open_with_causal(
            root,
            std::sync::Arc::new(super::archive_recovery::Causal::default()),
        )
    }
    pub(super) fn open_with_causal(
        root: &Path,
        archive_causal: std::sync::Arc<super::archive_recovery::Causal>,
    ) -> Result<Self, DataError> {
        let mut connection = Connection::open(root.join("dictionary.sqlite")).map_err(db_error)?;
        // Inspect before applying configuration so future/corrupt databases are not modified.
        connection
            .busy_timeout(Duration::from_secs(5))
            .map_err(db_error)?;
        migrate(&mut connection, root, MIGRATIONS)?;
        configure(&connection)?;
        let mut repository = Self {
            connection,
            archive_cursor: [0, 0],
            archive_causal,
        };
        if let Err(e) = super::archive_recovery::replay(&mut repository, root, true) {
            log::warn!(target:"sayweave", "operation=archive_recovery result={}", e.code);
        }
        super::cancellation::replay(&mut repository, root)?;
        if let Err(e) = super::archive_recovery::replay(&mut repository, root, false) {
            log::warn!(target:"sayweave", "operation=archive_recovery result={}", e.code);
        }
        Ok(repository)
    }
    pub(super) fn row(kind: Kind, row: &rusqlite::Row<'_>) -> rusqlite::Result<Record> {
        Ok(Record {
            id: row.get(0)?,
            kind,
            original: row.get(1)?,
            category: row.get(2)?,
            replacement: row.get(3)?,
            revision: row.get(4)?,
            created_at: row.get(5)?,
            updated_at: row.get(6)?,
            source: if row.get::<_, String>(7)? == "automatic" {
                Source::Automatic
            } else {
                Source::Manual
            },
        })
    }
    pub fn get(&self, kind: Kind, id: &str) -> Result<Record, DataError> {
        self.connection
            .query_row(
                &format!(
                    "SELECT {} FROM {} WHERE id=?1",
                    kind.columns(),
                    kind.table()
                ),
                [id],
                |r| Self::row(kind.clone(), r),
            )
            .optional()
            .map_err(db_error)?
            .ok_or(DataError::new("notFound"))
    }
    pub fn list(&mut self, query: Query) -> Result<Page, DataError> {
        if query.search.chars().count() > 128 || query.search.chars().any(char::is_control) {
            return Err(DataError::new("invalid"));
        }
        let search = key(&query.search);
        let predicate = match query.kind {
            Kind::Term => {
                "(?1='' OR instr(normalized_term,?1)>0 OR instr(coalesce(category_key,''),?1)>0)"
            }
            Kind::Correction => {
                "(?1='' OR instr(normalized_heard,?1)>0 OR instr(corrected_key,?1)>0)"
            }
        };
        let source = match query.source {
            Some(Source::Automatic) => "automatic",
            Some(Source::Manual) => "manual",
            None => "",
        };
        let predicate = format!(
            "{predicate} AND (?3='' OR {}=?3)",
            if query.kind == Kind::Term {
                "coalesce((SELECT source FROM language_term_sources WHERE term_id=dictionary_terms.id),'manual')"
            } else {
                "'manual'"
            }
        );
        let tx = self.connection.transaction().map_err(db_error)?;
        let total: u32 = tx
            .query_row(
                &format!(
                    "SELECT count(*) FROM {} WHERE {predicate}",
                    query.kind.table()
                ),
                params![search, 0, source],
                |r| r.get(0),
            )
            .map_err(db_error)?;
        let page = query.page.max(1).min(total.div_ceil(50).max(1));
        let records = {
            let mut statement = tx.prepare(&format!("SELECT {} FROM {} WHERE {predicate} ORDER BY updated_at DESC,id ASC LIMIT 50 OFFSET ?2", query.kind.columns(), query.kind.table())).map_err(db_error)?;
            statement
                .query_map(params![search, (page - 1) * 50, source], |r| {
                    Self::row(query.kind.clone(), r)
                })
                .map_err(db_error)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(db_error)?
        };
        tx.commit().map_err(db_error)?;
        Ok(Page {
            records,
            total,
            page,
        })
    }
    pub fn save(&mut self, input: Input, edit: Option<(String, i64)>) -> Result<Record, DataError> {
        let input = validate(input)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        let id = edit
            .as_ref()
            .map(|e| e.0.clone())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let previous_key: Option<String> = if input.kind == Kind::Term && edit.is_some() {
            tx.query_row(
                "SELECT normalized_term FROM dictionary_terms WHERE id=?1",
                [&id],
                |r| r.get(0),
            )
            .optional()
            .map_err(db_error)?
        } else {
            None
        };
        if let Some((_, revision)) = &edit {
            let current: Option<i64> = tx
                .query_row(
                    &format!("SELECT revision FROM {} WHERE id=?1", input.kind.table()),
                    [&id],
                    |r| r.get(0),
                )
                .optional()
                .map_err(db_error)?;
            match current {
                None => return Err(DataError::new("notFound")),
                Some(v) if v != *revision => return Err(DataError::new("conflict")),
                _ => (),
            }
        }
        let existing: Option<String> = tx
            .query_row(
                &format!(
                    "SELECT id FROM {} WHERE {}=?1 AND id<>?2",
                    input.kind.table(),
                    input.kind.key_column()
                ),
                params![key(&input.original), id],
                |r| r.get(0),
            )
            .optional()
            .map_err(db_error)?;
        if let Some(existing) = existing {
            return Err(DataError::duplicate(existing));
        }
        let time = now();
        match (&input.kind, edit) {
            (Kind::Term, None) => {
                tx.execute(
                    "INSERT INTO dictionary_terms VALUES(?1,?2,?3,?4,?5,1,?6,?6)",
                    params![
                        id,
                        input.original,
                        key(&input.original),
                        input.category,
                        input.category.as_deref().map(key),
                        time
                    ],
                )
                .map_err(db_error)?;
            }
            (Kind::Correction, None) => {
                tx.execute(
                    "INSERT INTO correction_rules VALUES(?1,?2,?3,?4,?5,1,?6,?6)",
                    params![
                        id,
                        input.original,
                        input.replacement,
                        key(&input.original),
                        input.replacement.as_deref().map(key),
                        time
                    ],
                )
                .map_err(db_error)?;
            }
            (Kind::Term, Some((_, rev))) => {
                tx.execute("UPDATE dictionary_terms SET term=?2,normalized_term=?3,category=?4,category_key=?5,revision=revision+1,updated_at=?6 WHERE id=?1 AND revision=?7", params![id, input.original, key(&input.original), input.category, input.category.as_deref().map(key), time, rev]).map_err(db_error)?;
            }
            (Kind::Correction, Some((_, rev))) => {
                tx.execute("UPDATE correction_rules SET heard=?2,corrected=?3,normalized_heard=?4,corrected_key=?5,revision=revision+1,updated_at=?6 WHERE id=?1 AND revision=?7", params![id, input.original, input.replacement, key(&input.original), input.replacement.as_deref().map(key), time, rev]).map_err(db_error)?;
            }
        }
        if input.kind == Kind::Term {
            if let Some(previous) = previous_key.filter(|old| old != &key(&input.original)) {
                tx.execute("INSERT INTO language_suppression(term_key,updated_at) VALUES(?1,?2) ON CONFLICT(term_key) DO UPDATE SET updated_at=excluded.updated_at",params![previous,time]).map_err(db_error)?;
            }
            tx.execute(
                "DELETE FROM language_suppression WHERE term_key=?1",
                [key(&input.original)],
            )
            .map_err(db_error)?;
            tx.execute("INSERT INTO language_term_sources(term_id,source,settings_revision,created_at) VALUES(?1,'manual',0,?2) ON CONFLICT(term_id) DO UPDATE SET source='manual',evidence_id=NULL",params![id,time]).map_err(db_error)?;
        }
        tx.commit().map_err(db_error)?;
        self.get(input.kind, &id)
    }
    pub fn delete(&mut self, kind: Kind, id: &str, revision: i64) -> Result<(), DataError> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        let current: Option<i64> = tx
            .query_row(
                &format!("SELECT revision FROM {} WHERE id=?1", kind.table()),
                [id],
                |r| r.get(0),
            )
            .optional()
            .map_err(db_error)?;
        match current {
            None => return Err(DataError::new("notFound")),
            Some(v) if v != revision => return Err(DataError::new("conflict")),
            _ => (),
        }
        if kind == Kind::Term {
            tx.execute("INSERT INTO language_suppression(term_key,updated_at) SELECT normalized_term,?2 FROM dictionary_terms WHERE id=?1 ON CONFLICT(term_key) DO UPDATE SET updated_at=excluded.updated_at",params![id,now()]).map_err(db_error)?;
        }
        tx.execute(
            &format!("DELETE FROM {} WHERE id=?1 AND revision=?2", kind.table()),
            params![id, revision],
        )
        .map_err(db_error)?;
        tx.commit().map_err(db_error)
    }
}
