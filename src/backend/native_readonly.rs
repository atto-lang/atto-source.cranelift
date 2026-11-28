//! Trusted closed source only: root-owned object storage, internal
//! pointer SSA parameters and non-escaping scalar returns.
use super::{Compiled,jit_module,validate_ir};
use crate::frontend::native_readonly::{Plan,Read,Op};
use cranelift_codegen::ir::{AbiParam,InstBuilder,MemFlagsData,StackSlotData,StackSlotKind,types};
use cranelift_codegen::settings::{self,Configurable};
use cranelift_frontend::{FunctionBuilder,FunctionBuilderContext};
use cranelift_module::{Module,Linkage,FuncId,default_libcall_names};
use cranelift_object::{ObjectBuilder,ObjectModule};

fn lower(plan:&Plan,module:&mut impl Module,linkage:Linkage)->Result<(FuncId,String),String>{
    let ptr=module.target_config().pointer_type();
    let mut helper_sig=module.make_signature();
    helper_sig.params.push(AbiParam::new(ptr));
    helper_sig.returns.push(AbiParam::new(types::I64));
    let mut ids=Vec::new();
    let mut ir=String::new();
    for (i,read) in plan.helpers().iter().enumerate(){
        let id=module.declare_function(&format!("atto_ro_get_{i}"),Linkage::Local,&helper_sig).map_err(|e|e.to_string())?;
        let mut ctx=module.make_context();ctx.func.signature=helper_sig.clone();
        let target=match *read{
            Read::Field(_)=>None,
            Read::Forward(n)=>Some(module.declare_func_in_func(ids[n],&mut ctx.func)),
        };
        let mut fb=FunctionBuilderContext::new();
        let mut b=FunctionBuilder::new(&mut ctx.func,&mut fb);
        let entry=b.create_block();b.append_block_params_for_function_params(entry);b.switch_to_block(entry);
        let incoming=b.block_params(entry)[0];
        let value=match *read{
            Read::Field(n)=>b.ins().load(types::I64,MemFlagsData::new(),incoming,(n*8) as i32),
            Read::Forward(_)=>{
                let call=b.ins().call(target.expect("verified preceding helper"),&[incoming]);
                b.inst_results(call)[0]
            }
        };
        b.ins().return_(&[value]);b.seal_all_blocks();b.finalize(module.target_config());
        validate_ir(&ctx.func,module.isa())?;
        ir.push_str(&ctx.func.display().to_string());ir.push('\n');
        module.define_function(id,&mut ctx).map_err(|e|e.to_string())?;
        ids.push(id);
    }
    // Existing stable scalar entry ABI; zero source arguments. Helpers are
    // local symbols only, so no foreign caller can forge a class pointer.
    let mut sig=module.make_signature();
    for _ in 0..4{sig.params.push(AbiParam::new(types::I64));}
    sig.params.push(AbiParam::new(ptr));sig.returns.push(AbiParam::new(types::I32));
    let id=module.declare_function("oreslang_scalar_entry",linkage,&sig).map_err(|e|e.to_string())?;
    let mut ctx=module.make_context();ctx.func.signature=sig;
    let targets:Vec<_>=ids.iter().map(|id|module.declare_func_in_func(*id,&mut ctx.func)).collect();
    let mut fb=FunctionBuilderContext::new();let mut b=FunctionBuilder::new(&mut ctx.func,&mut fb);
    let entry=b.create_block();let failure=b.create_block();
    b.append_block_params_for_function_params(entry);b.switch_to_block(entry);
    let output=b.block_params(entry)[4];
    let object=b.create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot,(plan.fields().len()*8) as u32,3));
    for (i,v) in plan.fields().iter().enumerate(){
        let value=b.ins().iconst(types::I64,*v);b.ins().stack_store(ptr,value,object,(i*8) as i32);
    }
    let owner=b.ins().stack_addr(ptr,object,0);
    let first=b.ins().call(targets[plan.calls()[0]],&[owner]);
    let mut value=b.inst_results(first)[0];
    for (&call,op) in plan.calls()[1..].iter().zip(plan.ops()){
        let inst=b.ins().call(targets[call],&[owner]);let rhs=b.inst_results(inst)[0];
        let (result,bad)=match op{Op::Add=>b.ins().sadd_overflow(value,rhs),Op::Sub=>b.ins().ssub_overflow(value,rhs)};
        let next=b.create_block();b.ins().brif(bad,failure,&[],next,&[]);b.switch_to_block(next);value=result;
    }
    b.ins().store(MemFlagsData::new(),value,output,0);
    let success=b.ins().iconst(types::I32,0);b.ins().return_(&[success]);
    b.switch_to_block(failure);let failed=b.ins().iconst(types::I32,1);b.ins().return_(&[failed]);
    b.seal_all_blocks();b.finalize(module.target_config());validate_ir(&ctx.func,module.isa())?;
    ir.push_str(&ctx.func.display().to_string());
    module.define_function(id,&mut ctx).map_err(|e|e.to_string())?;
    Ok((id,ir))
}
pub(crate) fn compile(plan:&Plan)->Result<Compiled,String>{
    let mut module=jit_module()?;let (id,ir)=lower(plan,&mut module,Linkage::Local)?;
    module.finalize_definitions().map_err(|e|e.to_string())?;
    #[cfg(not(test))] let _=ir;
    Ok(Compiled{module,id,arity:0,#[cfg(test)] clif:ir})
}
pub(crate) fn object(plan:&Plan)->Result<Vec<u8>,String>{
    let mut flags=settings::builder();flags.set("is_pic","true").map_err(|e|e.to_string())?;
    let isa=cranelift_native::builder().map_err(|e|e.to_string())?.finish(settings::Flags::new(flags)).map_err(|e|e.to_string())?;
    let builder=ObjectBuilder::new(isa,"atto_readonly",default_libcall_names()).map_err(|e|e.to_string())?;
    let mut module=ObjectModule::new(builder);lower(plan,&mut module,Linkage::Export)?;
    module.finish().emit().map_err(|e|e.to_string())
}
#[cfg(test)]
mod tests{
    use super::*;
    use crate::frontend::native_readonly::parse;
    const S:&str="// atto-ownership: borrow-default-v1\ndefine class Parcel as pub let int padding = 99; pub let int value = 21; end\nfnc get(Parcel p): int { return p.value; }\nfnc forward(Parcel p): int { return get(p); }\npub fnc main(): int { let p = new Parcel(); return forward(p) + get(p); }";
    #[test] fn readonly_class_native_jit_and_object(){
        let plan=parse(S).unwrap();let compiled=compile(&plan).unwrap();
        assert_eq!(compiled.run(&[]).unwrap(),42);
        assert_eq!(compiled.clif.matches("explicit_slot").count(),1);
        assert!(compiled.clif.contains("load.i64"));
        assert!(compiled.clif.contains("stack_addr"));
        assert!(!object(&plan).unwrap().is_empty());
        let overflow=S.replace("value = 21","value = 9223372036854775807");
        assert!(compile(&parse(&overflow).unwrap()).unwrap().run(&[]).is_err());
        assert_eq!(compiled.run(&[]).unwrap(),42);
    }
    #[test] fn readonly_class_rejects_escape_and_unproved_shapes(){
        for bad in [S.replace("return get(p);","return forward(p);"),S.replace("return get(p);","return get(new Parcel());"),
            S.replace("return p.value;","return p.missing;"),S.replace("return forward(p) + get(p);","return forward(p) * get(p);"),
            S.replace("fnc get(Parcel p)","fnc get(mut Parcel p)"),format!("{S} garbage")]{
            assert!(parse(&bad).is_err(),"{bad}");
        }
    }
}
