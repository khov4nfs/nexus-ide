'use strict';
const {test}=require('node:test');
const assert=require('node:assert/strict');
const {plan,offset,checkedPath}=require('./workbench-lsp-edits');
const root='/root/nexus-workbench',a='file://'+root+'/a.ts',b='file://'+root+'/b.ts';
const baselines=new Map([[a,{text:'export const old = 1;\n',wireVersion:7}],[b,{text:'import {old} from "./a";\nold;\n',wireVersion:8}]]);
const e=(line,start,end,newText)=>({range:{start:{line,character:start},end:{line,character:end}},newText});
test('cross-file rename plans all files without mutation',()=>{
    const out=plan({documentChanges:[{textDocument:{uri:a,version:7},edits:[e(0,13,16,'next')]},{textDocument:{uri:b,version:8},edits:[e(0,8,11,'next'),e(1,0,3,'next')]}]},baselines,root);
    assert.equal(out[0].replacement,'export const next = 1;\n');
    assert.equal(out[1].replacement,'import {next} from "./a";\nnext;\n');
    assert.equal(baselines.get(a).text,'export const old = 1;\n');
});
test('stale, overlapping, resource and external edits rejected',()=>{
    for(const bad of [
        {documentChanges:[{textDocument:{uri:a,version:6},edits:[e(0,13,16,'new')]}]},
        {changes:{[a]:[e(0,0,2,'x'),e(0,1,3,'y')]}},
        {changes:{[a]:[e(0,0,0,'x'),e(0,0,0,'y')]}},
        {documentChanges:[{kind:'rename',oldUri:a,newUri:b}]},
        {changes:{'file:///etc/passwd':[e(0,0,0,'x')]}},
        {changes:{[a]:[{...e(0,0,0,'x'),annotationId:'unreviewed'}]}},
        {changes:{[a]:[e(0,0,0,'\0')]}},
        {changes:{[a]:[e(2,0,0,'x')]}},
        {changes:{[a]:[e(0,0,0,'x')]},documentChanges:[]},
        {documentChanges:[{textDocument:{uri:a,version:7},edits:[e(0,0,0,'x')]},{textDocument:{uri:a,version:7},edits:[e(0,1,1,'y')]}]}
    ])assert.throws(()=>plan(bad,baselines,root));
    for(const uri of ['file:///root/nexus-workbench/.git/config','file:///root/nexus-workbench/node_modules/x.ts','file:///root/nexus-workbench/../other','file:///root/nexus-workbench/abs%2Fescape.ts'])assert.throws(()=>checkedPath(uri,root));
});
test('UTF-16 and CRLF boundaries',()=>{
    assert.equal(offset('😀x\r\ny',{line:0,character:2}),2);
    assert.equal(offset('😀x\r\ny',{line:1,character:1}),6);
    assert.throws(()=>offset('😀x\r\ny',{line:0,character:1}));
    assert.throws(()=>offset('😀x\r\ny',{line:0,character:4}));
});
