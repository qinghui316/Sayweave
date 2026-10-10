import {describe,it,expect} from 'vitest';
// @ts-expect-error JavaScript acceptance module has no declaration file.
import {temporaryPreferenceCleanup} from '../scripts/windows-update-registration.mjs';
const component='D:\\fixture\\production.dll',leaf='\\LanguageProfile\\0x00000409\\{6CD26440-E770-471E-8212-B5A565E816F5}';
type PreferenceEntry={path:string;values:Record<string,{kind:string;value:number}>};
function fixture(){
 const empty=()=>({com:false,tip:false,module:null,owner:null,tipEntries:[] as PreferenceEntry[]});
 const user=()=>({...empty(),tip:true,tipEntries:[{path:'',values:{}},{path:'\\LanguageProfile',values:{}},{path:'\\LanguageProfile\\0x00000409',values:{}},{path:leaf,values:{Enable:{kind:'DWord',value:1}}}] as PreferenceEntry[]});
 const owned=()=>({HKEY_CURRENT_USER:{...user(),com:true,module:`\\\\?\\${component}`,owner:'SayweaveStageA-v1'},HKEY_LOCAL_MACHINE:{...empty(),tip:true}});
 return {initial:{HKEY_CURRENT_USER:empty(),HKEY_LOCAL_MACHINE:empty()},prepared:owned(),anchor:owned(),after:{HKEY_CURRENT_USER:user(),HKEY_LOCAL_MACHINE:empty()},component,metadata:{profiles:0,categories:0,speechCategory:false}};
}
describe('fresh owned TIP preference cleanup permission',()=>{
 it('permits only the exact observed preference and four empty-parent paths',()=>{expect(temporaryPreferenceCleanup(fixture())).toEqual([leaf,'\\LanguageProfile\\0x00000409','\\LanguageProfile','']);});
 it.each(['initial','prepared','anchor','after'] as const)('rejects conflicting %s registration',snapshot=>{const f=fixture();f[snapshot].HKEY_LOCAL_MACHINE.com=true;expect(()=>temporaryPreferenceCleanup(f)).toThrow();});
 it('rejects an absent removal anchor or changed DLL owner/path',()=>{const f=fixture();f.anchor.HKEY_CURRENT_USER.module='D:\\other.dll';expect(()=>temporaryPreferenceCleanup(f)).toThrow('ownership');f.anchor=f.prepared;f.anchor.HKEY_CURRENT_USER.owner='other';expect(()=>temporaryPreferenceCleanup(f)).toThrow('ownership');});
 it.each(['prepared','anchor','after'] as const)('rejects new keys or values in %s',snapshot=>{const f=fixture();f[snapshot].HKEY_CURRENT_USER.tipEntries.push({path:'\\Other',values:{}});expect(()=>temporaryPreferenceCleanup(f)).toThrow('structure');const g=fixture();g[snapshot].HKEY_CURRENT_USER.tipEntries[0]!.values.Other={kind:'DWord',value:1};expect(()=>temporaryPreferenceCleanup(g)).toThrow('values');});
 it.each([{kind:'DWord',value:0},{kind:'String',value:1}])('rejects a changed Enable value/type %j',value=>{const f=fixture();f.after.HKEY_CURRENT_USER.tipEntries[3]!.values.Enable=value;expect(()=>temporaryPreferenceCleanup(f)).toThrow('changed');});
 it('rejects another language/profile or traversal and live metadata',()=>{for(const path of [leaf.replace('00000409','00000804'),leaf.replace('6CD26440','7CD26440'),'\\..\\Other']){const f=fixture();f.after.HKEY_CURRENT_USER.tipEntries[3]!.path=path;expect(()=>temporaryPreferenceCleanup(f)).toThrow();}const f=fixture();f.metadata.profiles=1;expect(()=>temporaryPreferenceCleanup(f)).toThrow('registered');});
 it.each(['prepared','anchor','after'] as const)('rejects missing, null or array values in %s',snapshot=>{for(const values of [undefined,null,[]]){const f=fixture();Object.assign(f[snapshot].HKEY_CURRENT_USER.tipEntries[0]!,{values});expect(()=>temporaryPreferenceCleanup(f)).toThrow('Incomplete');}});
 it('rejects non-boolean registration flags and non-string artifact identities',()=>{for(const snapshot of ['prepared','anchor'] as const){const f=fixture();Object.assign(f[snapshot].HKEY_CURRENT_USER,{com:1});expect(()=>temporaryPreferenceCleanup(f)).toThrow('Incomplete');}for(const value of [null,1,{}]){const f=fixture();Object.assign(f,{component:value});Object.assign(f.anchor.HKEY_CURRENT_USER,{module:value});expect(()=>temporaryPreferenceCleanup(f)).toThrow();}});
 it('rejects missing scope fields and duplicate preference paths',()=>{const f=fixture();Reflect.deleteProperty(f.prepared.HKEY_LOCAL_MACHINE,'tipEntries');expect(()=>temporaryPreferenceCleanup(f)).toThrow('Incomplete');const g=fixture();g.after.HKEY_CURRENT_USER.tipEntries[1]!.path='';expect(()=>temporaryPreferenceCleanup(g)).toThrow('keys');});
});
