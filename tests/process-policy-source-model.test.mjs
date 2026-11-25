// Pure source-model regression only. No native Cranelift guest or process creation.
import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {runInNewContext} from 'node:vm';
const source=readFileSync(new URL('../stdlib/process_policy.atto',import.meta.url),'utf8');
const fixture=readFileSync(new URL('./fixtures/std-process-policy-v1/main.atto',import.meta.url),'utf8');
function translate(input) {
  const clean=input.replace(/\/\/[^\n]*/g,'')
    .replace(/^\s*define pub module process_policy as\s*$/m,'')
    .replace(/^\s*end\s*$/m,'')
    .replace(/^\s*import module process_policy from "\.\.\/\.\.\/\.\.\/stdlib\/process_policy";\s*$/m,'')
    .replace(/\bprocess_policy\./g,'');
  if (/\b(import|require|eval|new|class|actor|while|for)\b/.test(clean)) throw Error('outside pure source model');
  const js=clean.replace(/pub fnc (\w+)\(([^)]*)\): (?:int|bool) \{/g,(_,name,args)=>
    'function '+name+'('+args.split(',').map(x=>x.trim().split(/\s+/).at(-1)).filter(Boolean).join(',')+') {')
    .replace(/\bif ([^\n]+?) then return ([^\n]+?); fi/g,(_,c,v)=>'if ('+c+') { return '+v+'; }')
    .replace(/(?<![\w])\d+(?![\w])/g,x=>x+'n');
  if (/\b(?:pub|fnc|then|fi|define)\b/.test(js)) throw Error('unsupported source-model grammar');
  return js;
}
function load(s=source,guest='') {
  const env=Object.create(null);
  runInNewContext(translate(s)+'\n'+translate(guest),env,{timeout:2000});
  return env;
}
test('25 numbered source fixture cases reach 42 in the pure model',()=>{
  assert.equal((fixture.match(/\bif /g)||[]).length,25);
  assert.equal(load(source,fixture).main(),42n);
});
test('bounded capture and child reservation agree with independent BigInt oracle',()=>{
  const m=load(),max=(1n<<63n)-1n;
  const values=[-1n,0n,1n,2n,4096n,max-1n,max];
  for(const b of values)for(const i of values)for(const c of values){
    assert.equal(m.capture_append_admissible(b,i,c),b>=0n&&i>=0n&&c>=0n&&b+i<=c);
    for(const children of [-1n,0n,1n,2n])
      assert.equal(m.child_reservation_admissible(children,2n,b,i,c),children>=0n&&children<2n&&b>=0n&&i>=0n&&c>=0n&&b+i<=c);
  }
});
test('negative mutation is detected by the source fixture',()=>{
  const broken=source.replace('return incoming <= capacity - buffered;','return true;');
  assert.notEqual(broken,source);
  assert.equal(load(broken,fixture).main(),12n);
});
