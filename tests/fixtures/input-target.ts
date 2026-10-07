import {Schema} from 'prosemirror-model';
import {EditorState, TextSelection} from 'prosemirror-state';
import {EditorView} from 'prosemirror-view';

const schema=new Schema({nodes:{doc:{content:'paragraph+'},paragraph:{content:'text*',group:'block',parseDOM:[{tag:'p'}],toDOM:()=>['p',0]},text:{group:'inline'}}});
const host=document.createElement('div');host.id='prosemirror';document.body.append(host);
// The official view stylesheet requires preserved whitespace for native editing.
// This minimal fixture owns only paragraph/text nodes; other application UI is unchanged.
const style=document.createElement('style');style.textContent='.ProseMirror{position:relative;white-space:pre-wrap;white-space:break-spaces;word-wrap:break-word;font-variant-ligatures:none;}';document.head.append(style);
const view=new EditorView(host,{state:EditorState.create({schema})});
declare global {interface Window {fixtureEditor:{set(text:string,start:number,end:number):void;read():string;inspect():unknown}}}
window.fixtureEditor={
 set(text,start,end){const doc=schema.node('doc',null,[schema.node('paragraph',null,text?schema.text(text):undefined)]);view.updateState(EditorState.create({schema,doc,selection:TextSelection.create(doc,start+1,end+1)}));view.focus();},
 read(){return view.state.doc.textBetween(0,view.state.doc.content.size,'\n');},
 inspect(){const selection=getSelection();return {html:view.dom.innerHTML,selection:{from:view.state.selection.from,to:view.state.selection.to},domSelection:selection?{anchor:{parent:selection.anchorNode?.parentElement?.tagName,offset:selection.anchorOffset,text:selection.anchorNode?.textContent},focus:{parent:selection.focusNode?.parentElement?.tagName,offset:selection.focusOffset,text:selection.focusNode?.textContent}}:null,document:view.state.doc.toJSON()};},
};
