//! Draft Cranelift native object emitter. Trusted schema manifest only.
//! Statically links a bounded codec implementation; no JIT or storage authority.
use cranelift_codegen::ir::{AbiParam, InstBuilder, types};
use cranelift_codegen::settings::{self, Configurable};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_module::{DataDescription, Linkage, Module, default_libcall_names};
use cranelift_object::{ObjectBuilder, ObjectModule};
use std::collections::BTreeSet;
use std::io::Write;

fn manifest(args: &[String]) -> Result<(u64, Vec<u8>), String> {
    let deny = || "E_DURABLE_BOUNDED_SCHEMA_V2".to_owned();
    let valid = |n: &str| {
        let alpha = |b: u8| b.is_ascii_alphabetic() || b == b'_';
        n.len() <= 128
            && n.bytes().next().is_some_and(alpha)
            && n.bytes().all(|b| alpha(b) || b.is_ascii_digit())
    };
    let actor = args.first().ok_or_else(deny)?;
    if !valid(actor) || args.len() > 257 {
        return Err(deny());
    }
    let mut sig = b"atto.actor.state.schema.v2\0".to_vec();
    fn sized(sig: &mut Vec<u8>, n: &str) {
        sig.extend_from_slice(&(n.len() as u32).to_le_bytes());
        sig.extend_from_slice(n.as_bytes());
    }
    sized(&mut sig, actor);
    let mut tags = Vec::new();
    let mut names = BTreeSet::new();
    for field in &args[1..] {
        let (name, kind) = field.split_once(':').ok_or_else(deny)?;
        if !valid(name) || !names.insert(name) {
            return Err(deny());
        }
        let tag = match kind {
            "int" => 1,
            "bool" => 2,
            "string" => 3,
            "bytes" => 4,
            _ => return Err(deny()),
        };
        sized(&mut sig, name);
        sig.push(tag);
        tags.push(tag);
    }
    let schema = sig.iter().fold(0xcbf29ce484222325u64, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(0x100000001b3)
    });
    Ok((schema, tags))
}
fn emit(schema: u64, mut tags: Vec<u8>) -> Result<Vec<u8>, String> {
    let count = tags.len() as i64;
    tags.push(0); // non-null read-only object even for empty schema
    let mut flags = settings::builder();
    flags.set("is_pic", "true").map_err(|e| e.to_string())?;
    let isa = cranelift_native::builder()
        .map_err(|e| e.to_string())?
        .finish(settings::Flags::new(flags))
        .map_err(|e| e.to_string())?;
    let builder = ObjectBuilder::new(isa, "atto_bounded_v2", default_libcall_names())
        .map_err(|e| e.to_string())?;
    let mut module = ObjectModule::new(builder);
    let ptr = module.target_config().pointer_type();
    if ptr != types::I64 {
        return Err("64-bit native target required".into());
    }
    let data_id = module
        .declare_data("atto_bounded_tags_v2", Linkage::Local, false, false)
        .map_err(|e| e.to_string())?;
    let mut data = DataDescription::new();
    data.define(tags.into_boxed_slice());
    module
        .define_data(data_id, &data)
        .map_err(|e| e.to_string())?;
    for (name, mode) in [
        ("atto_bounded_fields_v2", 0),
        ("atto_bounded_schema_v2", 1),
        ("atto_bounded_encode_v2", 2),
        ("atto_bounded_decode_v2", 3),
    ] {
        let mut sig = module.make_signature();
        if mode >= 2 {
            for t in [ptr, types::I32, ptr, types::I32] {
                sig.params.push(AbiParam::new(t));
            }
        }
        let ret = if mode == 1 { types::I64 } else { types::I32 };
        sig.returns.push(AbiParam::new(ret));
        let id = module
            .declare_function(name, Linkage::Export, &sig)
            .map_err(|e| e.to_string())?;
        let mut ctx = module.make_context();
        ctx.func.signature = sig;
        let imported = if mode >= 2 {
            let mut imp = module.make_signature();
            for t in [
                ptr,
                types::I32,
                ptr,
                types::I32,
                types::I64,
                ptr,
                types::I32,
            ] {
                imp.params.push(AbiParam::new(t));
            }
            imp.returns.push(AbiParam::new(types::I32));
            let target = module
                .declare_function(
                    if mode == 2 {
                        "atto_bounded_encode_v2_impl"
                    } else {
                        "atto_bounded_decode_v2_impl"
                    },
                    Linkage::Import,
                    &imp,
                )
                .map_err(|e| e.to_string())?;
            Some((
                module.declare_func_in_func(target, &mut ctx.func),
                module.declare_data_in_func(data_id, &mut ctx.func),
            ))
        } else {
            None
        };
        let mut fbctx = FunctionBuilderContext::new();
        let mut b = FunctionBuilder::new(&mut ctx.func, &mut fbctx);
        let entry = b.create_block();
        b.append_block_params_for_function_params(entry);
        b.switch_to_block(entry);
        let value = if let Some((target, tag_data)) = imported {
            let mut args = b.block_params(entry).to_vec();
            args.push(b.ins().iconst(types::I64, schema as i64));
            args.push(b.ins().symbol_value(ptr, tag_data));
            args.push(b.ins().iconst(types::I32, count));
            let call = b.ins().call(target, &args);
            b.inst_results(call)[0]
        } else {
            b.ins()
                .iconst(ret, if mode == 0 { count } else { schema as i64 })
        };
        b.ins().return_(&[value]);
        b.seal_all_blocks();
        b.finalize(module.target_config());
        cranelift_codegen::verify_function(&ctx.func, module.isa()).map_err(|e| e.to_string())?;
        module
            .define_function(id, &mut ctx)
            .map_err(|e| e.to_string())?;
    }
    module.finish().emit().map_err(|e| e.to_string())
}
fn main() {
    let result = (|| -> Result<(), String> {
        // Invalid OS argument encoding is a controlled rejection, not a panic.
        // Bound each trusted-manifest token and the number of tokens before emission.
        let args: Vec<String> = std::env::args_os()
            .skip(1)
            .take(258)
            .map(|arg| {
                let s = arg
                    .into_string()
                    .map_err(|_| "E_DURABLE_BOUNDED_SCHEMA_V2".to_owned())?;
                if s.len() > 135 {
                    return Err("E_DURABLE_BOUNDED_SCHEMA_V2".to_owned());
                }
                Ok(s)
            })
            .collect::<Result<_, String>>()?;
        let (schema, tags) = manifest(&args)?;
        let object = emit(schema, tags)?;
        std::io::stdout()
            .lock()
            .write_all(&object)
            .map_err(|e| e.to_string())
    })();
    if let Err(error) = result {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
