//! Native JIT for trusted stage-0 scalar source. Never treat JIT as a sandbox.
use crate::frontend::{Expr, Program};
use cranelift_codegen::ir::{AbiParam, InstBuilder, MemFlags, types};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{Linkage, Module, default_libcall_names};

pub struct Compiled {
    module: JITModule, // Own executable allocations until after every native call.
    id: cranelift_module::FuncId,
    pub clif: String,
}

fn emit(
    e: &Expr,
    b: &mut FunctionBuilder,
    params: &[cranelift_codegen::ir::Value],
    error: cranelift_codegen::ir::Block,
) -> cranelift_codegen::ir::Value {
    use Expr::*;
    match e {
        Literal(v) => b.ins().iconst(types::I64, *v),
        Param(i) => params[*i],
        _ => {
            let (value, overflow) = match e {
                Neg(x) => {
                    let rhs = emit(x,b,params,error);
                    let zero=b.ins().iconst(types::I64,0);
                    b.ins().ssub_overflow(zero,rhs)
                }
                Add(x,y) => {
                    let lhs=emit(x,b,params,error);
                    let rhs=emit(y,b,params,error);
                    b.ins().sadd_overflow(lhs,rhs)
                }
                Sub(x,y) => {
                    let lhs=emit(x,b,params,error);
                    let rhs=emit(y,b,params,error);
                    b.ins().ssub_overflow(lhs,rhs)
                }
                Mul(x,y) => {
                    let lhs=emit(x,b,params,error);
                    let rhs=emit(y,b,params,error);
                    b.ins().smul_overflow(lhs,rhs)
                }
                _ => unreachable!(),
            };
            let next=b.create_block();
            b.ins().brif(overflow,error,&[],next,&[]);
            b.switch_to_block(next);
            b.seal_block(next);
            value
        }
    }
}

pub fn compile(source: &Program) -> Result<Compiled,String> {
    let builder=JITBuilder::new(default_libcall_names()).map_err(|e|e.to_string())?;
    let mut module=JITModule::new(builder);
    let mut sig=module.make_signature();
    // Stable internal entry ABI: four i64 values + output pointer -> i32 status.
    // 0 is success; 1 is *checked arithmetic overflow*, output unchanged.
    for _ in 0..4 { sig.params.push(AbiParam::new(types::I64)); }
    sig.params.push(AbiParam::new(module.target_config().pointer_type()));
    sig.returns.push(AbiParam::new(types::I32));
    let id=module.declare_function("oreslang_scalar_entry",Linkage::Local,&sig)
        .map_err(|e|e.to_string())?;
    let mut ctx=module.make_context();
    ctx.func.signature=sig;
    let mut fbctx=FunctionBuilderContext::new();
    let mut b=FunctionBuilder::new(&mut ctx.func,&mut fbctx);
    let entry=b.create_block();
    let failure=b.create_block();
    b.append_block_params_for_function_params(entry);
    b.switch_to_block(entry);
    b.seal_block(entry);
    let p=b.block_params(entry).to_vec();
    let value=emit(&source.result,&mut b,&p[..4],failure);
    b.ins().store(MemFlags::new(),value,p[4],0);
    let ok=b.ins().iconst(types::I32,0);
    b.ins().return_(&[ok]);
    b.switch_to_block(failure);
    let fail=b.ins().iconst(types::I32,1);
    b.ins().return_(&[fail]);
    b.seal_all_blocks();
    b.finalize();
    let clif=ctx.func.display().to_string();
    module.define_function(id,&mut ctx).map_err(|e|e.to_string())?;
    module.clear_context(&mut ctx);
    module.finalize_definitions().map_err(|e|e.to_string())?;
    Ok(Compiled{module,id,clif})
}

impl Compiled {
    pub fn run(&self, args: &[i64]) -> Result<i64,String> {
        if args.len()>4 { return Err("too many runtime arguments".into()); }
        let mut padded=[0_i64;4];
        padded[..args.len()].copy_from_slice(args);
        let code=self.module.get_finalized_function(self.id);
        // SAFETY: emitted signature is exactly the C ABI below, the Module
        // owns the executable code for this entire call, and output is valid.
        // The caller MUST treat source as trusted; this is NOT an isolation
        // boundary for hostile Oreslang code or actor heaps.
        type Entry=unsafe extern "C" fn(i64,i64,i64,i64,*mut i64)->i32;
        let function:Entry=unsafe{std::mem::transmute(code)};
        let mut out=0_i64;
        let status=unsafe{function(padded[0],padded[1],padded[2],padded[3],&mut out)};
        if status==0 { Ok(out) }
        else if status==1 { Err("i64 arithmetic overflow".into()) }
        else { Err("invalid generated native status".into()) }
    }
}
