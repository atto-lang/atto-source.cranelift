//! Opt-in nominal scalar values. Real typed AST -> validated CLIF object,
//! not constant evaluation or a physical aggregate/actor ABI.
use super::{Expr, Parser, Program, Token, integer_digits, reject_source_unicode_controls};
use std::collections::BTreeMap;
type R<T> = Result<T,String>;
type Slots = BTreeMap<String,Scalar>;
#[derive(Clone,Copy,PartialEq,Eq,Debug)]
enum Ty { Int, Bool }
#[derive(Clone)]
struct Scalar { ty:Ty, expr:Expr }
#[derive(Clone)]
struct Field { name:String, ty:String }
#[derive(Clone)]
struct Leaf { path:String, ty:Ty }
#[derive(Clone)]
struct Ctor { params:Vec<Field>, assigns:Vec<(String,String)> }
#[derive(Clone)]
struct Method { ty:Ty, body:Expr }
#[derive(Clone)]
struct Mutation { params:Vec<Field>, writes:Vec<(String,Scalar)> }
#[derive(Clone)]
struct Owner { fields:Vec<Field>, leaves:Vec<Leaf>, ctors:Vec<Ctor>, methods:BTreeMap<String,Method>, mutations:BTreeMap<String,Mutation> }
#[derive(Clone)]
struct Value { owner:String, slots:Slots, writable:bool }
struct Argument { ty:String, slots:Slots }
struct Reader<'a> {
    p:Parser<'a>, owners:BTreeMap<String,Owner>, values:BTreeMap<String,Value>,
    locals:Vec<Expr>, tokens:usize, nodes:usize, members:usize,
}
fn demand(ok:bool,code:&str,why:&str)->R<()> {
    if ok {Ok(())} else {Err(format!("{code}: {why}"))}
}
fn scalar(t:&str)->Option<Ty>{match t {"int"=>Some(Ty::Int),"bool"=>Some(Ty::Bool),_=>None}}
fn spelling(t:Ty)->String{match t{Ty::Int=>"int",Ty::Bool=>"bool"}.into()}
fn path(a:&str,b:&str)->String{if b.is_empty(){a.to_owned()}else{format!("{a}.{b}")}}
fn local(ty:Ty,index:usize)->Scalar{Scalar{ty,expr:Expr::Local(index)}}
impl<'a> Reader<'a>{
    fn tick(&mut self)->R<()> {self.tokens+=1;demand(self.tokens<=8192,"E_SOURCE_LIMIT","token budget")}
    fn eat(&mut self,t:Token<'a>)->R<bool>{self.tick()?;self.p.take(t)}
    fn expect(&mut self,t:Token<'a>)->R<()>{self.tick()?;self.p.expect(t).map_err(|e|format!("E_STRUCT_SYNTAX: {e}"))}
    fn w(&mut self,t:&'a str)->R<bool>{self.eat(Token::Word(t))}
    fn ew(&mut self,t:&'a str)->R<()>{self.expect(Token::Word(t))}
    fn p(&mut self,t:u8)->R<bool>{self.eat(Token::Punct(t))}
    fn ep(&mut self,t:u8)->R<()>{self.expect(Token::Punct(t))}
    fn name(&mut self)->R<String>{
        self.tick()?;
        let n=self.p.aggregate_name()?;
        demand(n.bytes().all(|b|b.is_ascii_alphanumeric()||b==b'_')&&!matches!(n.as_str(),"constructor"|"mtd"|"with"|"copy"|"move"|"take"|"share"|"borrow"|"mut"|"define"|"type"|"return"|"let"|"true"|"false"),"E_STRUCT_SYNTAX","reserved name")?;
        Ok(n)
    }
    fn type_name(&mut self)->R<String>{
        if self.w("int")?{return Ok("int".into())}
        if self.w("bool")?{return Ok("bool".into())}
        let n=self.name()?;
        demand(self.owners.contains_key(&n),"E_TYPE","unknown, forward or cyclic nominal owner")?;
        Ok(n)
    }
    fn leaves(&self,t:&str)->Vec<Leaf>{
        if let Some(ty)=scalar(t){vec![Leaf{path:String::new(),ty}]}
        else{self.owners[t].leaves.clone()}
    }
    fn snapshot(&mut self,v:Scalar)->R<Scalar>{
        demand(self.locals.len()<64,"E_SOURCE_LIMIT","native scalar locals")?;
        let i=self.locals.len();self.locals.push(v.expr);Ok(local(v.ty,i))
    }
    fn formals(&mut self)->R<Vec<Field>>{
        self.ep(b'(')?;let mut f=Vec::<Field>::new();
        if !self.p(b')')?{
            loop{
                let ty=self.type_name()?;let name=self.name()?;
                demand(!f.iter().any(|p|p.name==name)&&!self.owners.contains_key(&name),"E_DUPLICATE","parameter")?;
                f.push(Field{name,ty});
                demand(f.iter().map(|p|self.leaves(&p.ty).len()).sum::<usize>()<=8,"E_SOURCE_LIMIT","formal leaves")?;
                if !self.p(b',')?{break}
            }
            self.ep(b')')?;
        }Ok(f)
    }
    fn member(&mut self,o:&mut Owner)->R<()>{
        self.members+=1;demand(self.members<=64,"E_SOURCE_LIMIT","attached members")?;
        if self.w("constructor")?{
            let params=self.formals()?;let mut assigns=Vec::<(String,String)>::new();
            demand(!o.ctors.iter().any(|c|c.params.iter().map(|p|&p.ty).eq(params.iter().map(|p|&p.ty))),"E_DUPLICATE","constructor signature")?;
            self.ep(b'{')?;
            while !self.p(b'}')?{
                self.ew("self")?;self.ep(b'.')?;let field=self.name()?;self.ep(b'=')?;let param=self.name()?;self.ep(b';')?;
                let f=o.fields.iter().find(|f|f.name==field).ok_or("E_STRUCT_INIT: unknown field")?;
                let p=params.iter().find(|p|p.name==param).ok_or("E_STRUCT_INIT: expected whole-field parameter")?;
                demand(f.ty==p.ty,"E_TYPE","nominal field identity")?;
                demand(!assigns.iter().any(|(n,_)|n==&field),"E_STRUCT_INIT","duplicate field initialization")?;
                assigns.push((field,param));
            }
            demand(assigns.len()==o.fields.len(),"E_STRUCT_INIT","missing field initialization")?;
            o.ctors.push(Ctor{params,assigns});return Ok(())
        }
        let mutable=self.w("mut")?;self.ew("mtd")?;
        let (name,params,ret)=if matches!(self.p.current,Token::Word("int"|"bool")){
            let ty=self.type_name()?;
            if !self.w("•")?{self.ew("<-")?}
            (self.name()?,self.formals()?,ty)
        }else{
            let name=self.name()?;let params=self.formals()?;
            if !self.w("->")?{self.ep(b':')?}
            let ty=if self.w("void")?{"void".into()}else{self.type_name()?};
            (name,params,ty)
        };
        demand(!o.methods.contains_key(&name)&&!o.mutations.contains_key(&name),"E_DUPLICATE","method name")?;
        let mut env=Slots::new();
        for (i,l) in o.leaves.iter().enumerate(){env.insert(l.path.clone(),local(l.ty,i));}
        for (i,f) in params.iter().enumerate(){
            let ty=scalar(&f.ty).ok_or("E_TYPE: scalar method parameters required")?;
            env.insert(format!("$arg.{}",f.name),local(ty,o.leaves.len()+i));
        }
        demand(o.leaves.len()+params.len()<=8,"E_SOURCE_LIMIT","method scalar frame")?;
        self.ep(b'{')?;
        if mutable{
            demand(ret=="void","E_TYPE","mutable method must return void")?;
            let mut writes=Vec::new();
            while !self.p(b'}')?{
                if self.w("return")?{self.ep(b';')?;self.ep(b'}')?;break}
                self.ew("self")?;self.ep(b'.')?;let mut field=self.name()?;
                while self.p(b'.')?{field=path(&field,&self.name()?);}
                let ty=env.get(&field).ok_or("E_TYPE: unknown mutable leaf")?.ty;
                self.ep(b'=')?;let expr=self.expression(Some(&env),1,0)?;self.ep(b';')?;
                demand(ty==expr.ty,"E_TYPE","mutation leaf type")?;
                demand(writes.len()<16,"E_SOURCE_LIMIT","mutable writes")?;writes.push((field,expr));
            }
            demand(!writes.is_empty(),"E_STRUCT_SYNTAX","empty mutable body")?;
            o.mutations.insert(name,Mutation{params,writes});
        }else{
            demand(params.is_empty(),"E_STRUCT_PROFILE","readonly method parameters")?;
            let ty=scalar(&ret).ok_or("E_TYPE: scalar method result")?;
            self.ew("return")?;let expr=self.expression(Some(&env),1,0)?;self.ep(b';')?;self.ep(b'}')?;
            demand(ty==expr.ty,"E_TYPE","method result")?;
            o.methods.insert(name,Method{ty,body:expr.expr});
        }Ok(())
    }
    fn declaration(&mut self)->R<()>{
        let name=if self.w("define")?{self.ew("struct")?;self.name()?}
            else{self.ew("type")?;let n=self.name()?;self.ep(b'=')?;self.ew("struct")?;n};
        demand(!self.owners.contains_key(&name)&&self.owners.len()<16,"E_DUPLICATE","nominal owner")?;
        let mut o=Owner{fields:vec![],leaves:vec![],ctors:vec![],methods:BTreeMap::new(),mutations:BTreeMap::new()};
        self.ep(b'{')?;
        while !self.p(b'}')?{
            let ty=self.type_name()?;let field=self.name()?;self.ep(b';')?;
            demand(!o.fields.iter().any(|f|f.name==field),"E_DUPLICATE","field")?;
            for l in self.leaves(&ty){o.leaves.push(Leaf{path:path(&field,&l.path),ty:l.ty})}
            demand(o.leaves.len()<=8,"E_SOURCE_LIMIT","nominal scalar leaves")?;
            o.fields.push(Field{name:field,ty});
        }
        demand(!o.fields.is_empty(),"E_TYPE","empty nominal value")?;
        loop{
            self.ew("with")?;
            if self.p(b'{')?{
                demand(self.p.current!=Token::Punct(b'}'),"E_STRUCT_SYNTAX","empty attachment")?;
                while !self.p(b'}')?{self.member(&mut o)?}
            }else{self.member(&mut o)?}
            if self.p.current!=Token::Word("with"){break}
        }
        demand(!o.ctors.is_empty(),"E_STRUCT_INIT","constructor required")?;
        self.owners.insert(name,o);Ok(())
    }
    fn operator(&self)->(&'static str,u8){
        match self.p.current{
            Token::Word("||")=>("||",1),Token::Word("&&")=>("&&",2),
            Token::Word("==")=>("==",3),Token::Word("!=")=>("!=",3),
            Token::Punct(b'<')=>("<",4),Token::Punct(b'>')=>(">",4),
            Token::Word("<=")=>("<=",4),Token::Word(">=")=>(">=",4),
            Token::Punct(b'+')=>("+",5),Token::Punct(b'-')=>("-",5),Token::Punct(b'*')=>("*",6),_=>("",0)
        }
    }
    fn expression(&mut self,env:Option<&Slots>,min:u8,depth:usize)->R<Scalar>{
        demand(depth<32,"E_SOURCE_LIMIT","expression depth")?;
        self.nodes+=1;demand(self.nodes<=1024,"E_SOURCE_LIMIT","expression nodes")?;
        let mut a=self.atom(env,depth+1)?;let mut chain=0;
        loop{
            let (op,prec)=self.operator();if prec<min{break}
            chain+=1;demand(chain<=16,"E_SOURCE_LIMIT","expression chain")?;
            self.tick()?;self.p.next()?;let b=self.expression(env,prec+1,depth+1)?;
            demand(a.ty==b.ty,"E_TYPE","mixed scalar operands")?;
            let ints=a.ty==Ty::Int;let left=Box::new(a.expr);let right=Box::new(b.expr);
            a=match op{
                "+"|"-"|"*" if ints=>Scalar{ty:Ty::Int,expr:match op{"+"=>Expr::Add(left,right),"-"=>Expr::Sub(left,right),_=>Expr::Mul(left,right)}},
                "=="|"!="=>Scalar{ty:Ty::Bool,expr:Expr::Compare(if op=="=="{0}else{1},left,right)},
                "<"|">"|"<="|">=" if ints=>Scalar{ty:Ty::Bool,expr:Expr::Compare(match op{"<"=>2,"<="=>3,">"=>4,_=>5},left,right)},
                "&&" if !ints=>Scalar{ty:Ty::Bool,expr:Expr::Choose(left,right,Box::new(Expr::Literal(0)))},
                "||" if !ints=>Scalar{ty:Ty::Bool,expr:Expr::Choose(left,Box::new(Expr::Literal(1)),right)},
                _=>return Err("E_TYPE: scalar operator".into())
            };
        }Ok(a)
    }
    fn atom(&mut self,env:Option<&Slots>,depth:usize)->R<Scalar>{
        if self.p(b'(')?{let e=self.expression(env,1,depth)?;self.ep(b')')?;return Ok(e)}
        if self.p(b'-')?{
            if self.p.current==Token::Number("9223372036854775808"){self.tick()?;self.p.next()?;return Ok(Scalar{ty:Ty::Int,expr:Expr::Literal(i64::MIN)})}
            let e=self.expression(env,7,depth)?;demand(e.ty==Ty::Int,"E_TYPE","negation")?;return Ok(Scalar{ty:Ty::Int,expr:Expr::Neg(Box::new(e.expr))})
        }
        if self.w("true")?{return Ok(Scalar{ty:Ty::Bool,expr:Expr::Literal(1)})}
        if self.w("false")?{return Ok(Scalar{ty:Ty::Bool,expr:Expr::Literal(0)})}
        if let Token::Number(raw)=self.p.current{
            let n=integer_digits(raw)?.parse::<i64>().map_err(|_|"E_TYPE: integer range")?;
            self.tick()?;self.p.next()?;return Ok(Scalar{ty:Ty::Int,expr:Expr::Literal(n)})
        }
        let root=if self.w("self")?{"self".to_owned()}else{self.name()?};
        if let Some(env)=env{
            if let Some(p)=env.get(&format!("$arg.{root}")){return Ok(p.clone())}
            demand(root=="self","E_TYPE","method lexical scope")?;
        }
        self.ep(b'.')?;let mut field=self.name()?;
        if env.is_none()&&self.p(b'(')?{
            self.ep(b')')?;let v=self.values.get(&root).ok_or("E_TYPE: unknown receiver")?;
            let o=&self.owners[&v.owner];let m=o.methods.get(&field).ok_or("E_TYPE: unknown readonly method")?;
            return Ok(Scalar{ty:m.ty,expr:Expr::ScopedCall(o.leaves.iter().map(|l|v.slots[&l.path].expr.clone()).collect(),vec![],Box::new(m.body.clone()))})
        }
        while self.p(b'.')?{field=path(&field,&self.name()?);}
        let slots=if let Some(env)=env{env}else{&self.values.get(&root).ok_or("E_TYPE: unknown value")?.slots};
        slots.get(&field).cloned().ok_or_else(||"E_TYPE: unknown scalar projection".into())
    }
    // Only a complete nominal operand can enter the copy/constructor lane.
    // A scalar projection or readonly call must go through typed expression parsing.
    fn value_name(&self)->Option<String>{
        if let Token::Word(n)=self.p.current
            && self.values.contains_key(n) && !self.p.next_significant_is(b'.')
        {return Some(n.to_owned())}
        None
    }
    fn snapshot_slots(&mut self,v:&Value)->R<Slots>{
        let mut slots=Slots::new();
        for l in self.owners[&v.owner].leaves.clone(){slots.insert(l.path.clone(),self.snapshot(v.slots[&l.path].clone())?);}
        Ok(slots)
    }
    fn binding(&mut self)->R<()>{
        self.ew("let")?;let writable=self.w("mut")?;let first=self.name()?;
        let (annotation,name)=if self.owners.contains_key(&first){(Some(first),self.name()?)}else{(None,first)};
        demand(!self.values.contains_key(&name)&&!self.owners.contains_key(&name),"E_DUPLICATE","local shadow")?;
        self.ep(b'=')?;let copy=self.w("copy!")?;
        let (owner,slots)=if let Some(old)=self.value_name(){
            self.tick()?;self.p.next()?;let v=self.values[&old].clone();let slots=self.snapshot_slots(&v)?;(v.owner,slots)
        }else{
            demand(!copy,"E_TYPE","copy requires nominal local")?;
            let owner=self.name()?;let o=self.owners.get(&owner).cloned().ok_or("E_TYPE: unknown constructor")?;
            let mut args=Vec::<Argument>::new();self.ep(b'(')?;
            if !self.p(b')')?{
                loop{
                    if let Some(old)=self.value_name(){self.tick()?;self.p.next()?;let v=self.values[&old].clone();args.push(Argument{ty:v.owner.clone(),slots:self.snapshot_slots(&v)?});}
                    else{let e=self.expression(None,1,0)?;args.push(Argument{ty:spelling(e.ty),slots:BTreeMap::from([(String::new(),self.snapshot(e)?)])});}
                    demand(args.iter().map(|a|a.slots.len()).sum::<usize>()<=8,"E_SOURCE_LIMIT","constructor arguments")?;
                    if !self.p(b',')?{break}
                }self.ep(b')')?;
            }
            let c=o.ctors.iter().find(|c|c.params.iter().map(|p|&p.ty).eq(args.iter().map(|a|&a.ty))).ok_or("E_TYPE: no exact nominal constructor")?;
            let mut slots=Slots::new();
            for (field,param) in &c.assigns{
                let index=c.params.iter().position(|p|&p.name==param).ok_or("E_STRUCT_INIT: unknown parameter")?;
                for (leaf,e) in &args[index].slots{slots.insert(path(field,leaf),self.snapshot(e.clone())?);}
            }(owner,slots)
        };
        demand(annotation.as_ref().is_none_or(|a|a==&owner),"E_TYPE","nominal binding identity")?;
        self.ep(b';')?;self.values.insert(name,Value{owner,slots,writable});Ok(())
    }
    fn invoke(&mut self)->R<()>{
        let name=self.name()?;self.ep(b'.')?;let member=self.name()?;
        let mut v=self.values.get(&name).cloned().ok_or("E_TYPE: unknown receiver")?;
        let o=self.owners[&v.owner].clone();let m=o.mutations.get(&member).ok_or("E_TYPE: unknown mutable method")?;
        demand(v.writable,"E_BORROW_MUT_REQUIRED","immutable receiver")?;
        let mut args=Vec::<Expr>::new();self.ep(b'(')?;
        if !self.p(b')')?{
            loop{
                demand(args.len()<m.params.len(),"E_TYPE","mutation arity")?;
                let e=self.expression(None,1,0)?;
                demand(Some(e.ty)==scalar(&m.params[args.len()].ty),"E_TYPE","mutation argument")?;
                args.push(self.snapshot(e)?.expr);
                if !self.p(b',')?{break}
            }self.ep(b')')?;
        }
        demand(args.len()==m.params.len(),"E_TYPE","mutation arity")?;self.ep(b';')?;
        for (field,e) in &m.writes{
            let operands=o.leaves.iter().map(|l|v.slots[&l.path].expr.clone()).chain(args.iter().cloned()).collect();
            let updated=self.snapshot(Scalar{ty:e.ty,expr:Expr::ScopedCall(operands,vec![],Box::new(e.expr.clone()))})?;
            v.slots.insert(field.clone(),updated);
        }
        self.values.insert(name,v);Ok(())
    }
    fn finish(mut self)->R<Program>{
        while matches!(self.p.current,Token::Word("define"|"type")){self.declaration()?}
        demand(!self.owners.is_empty(),"E_STRUCT_PROFILE","attached owner required")?;
        self.w("pub")?;self.ew("fnc")?;self.ew("main")?;self.ep(b'(')?;self.ep(b')')?;
        if !self.w("->")?{self.ep(b':')?}self.ew("int")?;self.ep(b'{')?;
        while self.p.current!=Token::Word("return"){if self.p.current==Token::Word("let"){self.binding()?}else{self.invoke()?}}
        self.ew("return")?;let e=self.expression(None,1,0)?;
        demand(e.ty==Ty::Int,"E_TYPE","main must return int")?;self.ep(b';')?;self.ep(b'}')?;self.expect(Token::End)?;
        Ok(Program{parameters:vec![],locals:self.locals,result:e.expr,trace_steps:vec![]})
    }
}
pub(crate) fn parse(source:&str)->R<Program>{
    demand(source.len()<=65536,"E_SOURCE_LIMIT","attached source bytes")?;
    reject_source_unicode_controls(source)?;
    let mut p=Parser::new(source)?;
    p.attached_struct_target=true;p.attached_scalar_values=true;
    Reader{p,owners:BTreeMap::new(),values:BTreeMap::new(),locals:vec![],tokens:0,nodes:0,members:0}.finish()
}
