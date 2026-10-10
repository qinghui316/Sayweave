import {createHash} from 'node:crypto';

const seedTables=['speech_records','speech_attempts','delivery_attempts','language_observations','language_profiles','language_profile_revisions'];
export function legacySeedRows(db){
 return Object.fromEntries(seedTables.map(table=>{
  const rows=db.prepare(`SELECT * FROM ${table} ORDER BY rowid LIMIT 21`).all();
  if(rows.length>20)throw Error('Legacy seed exceeds bound');
  return [table,rows];
 }));
}
export const inventoryTables=['schema_migrations','model_configs','model_defaults','dictionary_terms','correction_rules','language_term_sources','automatic_term_settings','language_settings','speech_preferences','processing_settings','shortcut_settings',...seedTables];
export function databaseInventory(db,tables=inventoryTables){
 return Object.fromEntries(tables.map(table=>{
  if(!inventoryTables.includes(table))throw Error('Unsupported inventory table');
  const rows=db.prepare(`SELECT * FROM ${table} ORDER BY rowid`).all();
  return [table,{count:rows.length,sha256:createHash('sha256').update(JSON.stringify(rows)).digest('hex')}];
 }));
}
// Append bounded synthetic rows to the actual old database; never replace its file,
// migrations, settings, native-created model/default/credential references or terms.
export function seedLegacyRows(db,seed,configId){
 const source=typeof seed.prepare==='function'?legacySeedRows(seed):seed;
 if(Object.keys(source??{}).sort().join()!==[...seedTables].sort().join())throw Error('Legacy seed inventory differs');
 if(!db.prepare('SELECT id FROM model_configs WHERE id=?').get(configId))throw Error('Legacy model is missing');
 const preserved=databaseInventory(db,['schema_migrations','model_configs','model_defaults','dictionary_terms','correction_rules','language_term_sources']);
 db.exec('PRAGMA foreign_keys=ON; BEGIN IMMEDIATE');
 try{
  for(const table of seedTables){
   const columns=db.prepare(`PRAGMA table_info(${table})`).all().map(column=>column.name);
   const rows=source[table];
   if(!Array.isArray(rows)||rows.length>20)throw Error('Legacy seed exceeds bound');
   for(const original of rows){
    const row={...original},keys=Object.keys(row);
    if(keys.length!==columns.length||keys.some(key=>!columns.includes(key)))throw Error(`Legacy schema mismatch: ${table}`);
    if(table==='speech_attempts')row.config_id=configId;
    if(table==='language_profile_revisions')row.analysis=JSON.stringify({...JSON.parse(row.analysis),configId});
    db.prepare(`INSERT INTO ${table}(${columns.join(',')}) VALUES(${columns.map(()=>'?').join(',')})`).run(...columns.map(column=>row[column]));
   }
  }
  if(db.prepare('PRAGMA foreign_key_check').all().length)throw Error('Legacy seed foreign key violation');
  if(JSON.stringify(databaseInventory(db,Object.keys(preserved)))!==JSON.stringify(preserved))throw Error('Legacy identity changed');
  db.exec('COMMIT');
 }catch(error){db.exec('ROLLBACK');throw error;}
 return databaseInventory(db);
}
