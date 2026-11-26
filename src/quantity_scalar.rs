//! Bounded rational-scale quantity admission on the existing stage-0 lexer and native Expr IR.
//! Dimensions are erased only after the entire closed expression is type checked.
use super::{Expr, Parser, Program, Token, MAX_DEPTH, integer_digits, validate_ast_depth};
type Dim = [i8; 8];
const SCALAR: Dim = [0; 8];
use super::{QuantityScale as Scale, ScaleUInt};
#[derive(Clone,Copy)]
struct Range {lo:i64,hi:i64,holes:u8}
impl Range {
    const FULL:Self=Self{lo:i64::MIN,hi:i64::MAX,holes:0};
    fn exact(v:i64)->Self{Self{lo:v,hi:v,holes:0}}
    fn contained(self,lo:i64,hi:i64)->bool{self.lo>=lo&&self.hi<=hi}
    fn bit(n:i64)->u8{match n{0=>1,-1=>2,_=>0}}
    fn has(self,n:i64)->bool{self.lo<=n&&n<=self.hi&&self.holes&Self::bit(n)==0}
    fn normalize(mut self)->Option<Self>{
        while self.lo<=self.hi&&self.holes&Self::bit(self.lo)!=0{self.lo+=1;}
        while self.lo<=self.hi&&self.holes&Self::bit(self.hi)!=0{self.hi-=1;}
        if self.lo>self.hi{None}else{Some(self)}
    }
    fn restrict(self,op:u8,n:i64)->Self{
        let mut r=self;
        match op{
            0=>{r.lo=r.lo.max(n);r.hi=r.hi.min(n);}
            1=>{if !r.has(n){return r;}if r.lo==r.hi{return self;}
                if r.lo==n{r.lo+=1;}else if r.hi==n{r.hi-=1;}else{r.holes|=Self::bit(n);}}
            2=>{let Some(v)=n.checked_sub(1) else{return self};r.hi=r.hi.min(v);}
            3=>r.hi=r.hi.min(n),
            4=>{let Some(v)=n.checked_add(1) else{return self};r.lo=r.lo.max(v);}
            5=>r.lo=r.lo.max(n),
            _=>return self
        }
        r.normalize().unwrap_or(self)
    }
    fn join(self,r:Self)->Self{
        let holes=(if !self.has(0)&&!r.has(0){1}else{0})|(if !self.has(-1)&&!r.has(-1){2}else{0});
        Self{lo:self.lo.min(r.lo),hi:self.hi.max(r.hi),holes}
    }
    fn bounds(lo:i128,hi:i128)->Self{
        let lo=lo.max(i64::MIN as i128);let hi=hi.min(i64::MAX as i128);
        if lo>hi{Self::FULL}else{Self{lo:lo as i64,hi:hi as i64,holes:0}}
    }
    fn neg(self)->Self{Self::bounds(-(self.hi as i128),-(self.lo as i128))}
    fn proves(self,rhs:Self,op:u8)->bool{
        let(a,b,c,d)=(self.lo as i128,self.hi as i128,rhs.lo as i128,rhs.hi as i128);
        let fits=|lo:i128,hi:i128|lo>=i64::MIN as i128&&hi<=i64::MAX as i128;
        match op{
            b'+'=>fits(a+c,b+d),b'-'=>fits(a-d,b-c),
            b'*'=>{let v=[a*c,a*d,b*c,b*d];fits(*v.iter().min().unwrap(),*v.iter().max().unwrap())},
            b'/'=> !rhs.has(0)&&!(self.has(i64::MIN)&&rhs.has(-1)),
            _=>false
        }
    }
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
#[derive(Clone,Copy,PartialEq,Eq)]
enum Width { Literal, I32, I64 }
const I32_LO:i64=i32::MIN as i64;
const I32_HI:i64=i32::MAX as i64;
impl Range {
    fn intersect(self,other:Self)->Self {
        let lo=self.lo.max(other.lo);let hi=self.hi.min(other.hi);
        // Disjoint checked cast has no normal successor. Retain target
        // bounds, never claim an impossible pre-cast value survived.
        if lo>hi{other}else{Self{lo,hi,holes:self.holes|other.holes}}
    }
}
struct Typed { expr: Expr, dim: Dim, scale:Scale, range:Range, width:Width }
#[derive(Clone,Copy)]
struct Binding {dim:Dim,scale:Scale,range:Range,slot:usize,width:Width}
#[derive(Clone,Copy)]
struct Refinement {dim:Dim,scale:Scale,lo:i64,hi:i64,width:Width}
#[derive(Clone)]
struct Contract {op:u8, parameter:Option<usize>, bound:Option<i64>}
#[derive(Clone)]
struct Helper {params:Vec<Refinement>,result:Refinement,locals:Vec<Expr>,body:Expr}
fn remap(expr:&Expr,slots:&[usize])->Result<Expr,String>{
    let one=|e:&Expr|remap(e,slots).map(Box::new);
    Ok(match expr{
        Expr::Literal(n)=>Expr::Literal(*n),
        Expr::Local(i)=>Expr::Local(*slots.get(*i).ok_or("E_UNIT_CALL: invalid local reference")?),
        Expr::Neg(x)=>Expr::Neg(one(x)?),
        Expr::Add(a,b)=>Expr::Add(one(a)?,one(b)?),
        Expr::Sub(a,b)=>Expr::Sub(one(a)?,one(b)?),
        Expr::Mul(a,b)=>Expr::Mul(one(a)?,one(b)?),
        Expr::Div(a,b)=>Expr::Div(one(a)?,one(b)?),
        Expr::Compare(op,a,b)=>Expr::Compare(*op,one(a)?,one(b)?),
        Expr::Choose(c,y,n)=>Expr::Choose(one(c)?,one(y)?,one(n)?),
        // Only call-site arguments refer to the outer helper's slots.
        Expr::ScopedCall(args,locals,body)=>Expr::ScopedCall(args.iter().map(|a|remap(a,slots)).collect::<Result<Vec<_>,_>>()?,locals.clone(),body.clone()),
        Expr::I32Checked(x)=>Expr::I32Checked(one(x)?),
        Expr::RefinementCast(x,l,h)=>Expr::RefinementCast(one(x)?,*l,*h),
        _=>return Err("E_UNIT_CALL: unsupported helper expression".into())
    })
}
struct Quantities<'a> {
    p: Parser<'a>,
    dimensions: Vec<String>,
    units: Vec<(String, usize, Scale)>,
    refinements: Vec<(String,Refinement)>,
    bindings: Vec<(String,Binding)>,
    strict: bool,
    guarded: bool,
    locals:Vec<Expr>,
    helpers:Vec<(String,Helper)>,
}
impl<'a> Quantities<'a> {
    fn declaration_name(&mut self) -> Result<String,String> {
        let name=self.p.name()?;
        if self.dimensions.contains(&name)||self.units.iter().any(|(n,_,_)|n==&name)||self.refinements.iter().any(|(n,_)|n==&name){return Err("E_UNIT_DUPLICATE: duplicate nominal name".into());}
        if !name.as_bytes().first().is_some_and(u8::is_ascii_uppercase){return Err("E_UNIT_NAME: invalid nominal name".into());}
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
                    .ok_or("E_UNIT_NAME: unknown dimension")?;
                let scale=if self.p.take(Token::Punct(b'='))?{
                    let (base,factor)=if matches!(self.p.current,Token::Number(_)){
                        let n=self.factor()?;self.p.expect(Token::Punct(b'*'))?;(self.p.name()?,Scale(ScaleUInt::from_u64(n as u64),ScaleUInt::ONE))
                    }else{
                        let base=self.p.name()?;
                        let factor=if self.p.take(Token::Punct(b'*'))?{Scale(ScaleUInt::from_u64(self.factor()? as u64),ScaleUInt::ONE)}
                        else if self.p.take(Token::Punct(b'/'))?{Scale(ScaleUInt::ONE,ScaleUInt::from_u64(self.factor()? as u64))}else{Scale::ONE};
                        (base,factor)
                    };
                    let (_,base_dim,scale)=self.units.iter().find(|(n,_,_)|n==&base).ok_or("E_UNIT_NAME: unknown alias")?;
                    if *base_dim!=index{return Err("E_UNIT_DIMENSION: incompatible alias".into());}
                    scale.mul(factor)?
                }else{
                    if self.units.iter().any(|(_,i,_)|*i==index){return Err("E_UNIT_DUPLICATE: duplicate base unit".into());}
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
            if matches!(self.p.current,Token::Word("int"|"i64"|"i32")){return Err("E_REFINEMENT_DUPLICATE: reserved carrier".into());}
            let name=self.declaration_name()?;
            self.p.expect(Token::Punct(b'='))?;
            let width=if self.p.take(Token::Word("i32"))?{Width::I32}else{if !self.p.take(Token::Word("int"))?{self.p.expect(Token::Word("i64"))?;}Width::I64};
            self.p.expect(Token::Word("where"))?;
            let(mut lo,mut hi)=if width==Width::I32{(I32_LO,I32_HI)}else{(i64::MIN,i64::MAX)};
            let mut count=0;
            let mut has_units=false;
            loop{
                if self.p.take(Token::Word("units"))?{has_units=true;break;}
                self.p.expect(Token::Word("self"))?;
                if self.p.take(Token::Word("eq"))?{let n=self.signed_bound()?;lo=lo.max(n);hi=hi.min(n);count+=1;if count>16||lo>hi{return Err("E_REFINEMENT_EMPTY: empty interval".into());}if !self.p.take(Token::Word("and"))?{break;}continue;}
                let end=self.p.pos;
                let lower=if self.p.take(Token::Punct(b'>'))?{true}else{self.p.expect(Token::Punct(b'<'))?;false};
                let inclusive=self.p.pos==end+1&&self.p.take(Token::Punct(b'='))?;
                let bound=self.signed_bound()?;
                if lower{let n=if inclusive{bound}else{bound.checked_add(1).ok_or("E_REFINEMENT_EMPTY: empty")?};lo=lo.max(n);}
                else{let n=if inclusive{bound}else{bound.checked_sub(1).ok_or("E_REFINEMENT_EMPTY: empty")?};hi=hi.min(n);}
                count+=1;if count>16||lo>hi{return Err("E_REFINEMENT_EMPTY: empty or excessive predicates".into());}
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
            self.refinements.push((name,Refinement{dim,scale,lo,hi,width}));
        }
        Ok(())
    }
    fn attach(&mut self, value:&mut Typed)->Result<(),String>{
        if value.dim!=SCALAR||value.scale!=Scale::ONE {return Err("E_UNIT_ATTACHMENT: already dimensioned".into());}
        let name=self.p.name()?;
        let (_,index,scale)=self.units.iter().find(|(n,_,_)|n==&name)
            .ok_or("E_UNIT_NAME: unknown or unsupported unit")?;
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
        if self.strict&&!value.range.contained(target.lo,target.hi){return Err("E_VERIFY_UNPROVEN: refinement cast may fail".into());}
        let expr=self.p.node(Expr::RefinementCast(Box::new(value.expr),target.lo,target.hi))?;
        Ok(Typed{expr,dim:value.dim,scale:value.scale,range:value.range.intersect(Range{lo:target.lo,hi:target.hi,holes:0}),width:target.width})
    }
    fn target(&mut self,value:Typed,cast:bool)->Result<Typed,String>{
        let mut value=value;
        if cast && matches!(self.p.current,Token::Word("int"|"i64")) {
            self.p.checked_i64_cast_target()?;Self::scalar(&value)?;value.width=Width::I64;
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
            let mut value=self.atom(depth+1)?;Self::scalar(&value)?;value.width=Width::I64;return Ok(value);
        }
        if self.p.take(Token::Punct(b'+'))? {return self.atom(depth+1);}
        if self.p.take(Token::Punct(b'-'))? {
            if let Token::Number(n)=self.p.current {
                if integer_digits(n)?=="9223372036854775808" {
                    let end=self.p.pos;
                    self.p.next()?;
                    let compact=self.p.pos==end+1;
                    let expr=self.p.node(Expr::Literal(i64::MIN))?;
                    return self.postfix(Typed{expr,dim:SCALAR,scale:Scale::ONE,range:Range::exact(i64::MIN),width:Width::Literal},compact,true);
                }
            }
            let value=self.atom(depth+1)?;
            if self.strict&&value.range.lo==i64::MIN{return Err("E_VERIFY_UNPROVEN: signed negation may overflow".into());}
            let range=value.range.neg();
            let expr=self.p.node(Expr::Neg(Box::new(value.expr)))?;
            let (expr,range)=self.checked_width(expr,range,value.width)?;
            return Ok(Typed{expr,dim:value.dim,scale:value.scale,range,width:value.width});
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
            return self.postfix(Typed{expr,dim:SCALAR,scale:Scale::ONE,range:Range::exact(v),width:Width::Literal},compact,true);
        }
        if let Token::Word(name)=self.p.current{
            if let Some((_,b))=self.bindings.iter().find(|(n,_)|n==name){
                let b=*b;self.p.next()?;
                let expr=self.p.node(Expr::Local(b.slot))?;
                return self.postfix(Typed{expr,dim:b.dim,scale:b.scale,range:b.range,width:b.width},false,false);
            }
        }
        if let Token::Word(name)=self.p.current{
            if let Some((_,helper))=self.helpers.iter().find(|(n,_)|n==name){
                let helper=helper.clone();self.p.next()?;
                return self.call(helper,depth+1);
            }
        }
        if let Token::Word(_)=self.p.current{if self.p.input.as_bytes().get(self.p.pos..).is_some_and(|tail|tail.iter().copied().find(|c|!c.is_ascii_whitespace())==Some(b'(')){return Err("E_REFINEMENT_CALL: unknown or recursive helper".into());}}
        Err("E_UNIT_PROFILE: expected quantity expression, binding or earlier private helper".into())
    }
    fn checked_width(&mut self,expr:Expr,range:Range,width:Width)->Result<(Expr,Range),String>{
        if width!=Width::I32{return Ok((expr,range));}
        if self.strict&&!range.contained(I32_LO,I32_HI){return Err("E_VERIFY_UNPROVEN: i32 arithmetic may overflow".into());}
        Ok((self.p.node(Expr::I32Checked(Box::new(expr)))?,range.intersect(Range{lo:I32_LO,hi:I32_HI,holes:0})))
    }
    fn push_local(&mut self,expr:Expr)->Result<usize,String>{
        if self.locals.len()>=64{return Err("E_UNIT_LIMIT: runtime locals".into());}
        validate_ast_depth(&expr)?;
        let slot=self.locals.len();self.locals.push(expr);Ok(slot)
    }
    fn materialize(&mut self,mut value:Typed)->Result<Typed,String>{
        let slot=self.push_local(value.expr)?;
        value.expr=self.p.node(Expr::Local(slot))?;Ok(value)
    }
    fn boundary(want:Refinement,value:&Typed,code:&str)->Result<(),String>{
        if want.dim!=value.dim{return Err("E_UNIT_DIMENSION: incompatible boundary".into());}
        if want.scale!=value.scale{return Err("E_UNIT_SCALE: incompatible boundary".into());}
        if !value.range.contained(want.lo,want.hi){return Err(format!("{code}: unproved refinement"));}
        Ok(())
    }
    // Eager arguments and helper locals get fresh slots; never substitute an
    // effectful expression at multiple uses of a parameter.
    fn call(&mut self,h:Helper,depth:usize)->Result<Typed,String>{
        if self.guarded {
            self.p.expect(Token::Punct(b'('))?;
            let mut args=Vec::new();
            for (i,want) in h.params.iter().enumerate(){
                if i>0{self.p.expect(Token::Punct(b','))?;}
                if self.p.current==Token::Punct(b')'){return Err("E_REFINEMENT_CALL: missing argument".into());}
                let value=self.expr(0,depth+1)?;
                Self::boundary(*want,&value,"E_REFINEMENT_PRECONDITION")?;
                args.push(value.expr);
            }
            self.p.expect(Token::Punct(b')'))?;
            let inits=h.locals.into_iter().skip(h.params.len()).collect::<Vec<_>>();
            let expr=self.p.node(Expr::ScopedCall(args,inits,Box::new(h.body)))?;
            validate_ast_depth(&expr)?;
            return self.postfix(Typed{expr,dim:h.result.dim,scale:h.result.scale,range:Range{lo:h.result.lo,hi:h.result.hi,holes:0},width:h.result.width},false,false);
        }
        self.p.expect(Token::Punct(b'('))?;
        let mut slots=Vec::new();
        for (i,want) in h.params.iter().enumerate(){
            if i>0{self.p.expect(Token::Punct(b','))?;}
            if self.p.current==Token::Punct(b')'){return Err("E_REFINEMENT_CALL: missing argument".into());}
            let value=self.expr(0,depth+1)?;
            Self::boundary(*want,&value,"E_REFINEMENT_PRECONDITION")?;
            slots.push(self.push_local(value.expr)?);
        }
        self.p.expect(Token::Punct(b')'))?;
        for init in h.locals.iter().skip(h.params.len()){
            let value=remap(init,&slots)?;
            slots.push(self.push_local(value)?);
        }
        let slot=self.push_local(remap(&h.body,&slots)?)?;
        let expr=self.p.node(Expr::Local(slot))?;
        self.postfix(Typed{expr,dim:h.result.dim,scale:h.result.scale,range:Range{lo:h.result.lo,hi:h.result.hi,holes:0},width:h.result.width},false,false)
    }
    fn annotation(&self,name:&str)->Result<Refinement,String>{
        if matches!(name,"int"|"i64"){return Ok(Refinement{dim:SCALAR,scale:Scale::ONE,lo:i64::MIN,hi:i64::MAX,width:Width::I64});}
        if name=="i32"{return Ok(Refinement{dim:SCALAR,scale:Scale::ONE,lo:I32_LO,hi:I32_HI,width:Width::I32});}
        self.refinements.iter().find(|(n,_)|n==name).map(|(_,r)|*r).ok_or_else(||"E_UNIT_NAME: unknown annotation".into())
    }
    fn binding_name(&self,name:&str)->Result<(),String>{
        if !name.as_bytes().first().is_some_and(u8::is_ascii_lowercase) || matches!(name,"using"|"dimension"|"unit"|"val"|"let"|"int"|"i64"|"i32"|"bool"|"float"|"string"|"void"|"return"|"self"|"main"|"fnc"|"pub"|"define"|"type"|"where"|"as"|"and"|"units"|"in"|"true"|"false"|"if"|"else"|"end"|"begin"|"while"|"loop"|"task"|"await"|"suspends"|"infer"|"new"|"match"|"const"|"mut"|"_" ) ||
           self.dimensions.iter().any(|n|n==name)||self.units.iter().any(|(n,_,_)|n==name)||self.refinements.iter().any(|(n,_)|n==name)||self.bindings.iter().any(|(n,_)|n==name)||self.helpers.iter().any(|(n,_)|n==name){
            return Err("E_UNIT_BINDING: reserved or duplicate binding".into());
        }
        Ok(())
    }
    fn bindings(&mut self)->Result<(),String>{
        let mut mutable_types:Vec<(String,Refinement)>=Vec::new();
        loop {
            let immutable=self.p.take(Token::Word("val"))?;
            let mutable=!immutable&&self.strict&&self.p.take(Token::Word("let"))?;
            if !immutable&&!mutable {
                let assignment=if let Token::Word(n)=self.p.current {
                    let tail=self.p.input.as_bytes().get(self.p.pos..).unwrap_or_default();
                    let tail=tail.iter().copied().skip_while(|c|c.is_ascii_whitespace()).collect::<Vec<_>>();
                    if tail.first()==Some(&b'=')&&tail.get(1)!=Some(&b'='){Some(n.to_owned())}else{None}
                }else{None};
                if !self.strict||assignment.is_none(){break;}
                let name=assignment.unwrap();
                let want=mutable_types.iter().find(|(n,_)|n==&name).map(|(_,w)|*w).ok_or("E_VERIFY_UNSUPPORTED: assignment requires mutable scalar")?;
                let index=self.bindings.iter().position(|(n,_)|n==&name).ok_or("E_VERIFY_UNSUPPORTED: assignment outside scope")?;
                self.p.next()?;self.p.expect(Token::Punct(b'='))?;
                let value=self.expr(0,0)?;self.p.expect(Token::Punct(b';'))?;
                Self::boundary(want,&value,"E_REFINEMENT_BINDING")?;
                let slot=self.push_local(value.expr)?;
                // A new SSA identity replaces the fact; old slots remain available
                // to existing immutable snapshots and ordered eager initializers.
                self.bindings[index].1=Binding{dim:value.dim,scale:value.scale,range:value.range,slot,width:want.width};
                continue;
            }
            if self.locals.len()==64{return Err("E_UNIT_LIMIT: local bindings".into());}
            let first=self.p.name()?;
            let (name,want)=if self.p.take(Token::Punct(b':'))?{
                let ty=self.p.name()?;(first,Some(self.annotation(&ty)?))
            }else if self.p.current==Token::Punct(b'='){
                (first,None)
            }else{
                let want=self.annotation(&first)?;(self.p.name()?,Some(want))
            };
            if mutable&&want.is_none(){return Err("E_VERIFY_UNSUPPORTED: mutable binding requires explicit type".into());}
            self.binding_name(&name)?;
            self.p.expect(Token::Punct(b'='))?;
            let mut value=self.expr(0,0)?;
            self.p.expect(Token::Punct(b';'))?;
            if let Some(w)=want{
                Self::boundary(w,&value,"E_REFINEMENT_BINDING")?;
                value.width=w.width;
            }else if value.width==Width::Literal{value.width=Width::I64;}
            let slot=self.push_local(value.expr)?;
            if mutable{mutable_types.push((name.clone(),want.unwrap()));}
            self.bindings.push((name,Binding{dim:value.dim,scale:value.scale,range:value.range,slot,width:value.width}));
        }
        Ok(())
    }
    fn contract_op(&mut self)->Result<u8,String>{
        if self.p.take(Token::Word("eq"))?{return Ok(0);}
        let Token::Punct(symbol)=self.p.current else{return Err("E_REFINEMENT_CONTRACT: comparison required".into());};
        let paired=self.p.input.as_bytes().get(self.p.pos)==Some(&b'=');
        self.p.next()?;if paired{self.p.expect(Token::Punct(b'='))?;}
        match(symbol,paired){(b'=',true)=>Ok(0),(b'<',false)=>Ok(2),(b'<',true)=>Ok(3),(b'>',false)=>Ok(4),(b'>',true)=>Ok(5),_=>Err("E_REFINEMENT_CONTRACT: unsupported comparison".into())}
    }
    fn contract_bound(&mut self,w:Refinement)->Result<i64,String>{
        let n=self.signed_bound()?;let mut dim=SCALAR;let mut scale=Scale::ONE;
        let dotted=self.p.take(Token::Punct(b'.'))?;
        if dotted||self.p.current==Token::Punct(b'<'){
            self.p.expect(Token::Punct(b'<'))?;let name=self.p.name()?;self.p.expect(Token::Punct(b'>'))?;
            let (_,index,s)=self.units.iter().find(|(u,_,_)|u==&name).ok_or("E_UNIT_NAME: contract unit")?;
            dim[*index]=1;scale=*s;
        }
        if dim!=w.dim{return Err("E_UNIT_DIMENSION: contract operand".into());}
        if scale!=w.scale{return Err("E_UNIT_SCALE: contract operand".into());}
        Ok(n)
    }
    fn contracts(&mut self,params:&mut [Refinement],result:Refinement,main:bool)->Result<Vec<Contract>,String>{
        let mut out=Vec::new();let mut count=0;
        loop{
            let require=self.p.take(Token::Word("requires"))?;
            if !require&&!self.p.take(Token::Word("ensures"))?{break;}
            count+=1;if main||count>16{return Err("E_REFINEMENT_CONTRACT: contract limit or exported entry".into());}
            let name=self.p.name()?;let op=self.contract_op()?;
            if require{
                let i=self.bindings.iter().position(|(n,_)|n==&name).ok_or("E_REFINEMENT_CONTRACT: unknown parameter")?;
                let w=params[i];let n=self.contract_bound(w)?;
                let (mut lo,mut hi)=(w.lo,w.hi);
                match op{0=>{lo=lo.max(n);hi=hi.min(n);},2=>hi=hi.min(n.checked_sub(1).ok_or("E_REFINEMENT_EMPTY: impossible requires")?),3=>hi=hi.min(n),4=>lo=lo.max(n.checked_add(1).ok_or("E_REFINEMENT_EMPTY: impossible requires")?),5=>lo=lo.max(n),_=>unreachable!()}
                if lo>hi{return Err("E_REFINEMENT_EMPTY: contradictory requires".into());}
                params[i].lo=lo;params[i].hi=hi;self.bindings[i].1.range=Range{lo,hi,holes:0};
            }else{
                if name!="result"{return Err("E_REFINEMENT_CONTRACT: ensures must constrain result".into());}
                if let Token::Word(p)=self.p.current{
                    let i=self.bindings.iter().position(|(n,_)|n==p).ok_or("E_REFINEMENT_CONTRACT: unknown relational parameter")?;
                    if params[i].dim!=result.dim{return Err("E_UNIT_DIMENSION: ensures operand".into());}
                    if params[i].scale!=result.scale{return Err("E_UNIT_SCALE: ensures operand".into());}
                    self.p.next()?;out.push(Contract{op,parameter:Some(i),bound:None});
                }else{let n=self.contract_bound(result)?;out.push(Contract{op,parameter:None,bound:Some(n)});}
            }
        }Ok(out)
    }
    // Prove a relation to the original parameter, not a call-site sample.
    // Only identity, checked casts and positive division of an already
    // nonnegative parameter are admitted. Every recursive edge decreases
    // depth; local references stop at parameter slots.
    fn relation(&self,e:&Expr,param:usize,param_count:usize,nonnegative:bool,depth:usize)->bool{
        if depth>=64{return false;}
        match e{
            Expr::Local(i) if *i<param_count=>*i==param,
            Expr::Local(i)=>self.locals.get(*i).is_some_and(|v|self.relation(v,param,param_count,nonnegative,depth+1)),
            Expr::RefinementCast(v,_,_)|Expr::I32Checked(v)=>self.relation(v,param,param_count,nonnegative,depth+1),
            Expr::Div(a,b) if nonnegative=>self.relation(a,param,param_count,nonnegative,depth+1)&&self.positive_constant(b,param_count,depth+1),
            _=>false
        }
    }
    fn positive_constant(&self,e:&Expr,param_count:usize,depth:usize)->bool{
        if depth>=64{return false;}
        match e{Expr::Literal(n)=>*n>=1,Expr::Local(i) if *i>=param_count=>self.locals.get(*i).is_some_and(|v|self.positive_constant(v,param_count,depth+1)),_=>false}
    }
    fn prove_contracts(&self,contracts:&[Contract],value:&Typed,params:&[Refinement])->Result<(),String>{
        for c in contracts{
            let proven=if let Some(n)=c.bound{match c.op{0=>value.range.lo==n&&value.range.hi==n,2=>value.range.hi<n,3=>value.range.hi<=n,4=>value.range.lo>n,5=>value.range.lo>=n,_=>false}}
            else if let Some(i)=c.parameter{
                let p=params[i];
                match c.op{3=>value.range.hi<=p.lo||self.relation(&value.expr,i,params.len(),p.lo>=0,0),2=>value.range.hi<p.lo,4=>value.range.lo>p.hi,5=>value.range.lo>=p.hi,0=>value.range.lo==value.range.hi&&p.lo==p.hi&&p.lo==value.range.lo,_=>false}
            }else{false};
            if !proven{return Err("E_REFINEMENT_POSTCONDITION: unproved ensures".into());}
        }Ok(())
    }
    fn pair(&mut self,b:u8)->Result<bool,String>{
        if self.p.current!=Token::Punct(b)||self.p.input.as_bytes().get(self.p.pos)!=Some(&b){return Ok(false);}
        self.p.next()?;self.p.expect(Token::Punct(b))?;Ok(true)
    }
    fn join_facts(a:&[(String,Binding)],b:&[(String,Binding)])->Vec<(String,Binding)>{
        a.iter().zip(b).map(|((name,x),(_,y))|{let mut v=*x;v.range=x.range.join(y.range);(name.clone(),v)}).collect()
    }
    fn predicate(&mut self, incoming:&[(String,Binding)],depth:usize)->Result<(Expr,Vec<(String,Binding)>,Vec<(String,Binding)>),String>{
        if depth>16{return Err("E_UNIT_LIMIT: guard depth".into());}
        self.bindings=incoming.to_vec();
        if self.p.take(Token::Punct(b'('))?{let p=self.guard_or(incoming,depth+1)?;self.p.expect(Token::Punct(b')'))?;return Ok(p);}
        let left=self.expr(0,0)?;
        let Token::Punct(symbol)=self.p.current else{return Err("E_VERIFY_UNSUPPORTED: expected comparison".into());};
        let paired=self.p.input.as_bytes().get(self.p.pos)==Some(&b'=');
        self.p.next()?;
        if paired{self.p.expect(Token::Punct(b'='))?;}
        let op=match (symbol,paired){(b'=',true)=>0,(b'!',true)=>1,(b'<',false)=>2,(b'<',true)=>3,(b'>',false)=>4,(b'>',true)=>5,_=>return Err("E_VERIFY_UNSUPPORTED: expected comparison".into())};
        let right=self.expr(0,0)?;
        if left.dim!=right.dim{return Err("E_UNIT_DIMENSION: incompatible guard operands".into());}
        if left.scale!=right.scale{return Err("E_UNIT_SCALE: incompatible guard operands".into());}
        let mut yes=incoming.to_vec();let mut no=incoming.to_vec();
        if let Expr::Local(slot)=&left.expr{
            if right.range.lo==right.range.hi{
                if let Some(i)=incoming.iter().position(|(_,b)|b.slot==*slot){
                    let r=incoming[i].1.range;
                    yes[i].1.range=r.restrict(op,right.range.lo);
                    no[i].1.range=r.restrict([1,0,5,4,3,2][op as usize],right.range.lo);
                }
            }
        }
        Ok((self.p.node(Expr::Compare(op,Box::new(left.expr),Box::new(right.expr)))?,yes,no))
    }
    fn guard_and(&mut self, incoming:&[(String,Binding)],depth:usize)->Result<(Expr,Vec<(String,Binding)>,Vec<(String,Binding)>),String>{
        let(mut e,mut yes,mut no)=self.predicate(incoming,depth)?;
        let mut count=0;
        while self.pair(b'&')?{
            count+=1;if count>16{return Err("E_UNIT_LIMIT: conjunction budget".into());}
            let(q,y,n)=self.predicate(&yes,depth)?;
            e=self.p.node(Expr::Choose(Box::new(e),Box::new(q),Box::new(Expr::Literal(0))))?;
            no=Self::join_facts(&no,&n);yes=y;
        }Ok((e,yes,no))
    }
    fn guard_or(&mut self, incoming:&[(String,Binding)],depth:usize)->Result<(Expr,Vec<(String,Binding)>,Vec<(String,Binding)>),String>{
        let(mut e,mut yes,mut no)=self.guard_and(incoming,depth)?;
        let mut count=0;
        while self.pair(b'|')?{
            count+=1;if count>16{return Err("E_UNIT_LIMIT: disjunction budget".into());}
            let(q,y,n)=self.guard_and(&no,depth)?;
            e=self.p.node(Expr::Choose(Box::new(e),Box::new(Expr::Literal(1)),Box::new(q)))?;
            yes=Self::join_facts(&yes,&y);no=n;
        }Ok((e,yes,no))
    }
    fn returning(&mut self,depth:usize)->Result<Typed,String>{
        if depth>16{return Err("E_UNIT_LIMIT: returning guard depth".into());}
        if self.strict&&self.p.take(Token::Word("if"))?{
            let incoming=self.bindings.clone();let old=self.guarded;self.guarded=true;
            let(c,yes,no)=self.guard_or(&incoming,0)?;
            self.p.expect(Token::Punct(b'{'))?;self.bindings=yes;
            let y=self.returning(depth+1)?;self.p.expect(Token::Punct(b'}'))?;
            self.bindings=no;let alt=self.p.take(Token::Word("else"))?;
            if alt{self.p.expect(Token::Punct(b'{'))?;}
            let n=self.returning(depth+1)?;if alt{self.p.expect(Token::Punct(b'}'))?;}
            self.bindings=incoming;self.guarded=old;
            if y.dim!=n.dim||y.scale!=n.scale{return Err("E_UNIT_DIMENSION: incompatible branch return".into());}
            return Ok(Typed{expr:self.p.node(Expr::Choose(Box::new(c),Box::new(y.expr),Box::new(n.expr)))?,dim:y.dim,scale:y.scale,range:y.range.join(n.range),width:if y.width==n.width{y.width}else{Width::I64}});
        }
        self.p.expect(Token::Word("return"))?;let value=self.expr(0,0)?;self.p.expect(Token::Punct(b';'))?;Ok(value)
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
            // A later operand can contain an eager helper expansion. Freeze
            // the left operand before parsing it, preserving failure order.
            if !self.guarded&&!self.helpers.is_empty(){lhs=self.materialize(lhs)?;}
            let rhs=self.expr(r,depth+1)?;
            let mut dim=lhs.dim;let mut scale=lhs.scale;
            if op==b'+'||op==b'-' {
                if dim!=rhs.dim {return Err("E_UNIT_DIMENSION: incompatible addition/subtraction".into());}
                if scale!=rhs.scale{return Err("E_UNIT_SCALE: mixed scales".into());}
            }else{
                // Do not silently change ordinary scalar division typing.
                // Checked scalar division is part of the canonical IR.
                scale=scale.mul(if op==b'/'{rhs.scale.inv()}else{rhs.scale})?;
                for (i,d) in dim.iter_mut().enumerate(){
                    let n=if op==b'*'{*d+rhs.dim[i]}else{*d-rhs.dim[i]};
                    if !(-32..=32).contains(&n){return Err("E_UNIT_LIMIT: exponent".into());}
                    *d=n;
                }
            }
            if self.strict&&!lhs.range.proves(rhs.range,op){return Err("E_VERIFY_UNPROVEN: signed arithmetic may overflow or divide by zero".into());}
            let width=match (lhs.width,rhs.width){
                (Width::I64,_)|(_,Width::I64)=>Width::I64,
                (Width::I32,_)|(_,Width::I32)=>Width::I32,
                _=>Width::I64
            };
            if width==Width::I32 && ((lhs.width==Width::Literal&&!lhs.range.contained(I32_LO,I32_HI))||(rhs.width==Width::Literal&&!rhs.range.contained(I32_LO,I32_HI))){return Err("E_I32_RANGE: literal operand out of range".into());}
            let range=lhs.range.binary(rhs.range,op);
            let left=Box::new(lhs.expr);let right=Box::new(rhs.expr);
            let expr=self.p.node(match op{b'+'=>Expr::Add(left,right),b'-'=>Expr::Sub(left,right),b'*'=>Expr::Mul(left,right),b'/'=>Expr::Div(left,right),_=>unreachable!()})?;
            let (expr,range)=self.checked_width(expr,range,width)?;
            lhs=Typed{expr,dim,scale,range,width};
        }
        // Quantity casts/ascriptions have lower precedence than arithmetic.
        // Parenthesize attachment when used as a subsequent operand.
        while min_bp==0 && self.p.take(Token::Word("as"))? {
            lhs=self.target(lhs,true)?;
        }
        Ok(lhs)
    }
}
pub(super) fn parse_mode(p:Parser<'_>,strict:bool)->Result<Program,String>{
    let mut q=Quantities{p,dimensions:Vec::new(),units:Vec::new(),refinements:Vec::new(),bindings:Vec::new(),strict,guarded:false,locals:Vec::new(),helpers:Vec::new()};
    q.declarations()?;q.refinement_declarations()?;
    if strict&&q.refinements.is_empty(){return Err("E_VERIFY_UNSUPPORTED: strict typed quantity profile required".into());}
    loop{
        q.bindings.clear();q.locals.clear();
        let public=q.p.take(Token::Word("pub"))?;
        q.p.expect(Token::Word("fnc"))?;
        let name=q.p.name()?;
        let main=name=="main";
        if !main{
            if public{return Err("E_REFINEMENT_EXPORT: private helpers only".into());}
            if q.helpers.len()>=16{return Err("E_REFINEMENT_CALL: helper limit".into());}
            q.binding_name(&name)?;
        }
        q.p.expect(Token::Punct(b'('))?;
        let mut params=Vec::new();
        if !q.p.take(Token::Punct(b')'))?{
            loop{
                if main||params.len()>=8{return Err("E_UNIT_LIMIT: parameters".into());}
                let ty=q.p.name()?;let want=q.annotation(&ty)?;
                let param=q.p.name()?;q.binding_name(&param)?;
                let slot=q.push_local(Expr::Literal(0))?;
                q.bindings.push((param,Binding{dim:want.dim,scale:want.scale,range:Range{lo:want.lo,hi:want.hi,holes:0},slot,width:want.width}));
                params.push(want);
                if q.p.take(Token::Punct(b')'))?{break;}
                q.p.expect(Token::Punct(b','))?;
            }
        }
        if !q.p.take(Token::Punct(b':'))?{q.p.expect(Token::Punct(b'-'))?;q.p.expect(Token::Punct(b'>'))?;}
        let ty=q.p.name()?;
        if main&&ty!="int"{return Err("E_UNIT_ERASURE: main requires plain int".into());}
        let result=q.annotation(&ty)?;
        let contracts=q.contracts(&mut params,result,main)?;
        q.p.expect(Token::Punct(b'{'))?;
        q.bindings()?;
        if q.p.current==Token::Word("let"){return Err("E_REFINEMENT_BODY: mutable binding not admitted".into());}
        let value=q.returning(0)?;
        if main{Quantities::scalar(&value)?;}
        Quantities::boundary(result,&value,"E_REFINEMENT_RETURN")?;
        q.prove_contracts(&contracts,&value,&params)?;
        q.p.expect(Token::Punct(b'}'))?;
        validate_ast_depth(&value.expr)?;
        if main{
            q.p.expect(Token::End)?;
            return Ok(Program{parameters:Vec::new(),locals:q.locals,result:value.expr,trace_steps:Vec::new()});
        }
        q.helpers.push((name,Helper{params,result,locals:std::mem::take(&mut q.locals),body:value.expr}));
    }
}
