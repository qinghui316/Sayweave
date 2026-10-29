// A candidate records the whole gesture before any modifier is released.
// Validation and saving remain owned by ShortcutService.
export class ShortcutRecording {
 private keys=new Set<string>();
 private candidate:string|null=null;
 accept(event:Pick<KeyboardEvent,'type'|'code'|'repeat'|'ctrlKey'|'altKey'|'shiftKey'|'metaKey'>):string|null {
  if(event.repeat)return null;
  if(event.type==='keyup')return this.keys.has(event.code)?this.candidate:null;
  this.keys.add(event.code);
  const modifier=/^(Alt|Control|Shift|Meta)(Left|Right)$/.test(event.code);
  const pair=modifier&&this.keys.size===2?(this.keys.has('AltRight')&&this.keys.has('ShiftRight')?'AltRight+ShiftRight':this.keys.has('AltLeft')&&this.keys.has('ShiftLeft')?'AltLeft+ShiftLeft':null):null;
  this.candidate=pair??[...(modifier?[]:[event.ctrlKey?'Control':'',event.altKey?'Alt':'',event.shiftKey?'Shift':'',event.metaKey?'Super':''].filter(Boolean)),event.code].join('+');
  return null;
 }
}
