//! Read-only projections; neither queries text bodies nor schedules learning.
use super::*;
#[cfg(test)]
#[path = "language_dashboard_tests.rs"]
mod tests;
const ROWS: &str = "WITH valid AS (
 SELECT o.scope,count(DISTINCT o.id) count,max(o.created_at) updated
 FROM language_observations o WHERE o.kind='edit' OR EXISTS(
 SELECT 1 FROM speech_attempts a WHERE a.id=json_extract(o.observation,'$.sourceRef') AND a.status='success')
 GROUP BY o.scope), scopes AS (
 SELECT scope FROM valid UNION SELECT scope FROM language_profiles WHERE version>0), rows AS (
 SELECT s.scope,coalesce(p.version,0) version,
 coalesce(json_array_length(r.preferences),0) preferences,coalesce(v.count,0) observations,
 max(coalesce(p.published_at,0),coalesce(v.updated,0)) updated,
 (SELECT json_extract(o.observation,'$.application.name') FROM language_observations o WHERE o.scope=s.scope ORDER BY o.sequence DESC LIMIT 1) name,
 (SELECT json_extract(o.observation,'$.application.scene') FROM language_observations o WHERE o.scope=s.scope ORDER BY o.sequence DESC LIMIT 1) scene
 FROM scopes s LEFT JOIN valid v ON v.scope=s.scope LEFT JOIN language_profiles p ON p.scope=s.scope
 LEFT JOIN language_profile_revisions r ON r.scope=p.scope AND r.version=p.version)
";
fn display_name(scope: &str, name: Option<String>, scene: Option<String>) -> String {
    if scope == "shared" {
        return "通用习惯".into();
    }
    if scope == "web" {
        return "网页".into();
    }
    let name = name.unwrap_or_default();
    let friendly = match name.to_lowercase().as_str() {
        "chatgpt.exe" if scene.as_deref() == Some("technical") => "Codex",
        "code.exe" => "VS Code",
        "cursor.exe" => "Cursor",
        "wechat.exe" => "微信",
        "qq.exe" => "QQ",
        "winword.exe" => "Word",
        "outlook.exe" => "Outlook",
        "notepad.exe" => "记事本",
        _ => name.strip_suffix(".exe").unwrap_or(&name),
    };
    let result: String = friendly
        .chars()
        .filter(|c| !c.is_control())
        .take(128)
        .collect();
    if result.is_empty() {
        "应用".into()
    } else {
        result
    }
}
fn summary(row: &rusqlite::Row<'_>) -> rusqlite::Result<ProfileSummary> {
    let scope: String = row.get(0)?;
    Ok(ProfileSummary {
        name: display_name(&scope, row.get(5)?, row.get(6)?),
        scope,
        version: row.get(1)?,
        preference_count: row.get(2)?,
        observation_count: row.get(3)?,
        updated_at: row.get(4)?,
    })
}
pub(super) fn overview(
    db: &mut Connection,
    after: Option<String>,
) -> Result<ProfileOverview, DataError> {
    if after.as_deref().is_some_and(|v| !scope(v) || v == "shared") {
        return Err(DataError::new("invalid"));
    }
    let tx = db.transaction().map_err(db_error)?;
    let (preference_count,application_count,observation_count)=tx.query_row(
        &format!("{ROWS} SELECT coalesce(sum(preferences),0),coalesce(sum(CASE WHEN scope<>'shared' THEN 1 ELSE 0 END),0),coalesce(sum(observations),0) FROM rows"),[],
        |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(db_error)?;
    let shared=tx.query_row(&format!("{ROWS} SELECT scope,version,preferences,observations,updated,name,scene FROM rows WHERE scope='shared'"),[],summary).optional().map_err(db_error)?.unwrap_or(ProfileSummary {
        scope:"shared".into(),name:"通用习惯".into(),version:0,preference_count:0,observation_count:0,updated_at:0 });
    let mut applications = {
        let mut query=tx.prepare(&format!("{ROWS} SELECT scope,version,preferences,observations,updated,name,scene FROM rows WHERE scope<>'shared' AND scope>?1 ORDER BY scope LIMIT 21")).map_err(db_error)?;
        query
            .query_map([after.unwrap_or_default()], summary)
            .map_err(db_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(db_error)?
    };
    let more = applications.len() > 20;
    applications.truncate(20);
    let next = more.then(|| applications.last().unwrap().scope.clone());
    tx.commit().map_err(db_error)?;
    Ok(ProfileOverview {
        preference_count,
        application_count,
        observation_count,
        shared,
        applications,
        next,
    })
}
pub(super) fn profile(db: &Connection, requested: String) -> Result<ProfileDetails, DataError> {
    if !scope(&requested) {
        return Err(DataError::new("invalid"));
    }
    let latest:Option<(i64,i64,String)>=db.query_row("SELECT p.version,p.published_at,r.preferences FROM language_profiles p JOIN language_profile_revisions r ON r.scope=p.scope AND r.version=p.version WHERE p.scope=?1",[&requested],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(db_error)?;
    let (version, updated_at, rules) = latest.unwrap_or((0, 0, "[]".into()));
    let values: Vec<Preference> = parse(&rules)?;
    Ok(ProfileDetails {
        scope: requested,
        version,
        updated_at,
        preferences: values
            .into_iter()
            .map(|p| DisplayPreference {
                rule: p.rule,
                language: p.language,
                context: p.context,
            })
            .collect(),
    })
}
