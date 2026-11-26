// Closed scalar trusted-host provider adapter; not a general namespace linker.
// Caller authenticates the complete inventory and explicitly selects a symbol.
fn ident(s:&str)->bool{
    !s.is_empty() && s.len()<=128 && s.bytes().enumerate().all(|(i,c)|
        c.is_ascii_alphabetic()||c==b'_'||(i>0&&c.is_ascii_digit()))
}
fn deny(reason:&str)->String{format!("E_NAMESPACE_PROFILE: {reason}")}
#[cfg(test)]
fn compile(input:&str,namespace:&str,symbol:&str)->Result<Vec<u8>,String>{
    compile_arity(input,namespace,symbol,0)
}
pub(crate) fn compile_arity(input:&str,namespace:&str,symbol:&str,arity:usize)->Result<Vec<u8>,String>{
    if input.len()>4096||input.contains('\0')||!ident(namespace)||!ident(symbol)||symbol=="main"{
        return Err(deny("source or identifier budget"));
    }
    let header=format!("namespace {namespace};\npub fnc {symbol}(");
    let rest=input.strip_prefix(&header).ok_or_else(||deny("explicit public provider required"))?;
    let (params,tail)=rest.split_once(')').ok_or_else(||deny("signature"))?;
    let count=if params.is_empty(){0}else{params.split(", ").count()};
    if count!=arity||count>4{return Err("E_NAMESPACE_ARITY: incompatible selected call".into());}
    let mut names=std::collections::HashSet::new();
    if count>0 {
        for p in params.split(", "){
            let name=p.strip_prefix("int ").ok_or_else(||deny("exact integer parameters required"))?;
            if !ident(name)||!names.insert(name){return Err(deny("invalid parameter"));}
        }
    }
    let expr=tail.strip_prefix(": int { return ").and_then(|s|s.strip_suffix("; }\n"))
        .ok_or_else(||deny("closed expression body required"))?;
    if expr.is_empty()||expr.len()>1024||!expr.bytes().all(|b|b.is_ascii_alphanumeric()||b"_+*() -".contains(&b)){
        return Err(deny("closed integer expression required"));
    }
    let source=format!("use namespace {namespace};\npub fnc main({params}): int {{ return {expr}; }}\n");
    let program=crate::frontend::parse(&source)?;
    if program.parameters.len()!=arity||!program.locals.is_empty()||!program.trace_steps.is_empty(){
        return Err(deny("unexpected scalar shape"));
    }
    fn closed(e:&crate::frontend::Expr,depth:usize,budget:&mut usize,arity:usize)->bool{
        use crate::frontend::Expr::*;
        *budget+=1;
        if depth>32||*budget>256{return false;}
        match e{
            Literal(_)=>true,Param(i)=>*i<arity,
            Neg(x)=>closed(x,depth+1,budget,arity),
            Add(a,b)|Sub(a,b)|Mul(a,b)=>closed(a,depth+1,budget,arity)&&closed(b,depth+1,budget,arity),
            _=>false,
        }
    }
    if !closed(&program.result,0,&mut 0,arity){return Err(deny("closed integer expression required"));}
    crate::backend::emit_object(&program)
}
#[cfg(test)]
mod tests{
    use super::*;
    #[test]
    fn exact_admission_and_native_object(){
        let src="namespace billing;\npub fnc function0(): int { return 42; }\n";
        let bytes=compile(src,"billing","function0").unwrap();
        assert!(bytes.starts_with(b"\x7fELF"));
        assert_eq!(bytes,compile(src,"billing","function0").unwrap());
        assert!(compile(src,"identity","function0").unwrap_err().starts_with("E_NAMESPACE_PROFILE:"));
        assert!(compile(&src.replace("pub fnc","fnc"),"billing","function0").is_err());
        assert!(compile(&src.replace("()","(int x)").replace("42","x"),"billing","function0").unwrap_err().starts_with("E_NAMESPACE_ARITY:"));
        assert!(compile(&src.replace("42","9999999999999999999"),"billing","function0").is_err());
        assert!(compile(&src.replace("42","42; } fnc main(): int { return 0"),"billing","function0").is_err());
    }
}
