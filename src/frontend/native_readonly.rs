//! Closed native readonly class profile using the existing token scanner.
//! A plan is only constructible by full source admission; no raw pointers,
//! object escape, arbitrary methods, imports, actors or recursive calls.
use super::{Parser,Token};
const MARKER: &str = "// atto-ownership: borrow-default-v1";
pub(crate) fn selected(s:&str)->bool { s.lines().next()==Some(MARKER) }
fn err()->String { "E_NATIVE_READONLY: unsupported closed class reference profile".into() }
#[derive(Clone, Copy, Debug)]
pub(crate) enum Read { Field(usize), Forward(usize) }
#[derive(Clone,Copy,Debug)]
pub(crate) enum Op { Add, Sub }
#[derive(Debug)]
pub(crate) struct Plan { fields: Vec<i64>, helpers:Vec<Read>, calls:Vec<usize>, ops:Vec<Op> }
impl Plan {
    pub(crate) fn fields(&self)->&[i64]{&self.fields}
    pub(crate) fn helpers(&self)->&[Read]{&self.helpers}
    pub(crate) fn calls(&self)->&[usize]{&self.calls}
    pub(crate) fn ops(&self)->&[Op]{&self.ops}
}
struct Reader<'a>{p:Parser<'a>}
impl Reader<'_>{
    fn word(&mut self,s:&str)->Result<(),String>{
        if !matches!(self.p.current,Token::Word(n) if n==s){return Err(err());} self.p.next()
    }
    fn punct(&mut self,b:u8)->Result<(),String>{
        if self.p.current!=Token::Punct(b){return Err(err());} self.p.next()
    }
    fn name(&mut self)->Result<String,String>{
        let n=self.p.aggregate_name()?;
        if n.len()>64 || !n.bytes().all(|b|b.is_ascii_alphanumeric()||b==b'_')
            || !n.as_bytes()[0].is_ascii_alphabetic() || matches!(n.as_str(),"new"|"main"|"define"|"class"|"pub"|"let"|"as"|"end"|"return"|"fnc"){return Err(err());}
        Ok(n)
    }
    fn number(&mut self)->Result<i64,String>{
        let Token::Number(n)=self.p.current else{return Err(err());};
        if n.len()>19 || !n.bytes().all(|b|b.is_ascii_digit()){return Err(err());}
        let result=n.parse::<i64>().map_err(|_|err())?;self.p.next()?;Ok(result)
    }
    fn call(&mut self,helpers:&[(String,Read)],param:&str)->Result<usize,String>{
        let name=self.name()?;let i=helpers.iter().position(|(n,_)|*n==name).ok_or_else(err)?;
        self.punct(b'(')?;self.word(param)?;self.punct(b')')?;Ok(i)
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
        let mut helpers=Vec::<(String,Read)>::new();
        while self.p.current==Token::Word("fnc"){
            if helpers.len()>=8{return Err(err());}
            self.word("fnc")?;let name=self.name()?;
            if name==class||helpers.iter().any(|(n,_)|*n==name){return Err(err());}
            self.punct(b'(')?;self.word(&class)?;let param=self.name()?;self.punct(b')')?;
            self.punct(b':')?;self.word("int")?;self.punct(b'{')?;self.word("return")?;
            let read=if matches!(self.p.current,Token::Word(n) if n==param) {
                self.word(&param)?;self.punct(b'.')?;let field=self.name()?;
                Read::Field(fields.iter().position(|(n,_)|*n==field).ok_or_else(err)?)
            } else { Read::Forward(self.call(&helpers,&param)?) };
            self.punct(b';')?;self.punct(b'}')?;helpers.push((name,read));
        }
        if helpers.is_empty(){return Err(err());}
        self.word("pub")?;self.word("fnc")?;self.word("main")?;self.punct(b'(')?;self.punct(b')')?;
        self.punct(b':')?;self.word("int")?;self.punct(b'{')?;self.word("let")?;let owner=self.name()?;
        self.punct(b'=')?;self.word("new")?;self.word(&class)?;self.punct(b'(')?;self.punct(b')')?;self.punct(b';')?;
        self.word("return")?;let mut calls=vec![self.call(&helpers,&owner)?];let mut ops=Vec::new();
        // Equal-precedence + and - only; multiplication is not admitted
        // until a precedence-aware expression proof is added.
        while let Token::Punct(b'+'|b'-')=self.p.current{
            if calls.len()>=8{return Err(err());}
            ops.push(if self.p.current==Token::Punct(b'+'){Op::Add}else{Op::Sub});
            self.p.next()?;calls.push(self.call(&helpers,&owner)?);
        }
        self.punct(b';')?;self.punct(b'}')?;if self.p.current!=Token::End{return Err(err());}
        Ok(Plan{fields:fields.into_iter().map(|(_,v)|v).collect(),helpers:helpers.into_iter().map(|(_,r)|r).collect(),calls,ops})
    }
}
pub(crate) fn parse(s:&str)->Result<Plan,String>{
    if !selected(s)||s.len()>65536||!s.bytes().all(|b| matches!(b,9..=13|32..=126)){return Err(err());}
    super::validate_source_envelope(s)?;
    let mut p=Parser::new(s)?;
    // Existing scanner's new gate is lifted only for this complete grammar.
    // No placement operations are accepted by the grammar above.
    p.point_placement_profile=true;
    Reader{p}.parse()
}
