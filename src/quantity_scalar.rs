//! Bounded scale-one quantity admission on the existing stage-0 lexer and native Expr IR.
//! Dimensions are erased only after the entire closed expression is type checked.
use super::{Expr, Parser, Program, Token, MAX_DEPTH, integer_digits, validate_ast_depth};
type Dim = [i8; 8];
const SCALAR: Dim = [0; 8];
struct Typed { expr: Expr, dim: Dim }
struct Quantities<'a> {
    p: Parser<'a>,
    dimensions: Vec<String>,
    units: Vec<(String, usize)>,
}
impl<'a> Quantities<'a> {
    fn declaration_name(&mut self) -> Result<String,String> {
        let name=self.p.name()?;
        if !name.as_bytes().first().is_some_and(u8::is_ascii_uppercase)
            || self.dimensions.contains(&name) || self.units.iter().any(|(n,_)|n==&name) {
            return Err("E_UNIT_DECLARATION: invalid or duplicate nominal name".into());
        }
        Ok(name)
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
                if self.p.take(Token::Punct(b'='))? {
                    let base=self.p.name()?;
                    if !self.units.iter().any(|(n,i)|n==&base && *i==index) {
                        return Err("E_UNIT_DECLARATION: unknown or incompatible alias".into());
                    }
                } else if self.units.iter().any(|(_,i)|*i==index) {
                    return Err("E_UNIT_DECLARATION: duplicate base unit".into());
                }
                self.units.push((name,index));
            }
            self.p.expect(Token::Punct(b';'))?;
        }
        Ok(())
    }
    fn attach(&mut self, value:&mut Typed)->Result<(),String>{
        if value.dim!=SCALAR {return Err("E_UNIT_ATTACHMENT: already dimensioned".into());}
        let name=self.p.name()?;
        let index=self.units.iter().find(|(n,_)|n==&name).map(|(_,i)|*i)
            .ok_or("E_UNIT_ATTACHMENT: unknown or unsupported unit")?;
        value.dim[index]=1;
        Ok(())
    }
    fn scalar(value:&Typed)->Result<(),String>{
        if value.dim!=SCALAR {return Err("E_UNIT_ERASURE: scalar cast or return would erase units".into());}
        Ok(())
    }
    fn target(&mut self,value:&mut Typed,cast:bool)->Result<(),String>{
        if cast && matches!(self.p.current,Token::Word("int"|"i64")) {
            self.p.checked_i64_cast_target()?;
            Self::scalar(value)
        } else {self.attach(value)}
    }
    fn postfix(&mut self,mut value:Typed,mut compact:bool)->Result<Typed,String>{
        loop{
            let cast=self.p.current==Token::Punct(b':')&&self.p.input.as_bytes().get(self.p.pos)==Some(&b':');
            if cast {self.p.expect(Token::Punct(b':'))?;self.p.expect(Token::Punct(b':'))?;}
            else if !self.p.take(Token::Punct(b'.'))?&&!(compact&&self.p.current==Token::Punct(b'<')){break;}
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
                    return self.postfix(Typed{expr,dim:SCALAR},compact);
                }
            }
            let value=self.atom(depth+1)?;
            return Ok(Typed{expr:self.p.node(Expr::Neg(Box::new(value.expr)))?,dim:value.dim});
        }
        if self.p.take(Token::Punct(b'('))? {
            let value=self.expr(0,depth+1)?;
            self.p.expect(Token::Punct(b')'))?;
            return self.postfix(value,false);
        }
        if let Token::Number(n)=self.p.current {
            let v=integer_digits(n)?.parse::<i64>().map_err(|_|"i64 literal out of range")?;
            let end=self.p.pos;
            self.p.next()?;
            let compact=self.p.pos==end+1;
            let expr=self.p.node(Expr::Literal(v))?;
            return self.postfix(Typed{expr,dim:SCALAR},compact);
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
            let mut dim=lhs.dim;
            if op==b'+'||op==b'-' {
                if dim!=rhs.dim {return Err("E_UNIT_DIMENSION: incompatible addition/subtraction".into());}
            }else{
                // Do not silently change ordinary scalar division typing.
                if op==b'/'&&dim==SCALAR&&rhs.dim==SCALAR {return Err("E_UNIT_PROFILE: ordinary scalar division not admitted".into());}
                for (i,d) in dim.iter_mut().enumerate(){
                    let n=if op==b'*'{*d+rhs.dim[i]}else{*d-rhs.dim[i]};
                    if !(-32..=32).contains(&n){return Err("E_UNIT_LIMIT: exponent".into());}
                    *d=n;
                }
            }
            let left=Box::new(lhs.expr);let right=Box::new(rhs.expr);
            let expr=self.p.node(match op{b'+'=>Expr::Add(left,right),b'-'=>Expr::Sub(left,right),b'*'=>Expr::Mul(left,right),b'/'=>Expr::QuantityDiv(left,right),_=>unreachable!()})?;
            lhs=Typed{expr,dim};
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
