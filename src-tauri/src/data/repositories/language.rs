//! Scoped language facts share the database worker; no network or native handles.
use crate::{
    contracts::{
        dictionary::{DataError, key},
        language::*,
    },
    data::repository::{Repository, db_error, now},
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
#[cfg(test)]
#[path = "language_tests.rs"]
mod tests;

fn settings(db: &Connection) -> Result<Settings, DataError> {
    db.query_row(
        "SELECT enabled,revision FROM language_settings WHERE singleton=1",
        [],
        |r| {
            Ok(Settings {
                enabled: r.get(0)?,
                revision: r.get(1)?,
            })
        },
    )
    .map_err(db_error)
}
fn live(db: &Connection, revision: i64) -> Result<(), DataError> {
    let current = settings(db)?;
    if current.enabled && current.revision == revision {
        Ok(())
    } else {
        Err(DataError::new("conflict"))
    }
}
fn scope(value: &str) -> bool {
    value == "web"
        || value == "shared"
        || value.starts_with("app:")
            && value.len() == 68
            && value[4..].bytes().all(|c| c.is_ascii_hexdigit())
}
fn bounded(value: &str, max: usize) -> bool {
    !value.is_empty()
        && value.chars().count() <= max
        && !value
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
}
fn json<T: serde::Serialize>(value: &T) -> Result<String, DataError> {
    serde_json::to_string(value).map_err(|_| DataError::new("invalid"))
}
fn parse<T: serde::de::DeserializeOwned>(value: &str) -> Result<T, DataError> {
    serde_json::from_str(value).map_err(|_| DataError::new("unavailable"))
}
fn preferences(
    db: &Connection,
    scope: &str,
) -> Result<(i64, Vec<Preference>, Vec<Preference>), DataError> {
    let latest:Option<(i64,String,String)>=db.query_row("SELECT r.version,r.preferences,r.candidates FROM language_profiles p JOIN language_profile_revisions r ON r.scope=p.scope AND r.version=p.version WHERE p.scope=?1",[scope],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(db_error)?;
    match latest {
        Some((version, ready, candidates)) => Ok((version, parse(&ready)?, parse(&candidates)?)),
        None => Ok((0, vec![], vec![])),
    }
}
fn validate_rules(scope: &str, values: &[Preference]) -> Result<(), DataError> {
    let mut seen = std::collections::HashSet::new();
    if values.len() > 6
        || values
            .iter()
            .map(|v| v.rule.chars().count() + v.context.as_deref().unwrap_or("").chars().count())
            .sum::<usize>()
            > 512
        || values.iter().any(|p| {
            p.scope != scope
                || !bounded(&p.rule, 256)
                || p.context.as_deref().is_some_and(|c| !bounded(c, 128))
                || p.evidence.is_empty()
                || p.evidence.len() > 8
                || !seen.insert(format!(
                    "{:?}:{}:{}",
                    p.language,
                    p.context.as_deref().unwrap_or(""),
                    key(&p.rule)
                ))
        })
    {
        Err(DataError::new("invalid"))
    } else {
        Ok(())
    }
}
fn changed_terms(before: &str, after: &str, original: &str, term: &str) -> bool {
    if original.is_empty() || term.is_empty() || original == term {
        return false;
    }
    let count = |text: &str, word: &str, other: &str| {
        // Do not count SQLit inside SQLite (or a short Chinese name inside its
        // corrected longer spelling). The ordered scan stays linear in range size.
        let excluded = if other.len() > word.len() && other.contains(word) {
            text.match_indices(other)
                .map(|(at, _)| at..at + other.len())
                .collect::<Vec<_>>()
        } else {
            vec![]
        };
        let mut index = 0;
        text.match_indices(word)
            .filter(|(at, _)| {
                while index < excluded.len() && excluded[index].end <= *at {
                    index += 1;
                }
                excluded.get(index).is_none_or(|r| !r.contains(at))
            })
            .count()
    };
    // Both quoted forms must participate in an actual replacement. Unchanged
    // words between two unrelated edits cannot acquire correction evidence.
    count(before, original, term) > count(after, original, term)
        && count(after, term, original) > count(before, term, original)
}
impl Repository {
    pub fn language_operation(&mut self, operation: DbOperation) -> Result<DbOutput, DataError> {
        match operation {
            DbOperation::ProfileAttempt {
                scope: requested,
                settings_revision,
                watermark,
                released,
            } => {
                if !scope(&requested) {
                    return Err(DataError::new("invalid"));
                }
                live(&self.connection, settings_revision)?;
                if released {
                    self.connection.execute("UPDATE language_profiles SET attempted_watermark=0,attempted_at=0 WHERE scope=?1 AND attempted_watermark=?2",params![requested,watermark]).map_err(db_error)?;
                } else {
                    self.connection.execute("INSERT INTO language_profiles(scope,attempted_watermark,attempted_at) VALUES(?1,?2,?3) ON CONFLICT(scope) DO UPDATE SET attempted_watermark=excluded.attempted_watermark,attempted_at=excluded.attempted_at",params![requested,watermark,now()]).map_err(db_error)?;
                }
                Ok(DbOutput::Done)
            }
            DbOperation::RetireEdit { id, reason } => {
                if reason.len() > 64 || !reason.bytes().all(|c| c.is_ascii_alphanumeric()) {
                    return Err(DataError::new("invalid"));
                }
                self.connection.execute("UPDATE language_edits SET state='failed',evidence='{}',decision=?2 WHERE id=?1 AND state='pending'",params![id,serde_json::json!({"error":reason}).to_string()]).map_err(db_error)?;
                Ok(DbOutput::Done)
            }
            DbOperation::PendingEdits => {
                let config = settings(&self.connection)?;
                if !config.enabled {
                    return Ok(DbOutput::PendingEdits(vec![]));
                }
                let mut query=self.connection.prepare("SELECT evidence FROM language_edits WHERE state='pending' AND settings_revision=?1 AND expires_at>?2 ORDER BY created_at,id LIMIT 16").map_err(db_error)?;
                let values = query
                    .query_map(params![config.revision, now()], |r| r.get::<_, String>(0))
                    .map_err(db_error)?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(db_error)?;
                Ok(DbOutput::PendingEdits(
                    values
                        .iter()
                        .map(|v| parse(v))
                        .collect::<Result<Vec<_>, _>>()?,
                ))
            }
            DbOperation::PendingScopes => {
                let config = settings(&self.connection)?;
                if !config.enabled {
                    return Ok(DbOutput::PendingScopes(vec![]));
                }
                let mut query=self.connection.prepare("SELECT o.scope FROM language_observations o LEFT JOIN language_profiles p ON p.scope=o.scope WHERE o.sequence>coalesce(p.watermark,0) AND o.settings_revision=?1 GROUP BY o.scope ORDER BY max(o.sequence) DESC LIMIT 63").map_err(db_error)?;
                let mut scopes = query
                    .query_map([config.revision], |r| r.get::<_, String>(0))
                    .map_err(db_error)?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(db_error)?;
                let shared_pending: bool = self.connection.query_row(
                    "SELECT EXISTS(SELECT 1 FROM language_observations WHERE settings_revision=?1 AND sequence>coalesce((SELECT watermark FROM language_profiles WHERE scope='shared'),0))",
                    [config.revision], |r|r.get(0)
                ).map_err(db_error)?;
                if shared_pending {
                    scopes.push("shared".into());
                }
                Ok(DbOutput::PendingScopes(scopes))
            }
            DbOperation::Expire => {
                self.connection.execute("UPDATE language_edits SET state='expired',evidence='{}' WHERE expires_at<?1 AND evidence<>'{}'",[now()]).map_err(db_error)?;
                Ok(DbOutput::Done)
            }
            DbOperation::LexicalContext { before, after } => {
                if before.len() > 65536 || after.len() > 65536 {
                    return Err(DataError::new("invalid"));
                }
                let mut query=self.connection.prepare("SELECT term,original,id FROM language_lexical_candidates WHERE instr(?1,original)>0 OR instr(?2,term)>0 ORDER BY created_at DESC,id LIMIT 16").map_err(db_error)?;
                let candidates=query.query_map(params![before,after],|r|Ok(serde_json::json!({"term":r.get::<_,String>(0)?,"original":r.get::<_,String>(1)?,"evidence_id":r.get::<_,String>(2)?}))).map_err(db_error)?.collect::<Result<Vec<_>,_>>().map_err(db_error)?;
                let mut query=self.connection.prepare("SELECT term FROM dictionary_terms WHERE instr(?1,term)>0 OR instr(?2,term)>0 ORDER BY updated_at DESC,id LIMIT 32").map_err(db_error)?;
                let terms = query
                    .query_map(params![before, after], |r| r.get::<_, String>(0))
                    .map_err(db_error)?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(db_error)?;
                Ok(DbOutput::LexicalContext(
                    serde_json::json!({"terms":terms,"ambiguous_candidates":candidates}),
                ))
            }
            DbOperation::Settings => Ok(DbOutput::Settings(settings(&self.connection)?)),
            DbOperation::SaveSettings(value) => {
                let tx = self
                    .connection
                    .transaction_with_behavior(TransactionBehavior::Immediate)
                    .map_err(db_error)?;
                let changed=tx.execute("UPDATE language_settings SET enabled=?1,revision=revision+1 WHERE singleton=1 AND revision=?2",params![value.enabled,value.revision]).map_err(db_error)?;
                if changed != 1 {
                    return Err(DataError::new("conflict"));
                }
                if !value.enabled {
                    tx.execute("UPDATE language_edits SET state='expired',evidence='{}' WHERE state='pending'",[]).map_err(db_error)?;
                }
                let result = settings(&tx)?;
                tx.commit().map_err(db_error)?;
                Ok(DbOutput::Settings(result))
            }
            DbOperation::Snapshot(application) => {
                let config = settings(&self.connection)?;
                let (profile_version, mut selected, _) =
                    preferences(&self.connection, &application.key)?;
                let (shared_version, shared, _) = preferences(&self.connection, "shared")?;
                selected.extend(shared);
                if !config.enabled {
                    selected.clear();
                }
                Ok(DbOutput::Snapshot(Snapshot {
                    settings: config,
                    application,
                    profile_version,
                    shared_version,
                    preferences: selected,
                }))
            }
            DbOperation::SaveEdit(evidence) => {
                if evidence.expires_at <= now() || evidence.expires_at > now() + 600000 {
                    return Err(DataError::new("invalid"));
                }
                if !scope(&evidence.application.key)
                    || evidence.application.key == "shared"
                    || uuid::Uuid::parse_str(&evidence.task).is_err()
                    || evidence.turn == 0
                    || evidence.operation == 0
                    || evidence.sequence == 0
                    || evidence.before == evidence.after
                    || !bounded(&evidence.before, 16384)
                    || evidence.after.chars().count() > 16384
                    || evidence.raw.chars().count() > 16384
                {
                    return Err(DataError::new("invalid"));
                }
                let tx = self.connection.transaction().map_err(db_error)?;
                live(&tx, evidence.settings_revision)?;
                // An immutable identity is never replaced by a repeated poll.
                tx.execute("INSERT OR IGNORE INTO language_edits(id,task_id,scope,settings_revision,evidence,state,created_at,expires_at) VALUES(?1,?2,?3,?4,?5,'pending',?6,?7)",params![evidence.id,evidence.task,evidence.application.key,evidence.settings_revision,json(&evidence)?,now(),evidence.expires_at]).map_err(db_error)?;
                tx.execute("UPDATE language_edits SET state='expired',evidence='{}' WHERE expires_at<?1 AND state='pending'",[now()]).map_err(db_error)?;
                let ready:Option<String>=tx.query_row("SELECT evidence FROM language_edits WHERE id=?1 AND state='pending' AND settings_revision=?2 AND expires_at>?3",params![evidence.id,evidence.settings_revision,now()],|r|r.get(0)).optional().map_err(db_error)?;
                tx.commit().map_err(db_error)?;
                Ok(DbOutput::EditReady(
                    ready.as_deref().map(parse).transpose()?,
                ))
            }
            DbOperation::ClassifyEdit {
                id,
                settings_revision,
                decision,
                analysis,
            } => {
                let tx = self
                    .connection
                    .transaction_with_behavior(TransactionBehavior::Immediate)
                    .map_err(db_error)?;
                live(&tx, settings_revision)?;
                let source:Option<String>=tx.query_row("SELECT evidence FROM language_edits WHERE id=?1 AND state='pending' AND settings_revision=?2 AND expires_at>=?3",params![id,settings_revision,now()],|r|r.get(0)).optional().map_err(db_error)?;
                let Some(source) = source else {
                    return Err(DataError::new("conflict"));
                };
                let source: EditEvidence = parse(&source)?;
                if matches!(decision.class, EditClass::Lexical | EditClass::Ambiguous) {
                    let original = decision
                        .original
                        .as_deref()
                        .filter(|s| bounded(s, 128))
                        .ok_or(DataError::new("invalid"))?;
                    let term = decision
                        .term
                        .as_deref()
                        .filter(|s| bounded(s, 128))
                        .ok_or(DataError::new("invalid"))?;
                    // Validate quoted edit evidence, not normal dictation output.
                    if key(original) == key(term) && original == term
                        || !source.before.contains(original)
                        || !source.after.contains(term)
                        || !changed_terms(&source.before, &source.after, original, term)
                        || term.chars().all(|c| {
                            c.is_ascii_digit() || c.is_whitespace() || c.is_ascii_punctuation()
                        })
                    {
                        return Err(DataError::new("invalid"));
                    }
                    let suppressed = tx
                        .query_row(
                            "SELECT EXISTS(SELECT 1 FROM language_suppression WHERE term_key=?1)",
                            [key(term)],
                            |r| r.get::<_, bool>(0),
                        )
                        .map_err(db_error)?;
                    if !suppressed {
                        if decision.class == EditClass::Lexical {
                            let term_id = uuid::Uuid::new_v4().to_string();
                            let changed=tx.execute("INSERT OR IGNORE INTO dictionary_terms(id,term,normalized_term,category,category_key,revision,created_at,updated_at) VALUES(?1,?2,?3,NULL,NULL,1,?4,?4)",params![term_id,term,key(term),now()]).map_err(db_error)?;
                            if changed == 1 {
                                tx.execute("INSERT INTO language_term_sources(term_id,source,evidence_id,settings_revision,created_at) VALUES(?1,'automatic',?2,?3,?4)",params![term_id,id,settings_revision,now()]).map_err(db_error)?;
                            } else {
                                let existing:Option<String>=tx.query_row("SELECT t.id FROM dictionary_terms t JOIN language_term_sources s ON s.term_id=t.id WHERE t.normalized_term=?1 AND s.source='automatic' AND t.term<>?2",params![key(term),term],|r|r.get(0)).optional().map_err(db_error)?;
                                if let Some(existing) = existing {
                                    tx.execute("UPDATE dictionary_terms SET term=?2,revision=revision+1,updated_at=?3 WHERE id=?1",params![existing,term,now()]).map_err(db_error)?;
                                    tx.execute("UPDATE language_term_sources SET evidence_id=?2,settings_revision=?3 WHERE term_id=?1",params![existing,id,settings_revision]).map_err(db_error)?;
                                }
                            }
                            tx.execute(
                                "DELETE FROM language_lexical_candidates WHERE term_key=?1",
                                [key(term)],
                            )
                            .map_err(db_error)?;
                        } else {
                            tx.execute("INSERT OR IGNORE INTO language_lexical_candidates(id,term_key,term,original,scope,created_at) VALUES(?1,?2,?3,?4,?5,?6)",params![id,key(term),term,original,source.application.key,now()]).map_err(db_error)?;
                        }
                    }
                } else if decision.class == EditClass::Style {
                    let summary = decision
                        .summary
                        .as_deref()
                        .filter(|s| bounded(s, 512))
                        .ok_or(DataError::new("invalid"))?;
                    let observation = StyleObservation {
                        id: id.clone(),
                        task: source.task.clone(),
                        application: source.application.clone(),
                        settings_revision,
                        language: WritingLanguage::of(&source.after),
                        kind: "edit".into(),
                        summary: summary.into(),
                        summary_complete: true,
                        source_ref: Some(id.clone()),
                    };
                    tx.execute("INSERT OR IGNORE INTO language_observations(id,task_id,scope,settings_revision,kind,observation,created_at) VALUES(?1,?2,?3,?4,'edit',?5,?6)",params![id,source.task,source.application.key,settings_revision,json(&observation)?,now()]).map_err(db_error)?;
                }
                tx.execute("UPDATE language_edits SET state='settled',decision=?2,analysis=?3,evidence='{}' WHERE id=?1",params![id,json(&decision)?,json(&analysis)?]).map_err(db_error)?;
                tx.commit().map_err(db_error)?;
                Ok(DbOutput::Done)
            }
            DbOperation::Observe(value) => {
                if !scope(&value.application.key)
                    || value.application.key == "shared"
                    || value.kind != "speech"
                    || value.summary.chars().count() > 512
                    || value.language == WritingLanguage::Any
                {
                    return Err(DataError::new("invalid"));
                }
                live(&self.connection, value.settings_revision)?;
                let attempt = value
                    .source_ref
                    .as_deref()
                    .ok_or(DataError::new("invalid"))?;
                let valid=self.connection.query_row("SELECT EXISTS(SELECT 1 FROM speech_attempts a JOIN delivery_attempts d ON d.attempt_id=a.id WHERE a.id=?1 AND a.status='success' AND d.task_id=?2)",params![attempt,value.task],|r|r.get::<_,bool>(0)).map_err(db_error)?;
                if !valid {
                    return Err(DataError::new("notFound"));
                }
                self.connection.execute("INSERT OR IGNORE INTO language_observations(id,task_id,scope,settings_revision,kind,observation,created_at) VALUES(?1,?2,?3,?4,'speech',?5,?6)",params![value.id,value.task,value.application.key,value.settings_revision,json(&value)?,now()]).map_err(db_error)?;
                Ok(DbOutput::Done)
            }
            DbOperation::Batch(requested) => {
                if !scope(&requested) {
                    return Err(DataError::new("invalid"));
                }
                let config = settings(&self.connection)?;
                if !config.enabled {
                    return Ok(DbOutput::Batch(None));
                }
                let (watermark, last, attempted): (i64, i64, i64) = self
                    .connection
                    .query_row(
                        "SELECT watermark,max(published_at,attempted_at),attempted_watermark FROM language_profiles WHERE scope=?1",
                        [&requested],
                        |r| Ok((r.get(0)?, r.get(1)?,r.get(2)?)),
                    )
                    .optional()
                    .map_err(db_error)?
                    .unwrap_or((0, 0,0));
                if now() - last < 600000 {
                    return Ok(DbOutput::Batch(None));
                }
                // Count all valid new evidence before selecting bounded model samples.
                // One noisy Session and one application cannot occupy the sampling window.
                let eligible = "(?1='shared' OR o.scope=?1) AND o.sequence>?2 AND o.settings_revision=?3 AND (o.kind='edit' OR EXISTS(SELECT 1 FROM speech_attempts a WHERE a.id=json_extract(o.observation,'$.sourceRef') AND a.status='success'))";
                let (speech,edits,apps):(i64,i64,i64)=self.connection.query_row(&format!("SELECT count(DISTINCT CASE WHEN o.kind='speech' THEN o.task_id END),count(DISTINCT CASE WHEN o.kind='edit' THEN o.task_id END),count(DISTINCT o.scope) FROM language_observations o WHERE {eligible}"),params![requested,watermark,config.revision],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(db_error)?;
                if speech < 10 && edits < 3 || requested == "shared" && apps < 2 {
                    return Ok(DbOutput::Batch(None));
                }
                let mut query=self.connection.prepare(&format!("WITH sessions AS (SELECT o.*,row_number() OVER(PARTITION BY o.task_id,o.kind ORDER BY o.sequence DESC) session_rank FROM language_observations o WHERE {eligible}), apps AS (SELECT *,row_number() OVER(PARTITION BY scope ORDER BY sequence DESC) app_rank FROM sessions WHERE session_rank=1) SELECT sequence,observation FROM apps ORDER BY app_rank,sequence DESC LIMIT 10")).map_err(db_error)?;
                let rows = query
                    .query_map(params![requested, watermark, config.revision], |r| {
                        Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
                    })
                    .map_err(db_error)?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(db_error)?;
                let mut observations = Vec::new();
                let mut through = watermark;
                for (sequence, value) in rows {
                    through = through.max(sequence);
                    let mut value: StyleObservation = parse(&value)?;
                    if value.kind == "speech" {
                        let body: Option<String> = self
                            .connection
                            .query_row(
                                "SELECT text FROM speech_attempts WHERE id=?1 AND status='success'",
                                [value.source_ref.as_deref().unwrap_or("")],
                                |r| r.get(0),
                            )
                            .optional()
                            .map_err(db_error)?;
                        let Some(body) = body else {
                            continue;
                        };
                        value.summary = body.chars().take(100).collect();
                        value.summary_complete = body.chars().count() <= 100;
                    }
                    observations.push(value);
                }
                let (version, current, candidates) = preferences(&self.connection, &requested)?;
                if through <= attempted {
                    return Ok(DbOutput::Batch(None));
                }
                // Read prior supporting facts, not just the previous model's prose.
                // These are bounded references; deleted speech bodies remain unavailable.
                let mut support = Vec::new();
                let ids = current
                    .iter()
                    .chain(&candidates)
                    .flat_map(|p| &p.evidence)
                    .collect::<std::collections::BTreeSet<_>>();
                for id in ids.into_iter().take(96) {
                    let row: Option<String> = self
                        .connection
                        .query_row(
                            "SELECT observation FROM language_observations WHERE id=?1",
                            [id],
                            |r| r.get(0),
                        )
                        .optional()
                        .map_err(db_error)?;
                    if let Some(row) = row {
                        let mut value: StyleObservation = parse(&row)?;
                        if value.kind == "speech" {
                            let body: Option<String> = self.connection.query_row(
                                "SELECT text FROM speech_attempts WHERE id=?1 AND status='success'",
                                [value.source_ref.as_deref().unwrap_or("")], |r| r.get(0)
                            ).optional().map_err(db_error)?;
                            let Some(body) = body else {
                                continue;
                            };
                            value.summary = body.chars().take(60).collect();
                            value.summary_complete = body.chars().count() <= 60;
                        }
                        support.push(value);
                    }
                }
                Ok(DbOutput::Batch(Some(ProfileBatch {
                    scope: requested,
                    settings_revision: config.revision,
                    version,
                    watermark: through,
                    current,
                    candidates,
                    observations,
                    support,
                })))
            }
            DbOperation::Publish(proposal) => {
                if !scope(&proposal.scope)
                    || !bounded(&proposal.reason, 512)
                    || proposal.evidence.is_empty()
                        && (!proposal.preferences.is_empty() || !proposal.candidates.is_empty())
                    || proposal.evidence.len() > 64
                {
                    return Err(DataError::new("invalid"));
                }
                validate_rules(&proposal.scope, &proposal.preferences)?;
                validate_rules(&proposal.scope, &proposal.candidates)?;
                let tx = self
                    .connection
                    .transaction_with_behavior(TransactionBehavior::Immediate)
                    .map_err(db_error)?;
                live(&tx, proposal.settings_revision)?;
                let (old, old_watermark): (i64, i64) = tx
                    .query_row(
                        "SELECT version,watermark FROM language_profiles WHERE scope=?1",
                        [&proposal.scope],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .optional()
                    .map_err(db_error)?
                    .unwrap_or((0, 0));
                if old != proposal.previous_version || proposal.watermark <= old_watermark {
                    return Err(DataError::new("conflict"));
                }
                let observed:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM language_observations WHERE sequence=?1 AND settings_revision=?2 AND (?3='shared' OR scope=?3))",params![proposal.watermark,proposal.settings_revision,proposal.scope],|r|r.get(0)).map_err(db_error)?;
                if !observed {
                    return Err(DataError::new("invalid"));
                }
                let (_, previous, previous_candidates) = preferences(&tx, &proposal.scope)?;
                let prior_ids = previous
                    .iter()
                    .chain(&previous_candidates)
                    .flat_map(|p| &p.evidence)
                    .collect::<std::collections::HashSet<_>>();
                let mut sources = std::collections::HashMap::new();
                let mut unique = std::collections::HashSet::new();
                for id in &proposal.evidence {
                    if !unique.insert(id) {
                        return Err(DataError::new("invalid"));
                    }
                    let (scope,sequence,revision,body):(String,i64,i64,String)=tx.query_row("SELECT scope,sequence,settings_revision,observation FROM language_observations WHERE id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().map_err(db_error)?.ok_or(DataError::new("notFound"))?;
                    if sequence > proposal.watermark
                        || proposal.scope != "shared" && scope != proposal.scope
                        || revision != proposal.settings_revision && !prior_ids.contains(id)
                    {
                        return Err(DataError::new("invalid"));
                    }
                    let source: StyleObservation = parse(&body)?;
                    sources.insert(id.as_str(), (scope, source.language));
                }
                for rule in proposal.preferences.iter().chain(&proposal.candidates) {
                    let mut apps = std::collections::HashSet::new();
                    let mut cited = std::collections::HashSet::new();
                    for id in &rule.evidence {
                        let (app, language) =
                            sources.get(id.as_str()).ok_or(DataError::new("invalid"))?;
                        if !cited.insert(id)
                            || rule.language != WritingLanguage::Any && *language != rule.language
                        {
                            return Err(DataError::new("invalid"));
                        }
                        apps.insert(app);
                    }
                    // Shared promotion is proved for each rule, not for an unrelated
                    // aggregate list of evidence supplied alongside the proposal.
                    if proposal.scope == "shared" && apps.len() < 2 {
                        return Err(DataError::new("invalid"));
                    }
                }
                tx.execute("INSERT INTO language_profile_revisions(scope,version,predecessor,watermark,settings_revision,preferences,candidates,evidence,reason,analysis,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",params![proposal.scope,old+1,old,proposal.watermark,proposal.settings_revision,json(&proposal.preferences)?,json(&proposal.candidates)?,json(&proposal.evidence)?,proposal.reason,json(&proposal.analysis)?,now()]).map_err(db_error)?;
                tx.execute("INSERT INTO language_profiles(scope,version,watermark,published_at) VALUES(?1,?2,?3,?4) ON CONFLICT(scope) DO UPDATE SET version=excluded.version,watermark=excluded.watermark,published_at=excluded.published_at",params![proposal.scope,old+1,proposal.watermark,now()]).map_err(db_error)?;
                tx.commit().map_err(db_error)?;
                Ok(DbOutput::Done)
            }
        }
    }
}
