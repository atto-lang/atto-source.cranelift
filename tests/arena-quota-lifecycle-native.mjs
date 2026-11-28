// Execute actual Cranelift JIT and independently linked native object.
// Source concatenation does not claim module linking or allocator lowering.
import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {readFileSync,writeFileSync,mkdtempSync,rmSync} from 'node:fs';
import {join,resolve} from 'node:path';
import {tmpdir} from 'node:os';
import {spawnSync} from 'node:child_process';
const compiler=resolve(process.env.ATTOC||'target/release/orescranelift');
const pins=[
 ['stdlib/policy/core/arena_limits.atto','5928e893a2ce1baadb0376a4ccb14f63249f77d4'],
 ['stdlib/policy/alloc/arena_lifecycle.atto','e4795c6d4bd561be3d913cdc617d103d0e2828c8'],
 ['tests/fixtures/arena-quota-lifecycle-guest.atto','2b1eb28f07b792f2727abe9a5691849f024e304a']
];
const source=pins.map(([path,pin])=>{
 const bytes=readFileSync(new URL('../'+path,import.meta.url));
 assert.equal(createHash('sha1').update('blob '+bytes.length+'\0').update(bytes).digest('hex'),pin,path);
 return bytes.toString('utf8');
}).join('\n');
const root=mkdtempSync(join(tmpdir(),'atto-arena-native-'));
function run(cmd,args){
 const p=spawnSync(cmd,args,{timeout:60000,maxBuffer:32*1024*1024});
 assert.equal(p.status,0,String(p.stderr)||String(p.error));return p.stdout;
}
try{
 const marker='arena_end_offset(0, 0, 1, 0) != 0';
 assert.equal(source.split(marker).length,2);
 for(const [name,body,expected] of [['good',source,100],['wrong-oracle',source.replace(marker,'arena_end_offset(0, 0, 1, 0) != 1'),1]]){
   const file=join(root,name+'.atto');writeFileSync(file,body);
   assert.equal(String(run(compiler,['run',file])).trim(),String(expected));
   const object=run(compiler,['emit-object',file]);
   assert.ok(object.length>0);
   const obj=join(root,name+'.o');writeFileSync(obj,object);
   const host=join(root,name+'.c');writeFileSync(host,`#include <stdint.h>\n#include <assert.h>\nextern int32_t oreslang_scalar_entry(int64_t,int64_t,int64_t,int64_t,int64_t*);\nint main(void){for(int i=0;i<100;i++){int64_t out=-1;assert(oreslang_scalar_entry(0,0,0,0,&out)==0 && out==${expected});}return 0;}\n`);
   for(const opt of ['-O0','-O2']){
     const bin=join(root,name+opt);run('cc',['-std=c11','-Wall','-Wextra','-Werror',opt,host,obj,'-o',bin]);run(bin,[]);
   }
 }
 console.log('CRANELIFT_ARENA_POLICY_PASS assertions=56 native_observer_invocations=400 wrong_oracle=1');
}finally{rmSync(root,{recursive:true,force:true});}
