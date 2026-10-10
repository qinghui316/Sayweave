// Pure permission policy for cleaning one fresh disposable-test TIP preference.
// No registry operations belong here; unknown or pre-existing state is rejected.
const scopes=['HKEY_CURRENT_USER','HKEY_LOCAL_MACHINE'];
const profile='{6CD26440-E770-471E-8212-B5A565E816F5}';
const moduleKey=value=>typeof value==='string'?value.replace(/^\\\\\?\\/,'').toLowerCase():null;
const empty=value=>value?.com===false&&value.tip===false&&value.module===null&&value.owner===null&&Array.isArray(value.tipEntries)&&value.tipEntries.length===0;
function preference(entries){
 if(!Array.isArray(entries)||entries.length!==4)throw Error('Unknown TIP preference structure');
 const leaf=entries.find(item=>new RegExp(`^\\\\LanguageProfile\\\\0x[0-9a-f]{8}\\\\${profile.replaceAll('{','\\{').replaceAll('}','\\}')}$`,'i').test(item.path));
 if(!leaf||Number.parseInt(leaf.path.split('\\')[2].slice(2),16)>65535)throw Error('Unknown TIP preference language/profile');
 const parent=leaf.path.slice(0,leaf.path.lastIndexOf('\\'));
 const paths=[leaf.path,parent,'\\LanguageProfile',''];
 if(new Set(entries.map(item=>item.path)).size!==4||entries.some(item=>!paths.includes(item.path)))throw Error('Unknown TIP preference keys');
 for(const item of entries){
  const names=Object.keys(item.values??{});
  if(item===leaf){if(names.join()!=='Enable'||item.values.Enable.kind!=='DWord'||item.values.Enable.value!==1)throw Error('TIP preference Enable changed');}
  else if(names.length)throw Error('Unknown TIP preference values');
 }
 return paths;
}
export function temporaryPreferenceCleanup({initial,prepared,anchor,after,component,metadata}){
 if(!initial||scopes.some(scope=>!empty(initial[scope])))throw Error('TIP cleanup requires an absent prestate');
 if(metadata?.profiles!==0||metadata.categories!==0||metadata.speechCategory!==false)throw Error('TIP metadata remains registered');
 for(const snapshot of [prepared,anchor]){
  const user=snapshot?.HKEY_CURRENT_USER,machine=snapshot?.HKEY_LOCAL_MACHINE;
  if(!user?.com||user.owner!=='SayweaveStageA-v1'||moduleKey(user.module)!==moduleKey(component)||!component||machine?.com!==false)throw Error('TIP removal ownership is unavailable');
 }
 if(!empty(after?.HKEY_LOCAL_MACHINE)||after?.HKEY_CURRENT_USER?.com!==false||after.HKEY_CURRENT_USER.module!==null||after.HKEY_CURRENT_USER.owner!==null||after.HKEY_CURRENT_USER.tip!==true)throw Error('TIP cleanup has conflicting registrations');
 const paths=preference(prepared.HKEY_CURRENT_USER.tipEntries);
 for(const snapshot of [anchor,after]){
  const actual=preference(snapshot.HKEY_CURRENT_USER.tipEntries);
  if(actual.join()!==paths.join())throw Error('TIP preference identity changed');
 }
 return paths;
}
