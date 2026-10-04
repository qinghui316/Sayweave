use super::types::*;
use crate::data::repository::{Repository, db_error, now};
use rusqlite::{Connection, OptionalExtension, params};
fn audio(row: &rusqlite::Row<'_>) -> rusqlite::Result<Audio> {
    Ok(Audio {
        id: row.get(0)?,
        record_id: row.get(1)?,
        file_key: row.get(2)?,
        status: row.get(3)?,
        bytes: row.get::<_, i64>(4)? as u64,
        expires_at: row.get(5)?,
    })
}
fn assets(c: &Connection, filter: &str, at: i64) -> Result<Vec<Audio>, Error> {
    c.prepare(&format!(
        "SELECT id,record_id,file_key,status,bytes,expires_at FROM audio_assets WHERE {filter}"
    ))
    .map_err(db_error)?
    .query_map([at], audio)
    .map_err(db_error)?
    .collect::<Result<Vec<_>, _>>()
    .map_err(db_error)
}
fn detail(c: &Connection, id: &str) -> Result<Record, Error> {
    let mut record=c.query_row("SELECT id,created_at,ended_at,duration_ms,status,current_attempt,revision FROM speech_records WHERE id=?1",[id],|r|Ok(Record{id:r.get(0)?,created_at:r.get(1)?,ended_at:r.get(2)?,duration_ms:r.get::<_,i64>(3)? as u64,status:r.get(4)?,current_attempt:r.get(5)?,revision:r.get(6)?,attempts:vec![],audio:None})).optional().map_err(db_error)?.ok_or(Error::new("notFound"))?;
    record.attempts=c.prepare("SELECT id,record_id,kind,config_id,config_name,model_id,service_host,created_at,ended_at,status,text,error_code,elapsed_ms,config_revision FROM speech_attempts WHERE record_id=?1 ORDER BY created_at DESC,id ASC").map_err(db_error)?.query_map([id],|r|Ok(Attempt{id:r.get(0)?,record_id:r.get(1)?,kind:r.get(2)?,config_id:r.get(3)?,config_name:r.get(4)?,model_id:r.get(5)?,service_host:r.get(6)?,created_at:r.get(7)?,ended_at:r.get(8)?,status:r.get(9)?,text:r.get(10)?,error_code:r.get(11)?,elapsed_ms:r.get::<_,i64>(12)? as u64,config_revision:r.get(13)?})).map_err(db_error)?.collect::<Result<Vec<_>,_>>().map_err(db_error)?;
    record.audio=c.query_row("SELECT id,record_id,file_key,status,bytes,expires_at FROM audio_assets WHERE record_id=?1",[id],audio).optional().map_err(db_error)?;
    Ok(record)
}
fn insert_attempt(c: &Connection, a: &Attempt) -> Result<(), Error> {
    c.execute("INSERT INTO speech_attempts(id,record_id,kind,config_id,config_name,model_id,service_host,created_at,config_revision,status) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,'preparing')",params![a.id,a.record_id,a.kind,a.config_id,a.config_name,a.model_id,a.service_host,a.created_at,a.config_revision]).map_err(db_error)?;
    Ok(())
}
impl Repository {
    pub fn history_operation(&mut self, op: DbOperation) -> Result<DbOutput, Error> {
        match op {
            DbOperation::Browse(q) => {
                if q.search.chars().count() > 256 || q.search.chars().any(char::is_control)
                    || q.cursor.as_ref().is_some_and(|c| c.created_at < 0 || uuid::Uuid::parse_str(&c.id).is_err()) {
                    return Err(Error::new("invalid"));
                }
                let tx = self.connection.transaction().map_err(db_error)?;
                let search = crate::data::model::key(&q.search);
                let filter = "(?1='' OR EXISTS(SELECT 1 FROM speech_attempts a WHERE a.record_id=r.id AND instr(a.search_key,?1)>0))";
                let total = tx.query_row(&format!("SELECT count(*) FROM speech_records r WHERE {filter}"), [&search], |r| r.get(0)).map_err(db_error)?;
                let at = q.cursor.as_ref().map(|c| c.created_at);
                let anchor = q.cursor.as_ref().map(|c| c.id.as_str()).unwrap_or("");
                // Select only a bounded preview. Full attempt bodies stay behind detail().
                let sql = format!("SELECT r.id,r.created_at,r.duration_ms,r.status,r.revision,r.current_attempt,
                    COALESCE((SELECT a.id FROM speech_attempts a WHERE a.id=r.current_attempt AND (?1='' OR instr(a.search_key,?1)>0)),
                    (SELECT a.id FROM speech_attempts a WHERE a.record_id=r.id AND (?1='' OR instr(a.search_key,?1)>0)
                     ORDER BY a.created_at DESC,a.id ASC LIMIT 1))
                    FROM speech_records r WHERE {filter} AND (?2 IS NULL OR r.created_at<?2 OR (r.created_at=?2 AND r.id>?3))
                    ORDER BY r.created_at DESC,r.id ASC LIMIT 51");
                let rows = tx.prepare(&sql).map_err(db_error)?.query_map(params![search,at,anchor], |r| Ok((
                    r.get::<_,String>(0)?, r.get::<_,i64>(1)?, r.get::<_,i64>(2)?, r.get::<_,String>(3)?,
                    r.get::<_,i64>(4)?, r.get::<_,Option<String>>(5)?, r.get::<_,Option<String>>(6)?
                ))).map_err(db_error)?.collect::<Result<Vec<_>,_>>().map_err(db_error)?;
                let more = rows.len() > 50;
                let mut records = Vec::with_capacity(rows.len().min(50));
                for (id,created_at,duration_ms,status,revision,current,display_attempt) in rows.into_iter().take(50) {
                    let preview = if let Some(a) = &display_attempt {
                        tx.query_row("SELECT substr(text,1,240),length(text)>240 FROM speech_attempts WHERE id=?1", [a], |r| Ok((r.get::<_,String>(0)?,r.get::<_,bool>(1)?))).map_err(db_error)?
                    } else { (String::new(),false) };
                    let latest = tx.query_row("SELECT id,status,error_code FROM speech_attempts WHERE record_id=?1 ORDER BY created_at DESC,id ASC LIMIT 1", [&id], |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,Option<String>>(2)?))).optional().map_err(db_error)?;
                    let audio = tx.query_row("SELECT id,record_id,file_key,status,bytes,expires_at FROM audio_assets WHERE record_id=?1", [&id], audio).optional().map_err(db_error)?;
                    records.push(Summary {id,created_at,duration_ms:duration_ms as u64,status,revision,
                        matched_old_version:!search.is_empty() && display_attempt != current,
                        display_attempt,preview:preview.0,truncated:preview.1,
                        latest_attempt:latest.as_ref().map(|a|a.0.clone()),latest_status:latest.as_ref().map(|a|a.1.clone()),
                        latest_error:latest.and_then(|a|a.2),audio});
                }
                let next_cursor = if more { records.last().map(|r| Cursor {created_at:r.created_at,id:r.id.clone()}) } else {None};
                tx.commit().map_err(db_error)?;
                Ok(DbOutput::Feed(Feed {records,total,next_cursor}))
            },
            DbOperation::Stage{attempt_id,status}=>{
                if !["recording","processing"].contains(&status.as_str()){return Err(Error::new("invalid"));}
                let tx=self.connection.transaction().map_err(db_error)?;
                tx.execute("UPDATE speech_attempts SET status=?2 WHERE id=?1 AND status IN ('preparing','recording','processing')",params![attempt_id,status]).map_err(db_error)?;
                tx.execute("UPDATE speech_records SET status=?2,revision=revision+1 WHERE id=(SELECT record_id FROM speech_attempts WHERE id=?1 AND kind='live') AND status IN ('preparing','recording','processing') AND status!=?2",params![attempt_id,status]).map_err(db_error)?;
                tx.commit().map_err(db_error)?;Ok(DbOutput::Done)
            },
            DbOperation::Preferences=>Ok(DbOutput::Preferences(self.connection.query_row("SELECT device_id,history_enabled,audio_enabled,retention_days,revision FROM speech_preferences WHERE singleton=1",[],|r|Ok(Preferences{device_id:r.get(0)?,history_enabled:r.get(1)?,audio_enabled:r.get(2)?,retention_days:r.get(3)?,revision:r.get(4)?})).map_err(db_error)?)),
            DbOperation::SavePreferences(p)=>{p.validate()?;let n=self.connection.execute("UPDATE speech_preferences SET device_id=?1,history_enabled=?2,audio_enabled=?3,retention_days=?4,revision=revision+1 WHERE singleton=1 AND revision=?5",params![p.device_id,p.history_enabled,p.audio_enabled,p.retention_days,p.revision]).map_err(db_error)?;if n!=1{return Err(Error::new("conflict"));}self.history_operation(DbOperation::Preferences)},
            DbOperation::Detail(id)=>Ok(DbOutput::Record(detail(&self.connection,&id)?)),
            DbOperation::List(q)=>{
                if q.search.chars().count()>256 || q.search.chars().any(char::is_control){return Err(Error::new("invalid"));}
                let tx=self.connection.transaction().map_err(db_error)?;
                let search=crate::data::model::key(&q.search);
                let filter="(?1='' OR EXISTS(SELECT 1 FROM speech_attempts a WHERE a.record_id=r.id AND instr(a.search_key,?1)>0))";
                let total:u32=tx.query_row(&format!("SELECT count(*) FROM speech_records r WHERE {filter}"),[&search],|r|r.get(0)).map_err(db_error)?;
                let page=q.page.max(1).min(total.div_ceil(50).max(1));
                let ids=tx.prepare(&format!("SELECT r.id FROM speech_records r WHERE {filter} ORDER BY r.created_at DESC,r.id ASC LIMIT 50 OFFSET ?2")).map_err(db_error)?.query_map(params![search,(page-1)*50],|r|r.get::<_,String>(0)).map_err(db_error)?.collect::<Result<Vec<_>,_>>().map_err(db_error)?;
                let records=ids.into_iter().map(|id|detail(&tx,&id)).collect::<Result<Vec<_>,_>>()?;
                tx.commit().map_err(db_error)?;Ok(DbOutput::Page(Page{records,total,page,page_size:50}))
            },
            DbOperation::Usage=>{
                let mut out=Usage::default();
                let mut stmt=self.connection.prepare("SELECT r.duration_ms,a.text FROM speech_records r JOIN speech_attempts a ON a.id=r.current_attempt WHERE a.status='success' AND r.status!='delete_pending'").map_err(db_error)?;
                for row in stmt.query_map([],|r|Ok((r.get::<_,i64>(0)? as u64,r.get::<_,String>(1)?))).map_err(db_error)?{let (duration,text)=row.map_err(db_error)?;out.count+=1;out.duration_ms+=duration;out.characters+=text.chars().filter(|c|!c.is_whitespace()).count() as u64;}
                Ok(DbOutput::Usage(out))
            },
            DbOperation::Prepare(p)=>{
                let tx=self.connection.transaction().map_err(db_error)?;
                tx.execute("INSERT INTO speech_records(id,created_at,status) VALUES(?1,?2,'preparing')",params![p.record_id,p.attempt.created_at]).map_err(db_error)?;
                insert_attempt(&tx,&p.attempt)?;
                if let Some(a)=p.audio {tx.execute("INSERT INTO audio_assets(id,record_id,file_key,status,expires_at) VALUES(?1,?2,?3,'prepared',?4)",params![a.id,a.record_id,a.file_key,a.expires_at]).map_err(db_error)?;}
                tx.commit().map_err(db_error)?;Ok(DbOutput::Done)
            },
            DbOperation::Retry(a)=>{let tx=self.connection.transaction().map_err(db_error)?;let r=detail(&tx,&a.record_id)?;if r.status=="delete_pending"{return Err(Error::new("busy"));}insert_attempt(&tx,&a)?;tx.commit().map_err(db_error)?;Ok(DbOutput::Done)},
            DbOperation::Checkpoint{attempt_id,text}=>{if text.len()>1024*1024{return Err(Error::new("invalid"));}self.connection.execute("UPDATE speech_attempts SET text=?2,search_key=?3 WHERE id=?1 AND status IN ('preparing','recording','processing')",params![attempt_id,text,crate::data::model::key(&text)]).map_err(db_error)?;Ok(DbOutput::Done)},
            DbOperation::Finish{attempt_id,status,text,error,duration_ms,elapsed_ms}=>{
                if !["success","failed","cancelled","interrupted"].contains(&status.as_str()) || text.len()>1024*1024{return Err(Error::new("invalid"));}
                let tx=self.connection.transaction().map_err(db_error)?;
                let (record_id,kind):(String,String)=tx.query_row("SELECT record_id,kind FROM speech_attempts WHERE id=?1",[&attempt_id],|r|Ok((r.get(0)?,r.get(1)?))).map_err(db_error)?;
                let n=tx.execute("UPDATE speech_attempts SET status=?2,text=?3,error_code=?4,elapsed_ms=?5,ended_at=?6,search_key=?7 WHERE id=?1 AND status IN ('preparing','recording','processing')",params![attempt_id,status,text,error,elapsed_ms as i64,now(),crate::data::model::key(&text)]).map_err(db_error)?;
                if n!=1{return Err(Error::new("conflict"));}
                if kind=="live" {tx.execute("UPDATE speech_records SET status=?2,ended_at=?3,duration_ms=?4,current_attempt=?5,revision=revision+1 WHERE id=?1",params![record_id,status,now(),duration_ms as i64,attempt_id]).map_err(db_error)?;}
                else if status=="success"{tx.execute("UPDATE speech_records SET current_attempt=?2,status='success',revision=revision+1 WHERE id=?1",params![record_id,attempt_id]).map_err(db_error)?;}
                tx.commit().map_err(db_error)?;Ok(DbOutput::Done)
            },
            DbOperation::AudioState{id,status,bytes}=>{self.connection.execute("UPDATE audio_assets SET status=?2,bytes=?3 WHERE id=?1",params![id,status,bytes as i64]).map_err(db_error)?;Ok(DbOutput::Done)},
            DbOperation::Audio(id)=>Ok(DbOutput::Audio(self.connection.query_row("SELECT id,record_id,file_key,status,bytes,expires_at FROM audio_assets WHERE id=?1",[id],audio).optional().map_err(db_error)?.ok_or(Error::new("notFound"))?)),
            DbOperation::Pending=>Ok(DbOutput::Assets(assets(&self.connection,"status IN ('prepared','delete_pending','expire_pending') AND ?1>=0",0)?)),
            DbOperation::Expired(at)=>Ok(DbOutput::Assets(assets(&self.connection,"(status='ready' AND expires_at IS NOT NULL AND expires_at<=?1) OR status='expire_pending'",at)?)),
            DbOperation::MarkDelete{id,revision}=>{let tx=self.connection.transaction().map_err(db_error)?;let n=tx.execute("UPDATE speech_records SET status='delete_pending',revision=revision+1 WHERE id=?1 AND revision=?2",params![id,revision]).map_err(db_error)?;if n!=1{return Err(Error::new("conflict"));}tx.execute("UPDATE audio_assets SET status='delete_pending' WHERE record_id=?1",[id]).map_err(db_error)?;tx.commit().map_err(db_error)?;Ok(DbOutput::Done)},
            DbOperation::DeleteRecord(id)=>{self.connection.execute("DELETE FROM speech_records WHERE id=?1 AND status='delete_pending'",[id]).map_err(db_error)?;Ok(DbOutput::Done)},
            DbOperation::Interrupted=>{
                let tx=self.connection.transaction().map_err(db_error)?;
                tx.execute("UPDATE speech_records SET current_attempt=(SELECT id FROM speech_attempts WHERE record_id=speech_records.id ORDER BY created_at DESC,id LIMIT 1),status='interrupted',ended_at=?1,revision=revision+1 WHERE status IN ('preparing','recording','processing')",[now()]).map_err(db_error)?;
                tx.execute("UPDATE speech_attempts SET status='interrupted',error_code='interrupted',ended_at=?1 WHERE status IN ('preparing','recording','processing')",[now()]).map_err(db_error)?;
                tx.execute("DELETE FROM speech_records WHERE status='delete_pending' AND NOT EXISTS(SELECT 1 FROM audio_assets WHERE record_id=speech_records.id)",[]).map_err(db_error)?;
                tx.commit().map_err(db_error)?;Ok(DbOutput::Done)
            }
        }
    }
}
