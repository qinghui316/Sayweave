import catalogue from './themes.json';
import {parseAppearance} from '../contracts/appearance';
export const themes = catalogue;
export function themeById(id:string){return themes.find(theme=>theme.id===id)??themes[0]!;}
export function startupTheme(desktop:boolean,native:unknown,readBrowser:()=>string|null):string {
 try {
  const value=desktop?native:JSON.parse(readBrowser()??'null');
  return themeById(parseAppearance(value).themeId).id;
 } catch {return 'light';}
}
export function applyTheme(id:string){
 const theme=themeById(id),root=document.documentElement;
 for(const [key,value] of Object.entries(theme.vars))root.style.setProperty(key,value);
 root.dataset.theme=theme.id;root.style.colorScheme=theme.scheme;
 return theme.id;
}
