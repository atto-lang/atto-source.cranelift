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
struct Typed { expr: Expr, dim: Dim, scale:Scale }
struct Quantities<'a> {
    p: Parser<'a>,
    dimensions: Vec<String>,
    units: Vec<(String, usize, Scale)>,
}
impl<'a> Quantities<'a> {
    fn declaration_name(&mut self) -> Result<String,String> {
        let name=self.p.name()?;
        if !name.as_bytes().first().is_some_and(u8::is_ascii_uppercase)
            || self.dimensions.contains(&name) || self.units.iter().any(|(n,_,_)|n==&name) {
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
    fn target(&mut self,value:&mut Typed,cast:bool)->Result<(),String>{
        if cast && matches!(self.p.current,Token::Word("int"|"i64")) {
            self.p.checked_i64_cast_target()?;
            Self::scalar(value)
        } else {self.attach(value)}
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
                    value.expr=Expr::Literal(value.scale.convert(n,*scale)?);value.scale=*scale;
                    conversion=false;compact=false;continue;
                }
            }else if !(compact&&self.p.current==Token::Punct(b'<')){break;}
            self.p.expect(Token::Punct(b'<'))?;
            self.target(&mut value,cast)?;
            self.p.expect(Token::Punct(b'>'))?;
            compact=false;
        }
        Ok(value)
    }
    fn atom(&mut self,depth:usize)->Result<Typed,String>{
        if depth>MAX_DEPTH {return Err("expression depth budget exceeded".into());}
        if self.p.take(Token::Punct(b'<'))? {
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
                    return self.postfix(Typed{expr,dim:SCALAR,scale:Scale::ONE},compact,false);
                }
            }
            let value=self.atom(depth+1)?;
            return Ok(Typed{expr:self.p.node(Expr::Neg(Box::new(value.expr)))?,dim:value.dim,scale:value.scale});
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
            return self.postfix(Typed{expr,dim:SCALAR,scale:Scale::ONE},compact,true);
        }
        Err("E_UNIT_PROFILE: expected closed i64 quantity expression".into())
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
            let left=Box::new(lhs.expr);let right=Box::new(rhs.expr);
            let expr=self.p.node(match op{b'+'=>Expr::Add(left,right),b'-'=>Expr::Sub(left,right),b'*'=>Expr::Mul(left,right),b'/'=>Expr::QuantityDiv(left,right),_=>unreachable!()})?;
            lhs=Typed{expr,dim,scale};
        }
        // Quantity casts/ascriptions have lower precedence than arithmetic.
        // Parenthesize attachment when used as a subsequent operand.
        if min_bp==0 && self.p.take(Token::Word("as"))? {
            self.target(&mut lhs,true)?;
        }
        Ok(lhs)
    }
}
pub(super) fn parse(p:Parser<'_>)->Result<Program,String>{
    let mut q=Quantities{p,dimensions:Vec::new(),units:Vec::new()};
    q.declarations()?;
    q.p.take(Token::Word("pub"))?;
    q.p.expect(Token::Word("fnc"))?;
    q.p.expect(Token::Word("main"))?;
    q.p.expect(Token::Punct(b'('))?;
    q.p.expect(Token::Punct(b')'))?;
    if !q.p.take(Token::Punct(b':'))? {q.p.expect(Token::Punct(b'-'))?;q.p.expect(Token::Punct(b'>'))?;}
    q.p.expect(Token::Word("int"))?;
    q.p.expect(Token::Punct(b'{'))?;
    q.p.expect(Token::Word("return"))?;
    let value=q.expr(0,0)?;
    Quantities::scalar(&value)?;
    q.p.expect(Token::Punct(b';'))?;
    q.p.expect(Token::Punct(b'}'))?;
    q.p.expect(Token::End)?;
    validate_ast_depth(&value.expr)?;
    Ok(Program{parameters:Vec::new(),locals:Vec::new(),result:value.expr,trace_steps:Vec::new()})
}
