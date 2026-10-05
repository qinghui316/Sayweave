import {Schema} from 'prosemirror-model';
import {EditorState, TextSelection} from 'prosemirror-state';
import {EditorView} from 'prosemirror-view';

const schema=new Schema({nodes:{doc:{content:'paragraph+'},paragraph:{content:'text*',group:'block',toDOM:()=>['p',0]},text:{group:'inline'}}});
const host=document.createElement('div');host.id='prosemirror';document.body.append(host);
const view=new EditorView(host,{state:EditorState.create({schema})});
declare global {interface Window {fixtureEditor:{set(text:string,start:number,end:number):void;read():string}}}
window.fixtureEditor={
 set(text,start,end){const doc=schema.node('doc',null,[schema.node('paragraph',null,text?schema.text(text):undefined)]);view.updateState(EditorState.create({schema,doc,selection:TextSelection.create(doc,start+1,end+1)}));view.focus();},
 read(){return view.state.doc.textBetween(0,view.state.doc.content.size,'\n');},
};
