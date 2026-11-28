'use strict';
// Text-only WorkspaceEdit planner. No resource operations, commands, or
// writes. Baselines must be captured before asking a server for an edit.
const path=require('path');
const {fileURLToPath,pathToFileURL}=require('url');
function checkedPath(uri,root){
    const file=fileURLToPath(uri);
    if(pathToFileURL(file).href!==uri||!file.startsWith(root+'/')||path.resolve(file)!==file)throw Error('outside workspace');
    const parts=file.slice(root.length+1).split('/');
    if(parts.some(x=>!x||x.startsWith('.')||['node_modules','target','vendor','build','dist'].includes(x)))throw Error('excluded project path');
    return file;
}
function offset(text,p){
    if(!p||!Number.isSafeInteger(p.line)||!Number.isSafeInteger(p.character)||p.line<0||p.character<0)throw Error('invalid position');
    let start=0;
    for(let i=0;i<p.line;i++){
        const n=text.indexOf('\n',start);if(n<0)throw Error('invalid line');start=n+1;
    }
    let end=text.indexOf('\n',start);if(end<0)end=text.length;
    if(end>start&&text.charCodeAt(end-1)===13)end--;
    const at=start+p.character;
    if(at>end||(at>0&&at<text.length&&text.charCodeAt(at-1)>=0xd800&&text.charCodeAt(at-1)<=0xdbff&&text.charCodeAt(at)>=0xdc00&&text.charCodeAt(at)<=0xdfff))throw Error('invalid UTF-16 position');
    return at;
}
function plan(edit,baselines,root){
    if(!edit||typeof edit!=='object'||Array.isArray(edit)||Object.keys(edit).some(k=>!['changes','documentChanges'].includes(k))||
        (edit.changes!==undefined)===(edit.documentChanges!==undefined))throw Error('unsupported workspace edit');
    let groups;
    if(edit.changes!==undefined){
        if(!edit.changes||typeof edit.changes!=='object'||Array.isArray(edit.changes))throw Error('invalid changes');
        groups=Object.entries(edit.changes).map(([uri,edits])=>({textDocument:{uri,version:null},edits}));
    }else groups=edit.documentChanges;
    if(!Array.isArray(groups)||groups.length<1||groups.length>32)throw Error('file limit');
    let total=0,bytes=0;const seen=new Set(),result=[];
    for(const group of groups){
        if(!group||Object.keys(group).some(k=>!['textDocument','edits'].includes(k)))throw Error('resource operation forbidden');
        const {uri,version}=group.textDocument||{};
        checkedPath(uri,root);
        const base=baselines.get(uri);
        if(!base||seen.has(uri)||(version!=null&&version!==base.wireVersion))throw Error('unknown or stale document');
        seen.add(uri);
        if(!Array.isArray(group.edits)||!group.edits.length||(total+=group.edits.length)>512)throw Error('edit limit');
        const rows=group.edits.map(e=>{
            if(!e||Object.keys(e).some(k=>!['range','newText'].includes(k))||typeof e.newText!=='string'||e.newText.includes('\0')||(bytes+=Buffer.byteLength(e.newText))>262144)throw Error('invalid text edit');
            const a=offset(base.text,e.range?.start),b=offset(base.text,e.range?.end);
            if(a>b)throw Error('inverted range');
            return {a,b,text:e.newText,range:e.range};
        }).sort((a,b)=>a.a-b.a||a.b-b.b);
        for(let i=1;i<rows.length;i++)if(rows[i-1].b>rows[i].a||rows[i-1].a===rows[i].a)throw Error('overlapping edits');
        let replacement=base.text;
        for(const e of [...rows].reverse())replacement=replacement.slice(0,e.a)+e.text+replacement.slice(e.b);
        if(Buffer.byteLength(replacement)>131072)throw Error('replacement limit');
        result.push({uri,base,rows,replacement});
    }
    return result;
}
module.exports={checkedPath,offset,plan};
