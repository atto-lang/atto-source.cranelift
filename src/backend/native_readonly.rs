//! Closed readonly native reference and scalar SSA profile.
//! Internal helper ABI returns value plus status; no failure can be
//! mistaken for a successful value or publish a partial root result.
use super::{Compiled,jit_module,validate_ir};
use crate::frontend::native_readonly::{Plan,Body,Expr,Arg,Kind,Op};
use cranelift_codegen::ir::{AbiParam,InstBuilder,MemFlagsData,StackSlotData,StackSlotKind,types,Value,Block,FuncRef,condcodes::IntCC};
use cranelift_codegen::settings::{self,Configurable};
use cranelift_frontend::{FunctionBuilder,FunctionBuilderContext};
use cranelift_module::{Module,Linkage,FuncId,default_libcall_names};
use cranelift_object::{ObjectBuilder,ObjectModule};

fn require_ok(b:&mut FunctionBuilder<'_>,bad:Value,failure:Block){
    let next=b.create_block();
    b.ins().brif(bad,failure,&[],next,&[]);
    b.switch_to_block(next);
}
fn expr(b:&mut FunctionBuilder<'_>,e:&Expr,values:&[Value],targets:&[FuncRef],failure:Block)->Value{
    match e{
        Expr::Literal(n)=>b.ins().iconst(types::I64,*n),
        Expr::Local(n)=>values[*n],
        Expr::Field(n,offset)=>b.ins().load(types::I64,MemFlagsData::new(),values[*n],(*offset*8) as i32),
        Expr::Call(i,args)=>{
            let mut actual=Vec::new();
            for arg in args{
                actual.push(match arg{Arg::Ref(n)=>values[*n],Arg::Int(e)=>expr(b,e,values,targets,failure)});
            }
            let call=b.ins().call(targets[*i],&actual);
            let value=b.inst_results(call)[0];
            let status=b.inst_results(call)[1];
            let bad=b.ins().icmp_imm_s(IntCC::NotEqual,status,0);
            require_ok(b,bad,failure);
            value
        }
        Expr::Bin(op,left,right)=>{
            let a=expr(b,left,values,targets,failure);
            let c=expr(b,right,values,targets,failure);
            let (value,bad)=match op{Op::Add=>b.ins().sadd_overflow(a,c),Op::Sub=>b.ins().ssub_overflow(a,c)};
            require_ok(b,bad,failure);
            value
        }
    }
}
fn body(b:&mut FunctionBuilder<'_>,body:&Body,mut values:Vec<Value>,targets:&[FuncRef],failure:Block)->Value{
    for init in &body.locals{
        let value=expr(b,init,&values,targets,failure);
        values.push(value);
    }
    expr(b,&body.result,&values,targets,failure)
}
fn lower(plan:&Plan,module:&mut impl Module,linkage:Linkage)->Result<(FuncId,String),String>{
    let ptr=module.target_config().pointer_type();
    let mut ids=Vec::new();
    let mut ir=String::new();
    for (i,helper) in plan.helpers().iter().enumerate(){
        let mut sig=module.make_signature();
        for kind in &helper.params{
            sig.params.push(AbiParam::new(match kind{Kind::Ref=>ptr,Kind::Int=>types::I64}));
        }
        sig.returns.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I32));
        let id=module.declare_function(&format!("atto_ro_get_{i}"),Linkage::Local,&sig).map_err(|e|e.to_string())?;
        let mut ctx=module.make_context();ctx.func.signature=sig;
        let targets:Vec<_>=ids.iter().map(|id|module.declare_func_in_func(*id,&mut ctx.func)).collect();
        let mut fb=FunctionBuilderContext::new();
        let mut b=FunctionBuilder::new(&mut ctx.func,&mut fb);
        let entry=b.create_block();let failure=b.create_block();
        b.append_block_params_for_function_params(entry);b.switch_to_block(entry);
        let incoming=b.block_params(entry).to_vec();
        let value=body(&mut b,&helper.body,incoming,&targets,failure);
        let success=b.ins().iconst(types::I32,0);
        b.ins().return_(&[value,success]);
        b.switch_to_block(failure);
        let zero=b.ins().iconst(types::I64,0);let status=b.ins().iconst(types::I32,1);
        b.ins().return_(&[zero,status]);
        b.seal_all_blocks();b.finalize(module.target_config());
        validate_ir(&ctx.func,module.isa())?;
        ir.push_str(&ctx.func.display().to_string());ir.push('\n');
        module.define_function(id,&mut ctx).map_err(|e|e.to_string())?;
        ids.push(id);
    }
    // Stable scalar entry ABI; the root owns the only stack allocation.
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
    let value=body(&mut b,plan.root(),vec![owner],&targets,failure);
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

#[cfg(test)]
mod mixed_tests {
    use super::*;
    use crate::frontend::native_readonly::parse;
    const S:&str="// atto-ownership: borrow-default-v1\ndefine class Parcel as pub let int padding = 99; pub let int value = 20; end\nfnc sum(Parcel a, int n, Parcel b): int { val int left = a.value + n; val int right = b.value; return left + right; }\nfnc forward(Parcel p, int n): int { val int result = sum(p, n, p); return result; }\npub fnc main(): int { let p = new Parcel(); val int n = 2; return forward(p, n); }";
    #[test] fn mixed_reference_and_scalar_jit(){
        let c=compile(&parse(S).unwrap()).unwrap();
        assert_eq!(c.run(&[]).unwrap(),42);
        assert_eq!(c.clif.matches("explicit_slot").count(),1);
        assert!(c.clif.contains("load.i64"));
        assert!(!object(&parse(S).unwrap()).unwrap().is_empty());
        let over=S.replace("value = 20","value = 9223372036854775807");
        assert!(compile(&parse(&over).unwrap()).unwrap().run(&[]).is_err());
        assert_eq!(c.run(&[]).unwrap(),42);
    }
    #[test] fn mixed_reference_rejects_bad_authority_and_rebinding(){
        for bad in [
            S.replace("val int right = b.value;", "left = 1; val int right = b.value;"),
            S.replace("val int right", "val int left"),
            S.replace("sum(p, n, p)", "sum(p, n, new Parcel())"),
            S.replace("sum(p, n, p)", "sum(p, p, p)"),
            S.replace("sum(p, n, p)", "sum(n, n, p)"),
            S.replace("Parcel a, int n", "Parcel a, int mut n"),
            S.replace("return left + right;", "return a;"),
            S.replace("val int left = a.value + n;", "val int left = right;"),
        ]{ assert!(parse(&bad).is_err(),"{bad}"); }
    }
}
