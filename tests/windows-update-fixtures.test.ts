import {DatabaseSync} from 'node:sqlite';
import {describe,expect,it} from 'vitest';
// @ts-expect-error Fixture JavaScript has no declaration file.
import {databaseInventory,seedLegacyRows,legacySeedRows} from '../scripts/windows-update-fixtures.mjs';

function fixture(){
 const old=new DatabaseSync(':memory:'),seed=new DatabaseSync(':memory:');
 const sql=`CREATE TABLE schema_migrations(version INTEGER,checksum TEXT);
 CREATE TABLE model_configs(id TEXT PRIMARY KEY,credential_ref TEXT);
 CREATE TABLE model_defaults(config_id TEXT);
 CREATE TABLE dictionary_terms(id TEXT,term TEXT);
 CREATE TABLE correction_rules(id TEXT);
 CREATE TABLE language_term_sources(term_id TEXT);
 CREATE TABLE automatic_term_settings(enabled INTEGER);
 CREATE TABLE language_settings(enabled INTEGER);
 CREATE TABLE speech_preferences(device_id TEXT);
 CREATE TABLE processing_settings(enabled INTEGER);
 CREATE TABLE shortcut_settings(binding TEXT);
 CREATE TABLE speech_records(id TEXT PRIMARY KEY);
 CREATE TABLE speech_attempts(id TEXT,record_id TEXT REFERENCES speech_records(id),config_id TEXT);
 CREATE TABLE delivery_attempts(id TEXT);
 CREATE TABLE language_observations(id TEXT);
 CREATE TABLE language_profiles(scope TEXT);
 CREATE TABLE language_profile_revisions(scope TEXT,analysis TEXT);`;
 old.exec(sql);seed.exec(sql);
 old.exec("INSERT INTO schema_migrations VALUES(9,'immutable');INSERT INTO model_configs VALUES('old-model','old-reference');INSERT INTO model_defaults VALUES('old-model');INSERT INTO dictionary_terms VALUES('old-term','原有术语');");
 seed.exec(`INSERT INTO speech_records VALUES('synthetic-record');INSERT INTO speech_attempts VALUES('synthetic-attempt','synthetic-record','fixture-model');INSERT INTO language_profiles VALUES('app:fixture');INSERT INTO language_profile_revisions VALUES('app:fixture','{"configId":"fixture-model"}');`);
 return {old,seed};
}
describe('old-install synthetic data fixture',()=>{
 it('accepts serialized same-candidate synthetic rows and retains native identities',()=>{
  const {old,seed}=fixture();try{
   const before=databaseInventory(old,['schema_migrations','model_configs','model_defaults','dictionary_terms']);
   seedLegacyRows(old,JSON.parse(JSON.stringify(legacySeedRows(seed))),'old-model');
   expect(databaseInventory(old,Object.keys(before))).toEqual(before);
   expect(old.prepare('SELECT config_id FROM speech_attempts').get()).toEqual({config_id:'old-model'});
  }finally{old.close();seed.close();}
 });
 it('rejects extra tables, excessive rows and malformed serialized schemas without mutation',()=>{
  const {old,seed}=fixture();try{
   const before=databaseInventory(old),rows=legacySeedRows(seed);
   expect(()=>seedLegacyRows(old,{...rows,model_configs:[]},'old-model')).toThrow('inventory');
   expect(()=>seedLegacyRows(old,{...rows,speech_records:Array(21).fill({id:'bad'})},'old-model')).toThrow('bound');
   expect(()=>seedLegacyRows(old,{...rows,language_profiles:[{wrong:'bad'}]},'old-model')).toThrow('schema');
   expect(databaseInventory(old)).toEqual(before);
  }finally{old.close();seed.close();}
 });
 it('keeps the old ledger, native model/default/credential and dictionary while appending source-bound rows',()=>{
  const {old,seed}=fixture();try{
   const before=databaseInventory(old,['schema_migrations','model_configs','model_defaults','dictionary_terms']);
   seedLegacyRows(old,seed,'old-model');
   expect(databaseInventory(old,Object.keys(before))).toEqual(before);
   expect(old.prepare('SELECT config_id FROM speech_attempts').get()).toEqual({config_id:'old-model'});
   expect(JSON.parse(old.prepare('SELECT analysis FROM language_profile_revisions').get()!.analysis as string)).toEqual({configId:'old-model'});
  }finally{old.close();seed.close();}
 });
 it('rolls back earlier inserts on a legacy schema mismatch',()=>{
  const {old,seed}=fixture();try{
   seed.exec('ALTER TABLE language_profiles ADD COLUMN incompatible TEXT');
   const before=databaseInventory(old);
   expect(()=>seedLegacyRows(old,seed,'old-model')).toThrow('Legacy schema mismatch');
   expect(databaseInventory(old)).toEqual(before);
  }finally{old.close();seed.close();}
 });
 it('rejects broken source foreign keys without changing the old data',()=>{
  const {old,seed}=fixture();try{
   seed.exec("PRAGMA foreign_keys=OFF;UPDATE speech_attempts SET record_id='missing'");
   const before=databaseInventory(old);
   expect(()=>seedLegacyRows(old,seed,'old-model')).toThrow();
   expect(databaseInventory(old)).toEqual(before);
  }finally{old.close();seed.close();}
 });
});
