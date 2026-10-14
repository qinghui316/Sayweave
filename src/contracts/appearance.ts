export interface Appearance {themeId:string;revision:number}
export interface AppearancePort {read():Promise<Appearance>;save(value:Appearance):Promise<Appearance>;subscribe(listener:(value:Appearance)=>void):Promise<()=>void>}
export function parseAppearance(value:unknown):Appearance {
 if(!value||typeof value!=='object')throw Error('Invalid appearance');
 const v=value as Record<string,unknown>;
 if(typeof v.themeId!=='string'||!v.themeId||v.themeId.length>64||!Number.isSafeInteger(v.revision)||(v.revision as number)<1)throw Error('Invalid appearance');
 return {themeId:v.themeId,revision:v.revision as number};
}
