//! Closed readonly object profile, with typed scalar SSA and a bounded call DAG.
use super::{Parser,Token};
const MARKER: &str = "// atto-ownership: borrow-default-v1";
pub(crate) fn selected(s:&str)->bool { s.lines().next()==Some(MARKER) }
fn err()->String { "E_NATIVE_READONLY: unsupported closed class reference profile".into() }
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub(crate) enum Kind { Ref, Int }
#[derive(Clone,Copy,Debug)]
pub(crate) enum Op { Add, Sub }
#[derive(Debug)]
pub(crate) enum Expr { Literal(i64), Local(usize), Field(usize,usize), Call(usize,Vec<Arg>), Bin(Op,Box<Expr>,Box<Expr>) }
#[derive(Debug)]
pub(crate) enum Arg { Ref(usize), Int(Expr) }
#[derive(Debug)]
pub(crate) struct Body { pub(crate) locals:Vec<Expr>, pub(crate) result:Expr }
#[derive(Debug)]
pub(crate) struct Helper { pub(crate) params:Vec<Kind>, pub(crate) body:Body }
#[derive(Debug)]
pub(crate) struct Plan { fields:Vec<i64>, helpers:Vec<Helper>, root:Body }
impl Plan {
    pub(crate) fn fields(&self)->&[i64]{&self.fields}
    pub(crate) fn helpers(&self)->&[Helper]{&self.helpers}
    pub(crate) fn root(&self)->&Body{&self.root}
}
type Names=Vec<(String,Kind)>;
struct Reader<'a>{p:Parser<'a>, budget:usize}
impl Reader<'_>{
    fn word(&mut self,s:&str)->Result<(),String>{
        if !matches!(self.p.current,Token::Word(n) if n==s){return Err(err());} self.p.next()
    }
    fn punct(&mut self,b:u8)->Result<(),String>{
        if self.p.current!=Token::Punct(b){return Err(err());} self.p.next()
    }
    fn name(&mut self)->Result<String,String>{
        let n=self.p.aggregate_name()?;
        if n.is_empty()||n.len()>64 || !n.bytes().all(|b|b.is_ascii_alphanumeric()||b==b'_')
            || !n.as_bytes()[0].is_ascii_alphabetic() || matches!(n.as_str(),"new"|"main"|"define"|"class"|"pub"|"let"|"val"|"const"|"as"|"end"|"return"|"fnc"|"mut"|"set"){return Err(err());}
        Ok(n)
    }
    fn number(&mut self)->Result<i64,String>{
        let Token::Number(n)=self.p.current else{return Err(err());};
        if n.len()>19 || !n.bytes().all(|b|b.is_ascii_digit()){return Err(err());}
        let result=n.parse::<i64>().map_err(|_|err())?;self.p.next()?;Ok(result)
    }
    fn charge(&mut self,depth:usize)->Result<(),String>{
        if depth>16||self.budget==0{return Err(err());}self.budget-=1;Ok(())
    }
    fn slot(names:&Names,n:&str,kind:Kind)->Result<usize,String>{
        names.iter().position(|(name,k)|name==n&&*k==kind).ok_or_else(err)
    }
    fn expr(&mut self,names:&Names,fields:&[(String,i64)],helpers:&[(String,Helper)],depth:usize)->Result<Expr,String>{
        self.charge(depth)?;
        let mut lhs=self.atom(names,fields,helpers,depth+1)?;
        let mut terms=1;
        while let Token::Punct(b'+'|b'-')=self.p.current{
            if terms>=8{return Err(err());}terms+=1;self.charge(depth)?;
            let op=if self.p.current==Token::Punct(b'+'){Op::Add}else{Op::Sub};
            self.p.next()?;let rhs=self.atom(names,fields,helpers,depth+1)?;
            lhs=Expr::Bin(op,Box::new(lhs),Box::new(rhs));
        }
        Ok(lhs)
    }
    fn atom(&mut self,names:&Names,fields:&[(String,i64)],helpers:&[(String,Helper)],depth:usize)->Result<Expr,String>{
        self.charge(depth)?;
        if matches!(self.p.current,Token::Number(_)){return Ok(Expr::Literal(self.number()?));}
        if self.p.current==Token::Punct(b'('){
            self.p.next()?;let e=self.expr(names,fields,helpers,depth+1)?;self.punct(b')')?;return Ok(e);
        }
        let n=self.name()?;
        if self.p.current==Token::Punct(b'.'){
            let slot=Self::slot(names,&n,Kind::Ref)?;self.p.next()?;let field=self.name()?;
            return Ok(Expr::Field(slot,fields.iter().position(|(f,_)|*f==field).ok_or_else(err)?));
        }
        if self.p.current==Token::Punct(b'('){
            // Local shadowing is never silently interpreted as a helper.
            if names.iter().any(|(name,_)|*name==n){return Err(err());}
            let i=helpers.iter().position(|(name,_)|*name==n).ok_or_else(err)?;
            self.p.next()?;let mut args=Vec::new();
            for (j,k) in helpers[i].1.params.iter().enumerate(){
                if j>0{self.punct(b',')?;}
                args.push(match k{
                    Kind::Ref=>Arg::Ref(Self::slot(names,&self.name()?,Kind::Ref)?),
                    Kind::Int=>Arg::Int(self.expr(names,fields,helpers,depth+1)?),
                });
            }
            self.punct(b')')?;return Ok(Expr::Call(i,args));
        }
        Ok(Expr::Local(Self::slot(names,&n,Kind::Int)?))
    }
    fn body(&mut self,mut names:Names,fields:&[(String,i64)],helpers:&[(String,Helper)])->Result<Body,String>{
        let mut locals=Vec::new();
        while self.p.current==Token::Word("val"){
            if locals.len()>=8{return Err(err());}
            self.word("val")?;self.word("int")?;let n=self.name()?;
            if names.iter().any(|(name,_)|*name==n)||helpers.iter().any(|(name,_)|*name==n){return Err(err());}
            self.punct(b'=')?;let e=self.expr(&names,fields,helpers,0)?;self.punct(b';')?;
            locals.push(e);names.push((n,Kind::Int));
        }
        self.word("return")?;let result=self.expr(&names,fields,helpers,0)?;
        self.punct(b';')?;self.punct(b'}')?;Ok(Body{locals,result})
    }
    fn parse(mut self)->Result<Plan,String>{
        self.word("define")?;self.word("class")?;let class=self.name()?;self.word("as")?;
        let mut fields=Vec::<(String,i64)>::new();
        while self.p.current==Token::Word("pub"){
            if fields.len()>=8{return Err(err());}
            self.word("pub")?;self.word("let")?;self.word("int")?;
            let name=self.name()?;if fields.iter().any(|(n,_)|*n==name){return Err(err());}
            self.punct(b'=')?;let value=self.number()?;self.punct(b';')?;fields.push((name,value));
        }
        if fields.is_empty(){return Err(err());}self.word("end")?;
        let mut helpers=Vec::<(String,Helper)>::new();
        while self.p.current==Token::Word("fnc"){
            if helpers.len()>=8{return Err(err());}
            self.word("fnc")?;let name=self.name()?;
            if name==class||helpers.iter().any(|(n,_)|*n==name){return Err(err());}
            self.punct(b'(')?;let mut names=Names::new();
            loop{
                if names.len()>=8{return Err(err());}
                let kind=if self.p.current==Token::Word("int"){self.word("int")?;Kind::Int}else{self.word(&class)?;Kind::Ref};
                let n=self.name()?;
                if n==name||names.iter().any(|(p,_)|*p==n)||helpers.iter().any(|(p,_)|*p==n){return Err(err());}
                names.push((n,kind));
                if self.p.current!=Token::Punct(b','){break;}self.next_comma()?;
            }
            if !names.iter().any(|(_,k)|*k==Kind::Ref){return Err(err());}
            self.punct(b')')?;self.punct(b':')?;self.word("int")?;self.punct(b'{')?;
            let params=names.iter().map(|(_,k)|*k).collect();
            let body=self.body(names,&fields,&helpers)?;helpers.push((name,Helper{params,body}));
        }
        if helpers.is_empty(){return Err(err());}
        self.word("pub")?;self.word("fnc")?;self.word("main")?;self.punct(b'(')?;self.punct(b')')?;
        self.punct(b':')?;self.word("int")?;self.punct(b'{')?;self.word("let")?;let owner=self.name()?;
        if helpers.iter().any(|(name,_)|*name==owner){return Err(err());}
        self.punct(b'=')?;self.word("new")?;self.word(&class)?;self.punct(b'(')?;self.punct(b')')?;self.punct(b';')?;
        let root=self.body(vec![(owner,Kind::Ref)],&fields,&helpers)?;
        if self.p.current!=Token::End{return Err(err());}
        Ok(Plan{fields:fields.into_iter().map(|(_,v)|v).collect(),helpers:helpers.into_iter().map(|(_,h)|h).collect(),root})
    }
    fn next_comma(&mut self)->Result<(),String>{self.punct(b',')}
}
pub(crate) fn parse(s:&str)->Result<Plan,String>{
    if !selected(s)||s.len()>65536||!s.bytes().all(|b| matches!(b,9..=13|32..=126)){return Err(err());}
    super::validate_source_envelope(s)?;
    let mut p=Parser::new(s)?;p.point_placement_profile=true;
    Reader{p,budget:512}.parse()
}
