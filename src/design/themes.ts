import catalogue from './themes.json';
export const themes = catalogue;
export function themeById(id:string){return themes.find(theme=>theme.id===id)??themes[0]!;}
export function applyTheme(id:string){
 const theme=themeById(id),root=document.documentElement;
 for(const [key,value] of Object.entries(theme.vars))root.style.setProperty(key,value);
 root.dataset.theme=theme.id;root.style.colorScheme=theme.scheme;
 return theme.id;
}
