//! Bounded rational-scale quantity admission on the existing stage-0 lexer and native Expr IR.
//! Dimensions are erased only after the entire closed expression is type checked.
use super::{Expr, Parser, Program, Token, MAX_DEPTH, integer_digits, validate_ast_depth};
type Dim = [i8; 8];
const SCALAR: Dim = [0; 8];
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Scale { n:i64, d:i64 }
fn gcd(mut a:i64,mut b:i64)->i64 {while b!=0{let r=a%b;a=b;b=r;}a}
impl Scale {
    const ONE:Self=Self{n:1,d:1};
    fn inv(self)->Self{Self{n:self.d,d:self.n}}
    fn mul(self,rhs:Self)->Result<Self,String>{
        let a=gcd(self.n,rhs.d);let b=gcd(rhs.n,self.d);
        Ok(Self{n:(self.n/a).checked_mul(rhs.n/b).ok_or("E_UNIT_LIMIT: numerator")?,
                d:(self.d/b).checked_mul(rhs.d/a).ok_or("E_UNIT_LIMIT: denominator")?})
    }
    fn convert(self,value:i64,target:Self)->Result<i64,String>{
        let ratio=self.mul(target.inv())?;
        if value%ratio.d!=0{return Err("E_UNIT_INEXACT: nonintegral conversion".into());}
        (value/ratio.d).checked_mul(ratio.n).ok_or_else(||"E_UNIT_RANGE: conversion exceeds i64".into())
    }
}
#[derive(Clone,Copy)]
struct Range {lo:i64,hi:i64}
impl Range {
    const FULL:Self=Self{lo:i64::MIN,hi:i64::MAX};
    fn exact(v:i64)->Self{Self{lo:v,hi:v}}
    fn contained(self,lo:i64,hi:i64)->bool{self.lo>=lo&&self.hi<=hi}
    fn bounds(lo:i128,hi:i128)->Self{
        let lo=lo.max(i64::MIN as i128);let hi=hi.min(i64::MAX as i128);
        if lo>hi{Self::FULL}else{Self{lo:lo as i64,hi:hi as i64}}
    }
    fn neg(self)->Self{Self::bounds(-(self.hi as i128),-(self.lo as i128))}
    fn binary(self,rhs:Self,op:u8)->Self{
        let(a,b,c,d)=(self.lo as i128,self.hi as i128,rhs.lo as i128,rhs.hi as i128);
        match op{
            b'+'=>Self::bounds(a+c,b+d),
            b'-'=>Self::bounds(a-d,b-c),
            b'*'=>{let v=[a*c,a*d,b*c,b*d];Self::bounds(*v.iter().min().unwrap(),*v.iter().max().unwrap())},
            b'/' if c>0||d<0=>{let v=[a/c,a/d,b/c,b/d];Self::bounds(*v.iter().min().unwrap(),*v.iter().max().unwrap())},
            _=>Self::FULL
        }
    }
}
struct Typed { expr: Expr, dim: Dim, scale:Scale, range:Range }
#[derive(Clone,Copy)]
struct Binding {dim:Dim,scale:Scale,range:Range}
#[derive(Clone,Copy)]
struct Refinement {dim:Dim,scale:Scale,lo:i64,hi:i64}
struct Quantities<'a> {
    p: Parser<'a>,
    dimensions: Vec<String>,
    units: Vec<(String, usize, Scale)>,
    refinements: Vec<(String,Refinement)>,
    bindings: Vec<(String,Binding)>,
}
impl<'a> Quantities<'a> {
    fn declaration_name(&mut self) -> Result<String,String> {
        let name=self.p.name()?;
        if !name.as_bytes().first().is_some_and(u8::is_ascii_uppercase)
            || self.dimensions.contains(&name) || self.units.iter().any(|(n,_,_)|n==&name) || self.refinements.iter().any(|(n,_)|n==&name) {
            return Err("E_UNIT_DECLARATION: invalid or duplicate nominal name".into());
        }
        Ok(name)
    }
    fn factor(&mut self)->Result<i64,String>{
        if let Token::Number(n)=self.p.current{
            let v=integer_digits(n)?.parse::<i64>().map_err(|_|"E_UNIT_SCALE: invalid factor")?;
            if v>0{self.p.next()?;return Ok(v);}
        }
        Err("E_UNIT_SCALE: expected positive i64 factor".into())
    }
    fn declarations(&mut self) -> Result<(),String> {
        while self.p.take(Token::Word("define"))? {
            let dimension=self.p.take(Token::Word("dimension"))?;
            if !dimension { self.p.expect(Token::Word("unit"))?; }
            let name=self.declaration_name()?;
            if dimension {
                if self.dimensions.len()==8 { return Err("E_UNIT_LIMIT: dimensions".into()); }
                self.dimensions.push(name);
            } else {
                if self.units.len()==32 { return Err("E_UNIT_LIMIT: units".into()); }
                self.p.expect(Token::Punct(b':'))?;
                let target=self.p.name()?;
                let index=self.dimensions.iter().position(|n|n==&target)
                    .ok_or("E_UNIT_DECLARATION: unknown dimension")?;
                let scale=if self.p.take(Token::Punct(b'='))?{
                    let (base,factor)=if matches!(self.p.current,Token::Number(_)){
                        let n=self.factor()?;self.p.expect(Token::Punct(b'*'))?;(self.p.name()?,Scale{n,d:1})
                    }else{
                        let base=self.p.name()?;
                        let factor=if self.p.take(Token::Punct(b'*'))?{Scale{n:self.factor()?,d:1}}
                        else if self.p.take(Token::Punct(b'/'))?{Scale{n:1,d:self.factor()?}}else{Scale::ONE};
                        (base,factor)
                    };
                    let (_,_,scale)=self.units.iter().find(|(n,i,_)|n==&base&&*i==index)
                        .ok_or("E_UNIT_DECLARATION: unknown or incompatible alias")?;
                    scale.mul(factor)?
                }else{
                    if self.units.iter().any(|(_,i,_)|*i==index){return Err("E_UNIT_DECLARATION: duplicate base unit".into());}
                    Scale::ONE
                };
                self.units.push((name,index,scale));
            }
            self.p.expect(Token::Punct(b';'))?;
        }
        Ok(())
    }
    fn signed_bound(&mut self)->Result<i64,String>{
        let neg=self.p.take(Token::Punct(b'-'))?;
        let Token::Number(n)=self.p.current else{return Err("E_REFINEMENT_BOUND: expected literal".into());};
        let n=integer_digits(n)?;self.p.next()?;
        let signed=if neg{format!("-{n}")}else{n};
        signed.parse::<i64>().map_err(|_|"E_REFINEMENT_BOUND: out of range".into())
    }
    fn refinement_declarations(&mut self)->Result<(),String>{
        while self.p.take(Token::Word("type"))?{
            if self.refinements.len()==32{return Err("E_REFINEMENT_LIMIT".into());}
            let name=self.declaration_name()?;
            self.p.expect(Token::Punct(b'='))?;
            if !self.p.take(Token::Word("int"))?{self.p.expect(Token::Word("i64"))?;}
            self.p.expect(Token::Word("where"))?;
            let(mut lo,mut hi)=(i64::MIN,i64::MAX);
            let mut count=0;
            let mut has_units=false;
            loop{
                self.p.expect(Token::Word("self"))?;
                let end=self.p.pos;
                let lower=if self.p.take(Token::Punct(b'>'))?{true}else{self.p.expect(Token::Punct(b'<'))?;false};
                let inclusive=self.p.pos==end+1&&self.p.take(Token::Punct(b'='))?;
                let bound=self.signed_bound()?;
                if lower{let n=if inclusive{bound}else{bound.checked_add(1).ok_or("E_REFINEMENT_INTERVAL: empty")?};lo=lo.max(n);}
                else{let n=if inclusive{bound}else{bound.checked_sub(1).ok_or("E_REFINEMENT_INTERVAL: empty")?};hi=hi.min(n);}
                count+=1;if count>16||lo>hi{return Err("E_REFINEMENT_INTERVAL: empty or excessive predicates".into());}
                if !self.p.take(Token::Word("and"))?{break;}
                if self.p.take(Token::Word("units"))?{has_units=true;break;}
            }
            let(mut dim,mut scale)=(SCALAR,Scale::ONE);
            if has_units{
                self.p.expect(Token::Word("in"))?;
                let name=self.p.name()?;
                let (_,i,s)=self.units.iter().find(|(n,_,_)|n==&name).ok_or("E_UNIT_NAME: unknown refinement unit")?;
                dim[*i]=1;scale=*s;
            }
            self.p.expect(Token::Punct(b';'))?;
            self.refinements.push((name,Refinement{dim,scale,lo,hi}));
        }
        Ok(())
    }
    fn attach(&mut self, value:&mut Typed)->Result<(),String>{
        if value.dim!=SCALAR||value.scale!=Scale::ONE {return Err("E_UNIT_ATTACHMENT: already dimensioned".into());}
        let name=self.p.name()?;
        let (_,index,scale)=self.units.iter().find(|(n,_,_)|n==&name)
            .ok_or("E_UNIT_ATTACHMENT: unknown or unsupported unit")?;
        value.dim[*index]=1;value.scale=*scale;
        Ok(())
    }
    fn scalar(value:&Typed)->Result<(),String>{
        if value.dim!=SCALAR||value.scale!=Scale::ONE {return Err("E_UNIT_ERASURE: scalar cast or return would erase units".into());}
        Ok(())
    }
    fn checked_refinement(&mut self,value:Typed,target:Refinement)->Result<Typed,String>{
        if value.dim!=target.dim{return Err("E_UNIT_DIMENSION: refinement dimension mismatch".into());}
        if value.scale!=target.scale{return Err("E_UNIT_SCALE: refinement scale mismatch".into());}
        let expr=self.p.node(Expr::RefinementCast(Box::new(value.expr),target.lo,target.hi))?;
        Ok(Typed{expr,dim:value.dim,scale:value.scale,range:Range{lo:target.lo,hi:target.hi}})
    }
    fn target(&mut self,value:Typed,cast:bool)->Result<Typed,String>{
        let mut value=value;
        if cast && matches!(self.p.current,Token::Word("int"|"i64")) {
            self.p.checked_i64_cast_target()?;Self::scalar(&value)?;
        }else if cast{
            let target=self.refinements.iter().find(|(n,_)|matches!(self.p.current,Token::Word(s) if s==n)).map(|(_,r)|*r);
            if let Some(r)=target{self.p.next()?;return self.checked_refinement(value,r);}
            self.attach(&mut value)?;
        }else{self.attach(&mut value)?;}
        Ok(value)
    }
    fn postfix(&mut self,mut value:Typed,mut compact:bool,mut conversion:bool)->Result<Typed,String>{
        loop{
            let cast=self.p.current==Token::Punct(b':')&&self.p.input.as_bytes().get(self.p.pos)==Some(&b':');
            if cast {self.p.expect(Token::Punct(b':'))?;self.p.expect(Token::Punct(b':'))?;}
            else if self.p.take(Token::Punct(b'.'))?{
                if self.p.take(Token::Word("to_unit"))?{
                    if !conversion{return Err("E_UNIT_PROFILE: conversion requires quantity literal".into());}
                    self.p.expect(Token::Punct(b'<'))?;let name=self.p.name()?;
                    self.p.expect(Token::Punct(b'>'))?;self.p.expect(Token::Punct(b'('))?;self.p.expect(Token::Punct(b')'))?;
                    let (_,index,scale)=self.units.iter().find(|(n,_,_)|n==&name).ok_or("E_UNIT_NAME: unknown conversion target")?;
                    let mut dim=SCALAR;dim[*index]=1;
                    if value.dim!=dim{return Err("E_UNIT_DIMENSION: conversion dimensions differ".into());}
                    let Expr::Literal(n)=value.expr else{return Err("E_UNIT_PROFILE: conversion requires literal".into())};
                    let converted=value.scale.convert(n,*scale)?;
                    value.expr=Expr::Literal(converted);value.scale=*scale;value.range=Range::exact(converted);
                    conversion=false;compact=false;continue;
                }
            }else if !(compact&&self.p.current==Token::Punct(b'<')){break;}
            self.p.expect(Token::Punct(b'<'))?;
            value=self.target(value,cast)?;
            self.p.expect(Token::Punct(b'>'))?;
            compact=false;
        }
        Ok(value)
    }
    fn atom(&mut self,depth:usize)->Result<Typed,String>{
        if depth>MAX_DEPTH {return Err("expression depth budget exceeded".into());}
        if self.p.take(Token::Punct(b'<'))? {
            let target=self.refinements.iter().find(|(n,_)|matches!(self.p.current,Token::Word(s) if s==n)).map(|(_,r)|*r);
            if let Some(r)=target{
                self.p.next()?;self.p.expect(Token::Punct(b'>'))?;
                let value=self.atom(depth+1)?;return self.checked_refinement(value,r);
            }
            self.p.checked_i64_cast_target()?;
            self.p.expect(Token::Punct(b'>'))?;
            let value=self.atom(depth+1)?;Self::scalar(&value)?;return Ok(value);
        }
        if self.p.take(Token::Punct(b'+'))? {return self.atom(depth+1);}
        if self.p.take(Token::Punct(b'-'))? {
            if let Token::Number(n)=self.p.current {
                if integer_digits(n)?=="9223372036854775808" {
                    let end=self.p.pos;
                    self.p.next()?;
                    let compact=self.p.pos==end+1;
                    let expr=self.p.node(Expr::Literal(i64::MIN))?;
                    return self.postfix(Typed{expr,dim:SCALAR,scale:Scale::ONE,range:Range::exact(i64::MIN)},compact,false);
                }
            }
            let value=self.atom(depth+1)?;
            return Ok(Typed{expr:self.p.node(Expr::Neg(Box::new(value.expr)))?,dim:value.dim,scale:value.scale,range:value.range.neg()});
        }
        if self.p.take(Token::Punct(b'('))? {
            let value=self.expr(0,depth+1)?;
            self.p.expect(Token::Punct(b')'))?;
            return self.postfix(value,false,false);
        }
        if let Token::Number(n)=self.p.current {
            let v=integer_digits(n)?.parse::<i64>().map_err(|_|"i64 literal out of range")?;
            let end=self.p.pos;
            self.p.next()?;
            let compact=self.p.pos==end+1;
            let expr=self.p.node(Expr::Literal(v))?;
            return self.postfix(Typed{expr,dim:SCALAR,scale:Scale::ONE,range:Range::exact(v)},compact,true);
        }
        if let Token::Word(name)=self.p.current{
            if let Some((i,(_,b)))=self.bindings.iter().enumerate().find(|(_, (n,_))|n==name){
                let b=*b;self.p.next()?;
                let expr=self.p.node(Expr::Local(i))?;
                return self.postfix(Typed{expr,dim:b.dim,scale:b.scale,range:b.range},false,false);
            }
        }
        Err("E_UNIT_PROFILE: expected quantity expression or earlier immutable binding".into())
    }
    fn annotation(&self,name:&str)->Result<Refinement,String>{
        if matches!(name,"int"|"i64"){return Ok(Refinement{dim:SCALAR,scale:Scale::ONE,lo:i64::MIN,hi:i64::MAX});}
        self.refinements.iter().find(|(n,_)|n==name).map(|(_,r)|*r).ok_or_else(||"E_UNIT_NAME: unknown annotation".into())
    }
    fn binding_name(&self,name:&str)->Result<(),String>{
        if matches!(name,"val"|"let"|"int"|"i64"|"i32"|"bool"|"float"|"string"|"void"|"return"|"self"|"main"|"fnc"|"pub"|"define"|"type"|"where"|"as"|"and"|"units"|"in"|"true"|"false"|"if"|"else"|"end"|"begin"|"while"|"loop"|"task"|"await"|"suspends"|"infer"|"new"|"match"|"const"|"mut"|"_" ) ||
           self.dimensions.iter().any(|n|n==name)||self.units.iter().any(|(n,_,_)|n==name)||self.refinements.iter().any(|(n,_)|n==name)||self.bindings.iter().any(|(n,_)|n==name){
            return Err("E_UNIT_BINDING: reserved or duplicate binding".into());
        }
        Ok(())
    }
    fn bindings(&mut self)->Result<Vec<Expr>,String>{
        let mut locals=Vec::new();
        while self.p.take(Token::Word("val"))?{
            if locals.len()==64{return Err("E_UNIT_LIMIT: local bindings".into());}
            let first=self.p.name()?;
            let (name,want)=if self.p.take(Token::Punct(b':'))?{
                let ty=self.p.name()?;(first,Some(self.annotation(&ty)?))
            }else if self.p.current==Token::Punct(b'='){
                (first,None)
            }else{
                let want=self.annotation(&first)?;(self.p.name()?,Some(want))
            };
            self.binding_name(&name)?;
            self.p.expect(Token::Punct(b'='))?;
            let value=self.expr(0,0)?;
            self.p.expect(Token::Punct(b';'))?;
            if let Some(w)=want{
                if value.dim!=w.dim||value.scale!=w.scale{return Err("E_UNIT_BINDING: incompatible unit or scale".into());}
                if !value.range.contained(w.lo,w.hi){return Err("E_REFINEMENT_BINDING: unproved refinement".into());}
            }
            validate_ast_depth(&value.expr)?;
            self.bindings.push((name,Binding{dim:value.dim,scale:value.scale,range:value.range}));
            locals.push(value.expr);
        }
        Ok(locals)
    }
    fn expr(&mut self,min_bp:u8,depth:usize)->Result<Typed,String>{
        if depth>MAX_DEPTH {return Err("expression depth budget exceeded".into());}
        let mut lhs=self.atom(depth+1)?;
        loop{
            let (l,r,op)=match self.p.current{
                Token::Punct(b'+')=>(1,2,b'+'),
                Token::Punct(b'-')=>(1,2,b'-'),
                Token::Punct(b'*')=>(3,4,b'*'),
                Token::Punct(b'/')=>(3,4,b'/'),
                _=>break
            };
            if l<min_bp {break;}
            self.p.next()?;
            let rhs=self.expr(r,depth+1)?;
            let mut dim=lhs.dim;let mut scale=lhs.scale;
            if op==b'+'||op==b'-' {
                if dim!=rhs.dim {return Err("E_UNIT_DIMENSION: incompatible addition/subtraction".into());}
                if scale!=rhs.scale{return Err("E_UNIT_SCALE: mixed scales".into());}
            }else{
                // Do not silently change ordinary scalar division typing.
                if op==b'/'&&dim==SCALAR&&rhs.dim==SCALAR&&scale==Scale::ONE&&rhs.scale==Scale::ONE {return Err("E_UNIT_PROFILE: ordinary scalar division not admitted".into());}
                scale=scale.mul(if op==b'/'{rhs.scale.inv()}else{rhs.scale})?;
                for (i,d) in dim.iter_mut().enumerate(){
                    let n=if op==b'*'{*d+rhs.dim[i]}else{*d-rhs.dim[i]};
                    if !(-32..=32).contains(&n){return Err("E_UNIT_LIMIT: exponent".into());}
                    *d=n;
                }
            }
            let range=lhs.range.binary(rhs.range,op);
            let left=Box::new(lhs.expr);let right=Box::new(rhs.expr);
            let expr=self.p.node(match op{b'+'=>Expr::Add(left,right),b'-'=>Expr::Sub(left,right),b'*'=>Expr::Mul(left,right),b'/'=>Expr::QuantityDiv(left,right),_=>unreachable!()})?;
            lhs=Typed{expr,dim,scale,range};
        }
        // Quantity casts/ascriptions have lower precedence than arithmetic.
        // Parenthesize attachment when used as a subsequent operand.
        if min_bp==0 && self.p.take(Token::Word("as"))? {
            lhs=self.target(lhs,true)?;
        }
        Ok(lhs)
    }
}
pub(super) fn parse(p:Parser<'_>)->Result<Program,String>{
    let mut q=Quantities{p,dimensions:Vec::new(),units:Vec::new(),refinements:Vec::new(),bindings:Vec::new()};
    q.declarations()?;q.refinement_declarations()?;
    q.p.take(Token::Word("pub"))?;
    q.p.expect(Token::Word("fnc"))?;
    q.p.expect(Token::Word("main"))?;
    q.p.expect(Token::Punct(b'('))?;
    q.p.expect(Token::Punct(b')'))?;
    if !q.p.take(Token::Punct(b':'))? {q.p.expect(Token::Punct(b'-'))?;q.p.expect(Token::Punct(b'>'))?;}
    q.p.expect(Token::Word("int"))?;
    q.p.expect(Token::Punct(b'{'))?;
    let locals=q.bindings()?;
    q.p.expect(Token::Word("return"))?;
    let value=q.expr(0,0)?;
    Quantities::scalar(&value)?;
    q.p.expect(Token::Punct(b';'))?;
    q.p.expect(Token::Punct(b'}'))?;
    q.p.expect(Token::End)?;
    validate_ast_depth(&value.expr)?;
    Ok(Program{parameters:Vec::new(),locals,result:value.expr,trace_steps:Vec::new()})
}
