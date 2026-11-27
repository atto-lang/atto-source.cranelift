use super::{Parser,Token};
use std::collections::BTreeMap;
#[derive(Clone, Copy, Debug)]
pub(crate) enum Step {
    Construct(i64,i64), Bind, CopyField(u8), Detach, Destroy, Free,
}
#[derive(Clone, Debug)]
pub(crate) struct Plan {
    pub steps: Vec<Step>,
    pub result: Vec<usize>,
}
fn err() -> String { "E_POINTER_PLACEMENT: invalid closed typed Point placement profile".into() }
impl Plan {
    pub(crate) fn verify(&self) -> Result<usize,String> {
        let bad=err;
        if self.steps.is_empty() || self.steps.len()>32 || self.result.is_empty() || self.result.len()>2 {
            return Err(bad());
        }
        #[derive(PartialEq,Eq)]
        enum State { Empty, Live, Freed }
        let mut state=State::Empty;
        let mut generation=0u32;
        let mut bound=None;
        let mut fields=[0i64;2];
        let mut copies=Vec::new();
        for step in &self.steps {
            match *step {
                Step::Construct(x,y) if state==State::Empty && bound.is_none() && x>=0 && y>=0 => {
                    generation=generation.checked_add(1).ok_or_else(bad)?;
                    fields=[x,y]; state=State::Live;
                }
                Step::Bind if state==State::Live && bound.is_none() => bound=Some(generation),
                Step::CopyField(f) if state==State::Live && bound==Some(generation) && f<2 => copies.push(fields[f as usize]),
                Step::Detach if state==State::Live && bound==Some(generation) => bound=None,
                Step::Destroy if state==State::Live && bound.is_none() => state=State::Empty,
                Step::Free if state==State::Empty && bound.is_none() => state=State::Freed,
                _ => return Err(bad()),
            }
        }
        if state!=State::Freed || bound.is_some() {return Err(bad());}
        let mut sum=0i64;
        for i in &self.result {
            sum=sum.checked_add(*copies.get(*i).ok_or_else(bad)?).ok_or_else(bad)?;
        }
        Ok(copies.len())
    }
}

struct PointParser<'a> { p: Parser<'a> }
impl PointParser<'_> {
    fn word_is(&self,w:&str)->bool { matches!(self.p.current,Token::Word(n) if n==w) }
    fn word(&mut self,w:&str)->Result<(),String> {
        if !self.word_is(w){return Err(err());}self.p.next()
    }
    fn need(&mut self,t:Token<'_>)->Result<(),String>{
        if self.p.current!=t{return Err(err());}self.p.next()
    }
    fn op(&mut self,w:&str)->Result<(),String>{
        let expected=match w {"alloc"=>"alloc!","ref"=>"ref!","deref"=>"deref!","unref"=>"unref!","destroy"=>"destroy!","free"=>"free!",_=>return Err(err())};
        self.word(expected)
    }
    fn number(&mut self)->Result<i64,String>{
        let Token::Number(n)=self.p.current else{return Err(err());};
        if !n.bytes().all(|b|b.is_ascii_digit()){return Err(err());}
        let v=n.parse::<i64>().map_err(|_|err())?;self.p.next()?;Ok(v)
    }
    fn ident(&mut self)->Result<String,String>{
        let Token::Word(n)=self.p.current else{return Err(err());};
        if n.len()>64||!n.starts_with(|c:char|c.is_ascii_lowercase())||
            !n.chars().all(|c|c.is_ascii_lowercase()||c.is_ascii_digit()||c=='_')||
            matches!(n,"s"|"p"|"x"|"y"|"main"|"int"|"pub"|"fnc"|"let"|"val"|"mut"|"return"|"alloc"|"destroy"|"free"|"ref"|"deref"|"unref"|"new"|"at"|"to"|"empty"|"null"|"true"|"false"|"self"|"constructor"|"define"|"struct"|"with"|"await"|"suspends"|"rt"|"if"|"else"|"end"|"while"|"for"|"class"|"actor"|"task"|"select"|"defer"|"recover"|"copy"|"take"|"borrow"|"share"|"drop"|"move"|"pin"|"import"|"export"|"use"|"namespace"|"module"|"const"|"unsafe"|"void"|"panic"|"trap"|"throw"|"raise") {return Err(err());}
        self.p.name()
    }
    fn parse(&mut self)->Result<Plan,String> {
        self.word("define")?;self.word("struct")?;self.word("Point")?;self.need(Token::Punct(b'{'))?;
        for f in ["x","y"] {self.word("int")?;self.word(f)?;self.need(Token::Punct(b';'))?;}
        self.need(Token::Punct(b'}'))?;self.word("with")?;self.need(Token::Punct(b'{'))?;self.word("constructor")?;
        self.need(Token::Punct(b'('))?;self.word("int")?;self.word("x")?;self.need(Token::Punct(b','))?;
        self.word("int")?;self.word("y")?;self.need(Token::Punct(b')'))?;self.need(Token::Punct(b'{'))?;
        for f in ["x","y"] {self.word("self")?;self.need(Token::Punct(b'.'))?;self.word(f)?;self.need(Token::Punct(b'='))?;self.word(f)?;self.need(Token::Punct(b';'))?;}
        self.need(Token::Punct(b'}'))?;self.need(Token::Punct(b'}'))?;
        self.word("pub")?;self.word("fnc")?;self.word("main")?;self.need(Token::Punct(b'('))?;self.need(Token::Punct(b')'))?;
        self.need(Token::Punct(b':'))?;self.word("int")?;self.need(Token::Punct(b'{'))?;
        self.word("let")?;self.word("Storage")?;self.need(Token::Punct(b'<'))?;self.word("Point")?;self.need(Token::Punct(b'>'))?;
        self.word("s")?;self.need(Token::Punct(b'='))?;self.op("alloc")?;self.word("Point")?;self.need(Token::Punct(b';'))?;
        self.word("let")?;self.word("mut")?;self.word("RawPointer")?;self.need(Token::Punct(b'<'))?;self.word("Point")?;
        self.need(Token::Punct(b'>'))?;self.word("p")?;self.need(Token::Punct(b'='))?;self.word("RawPointer")?;self.need(Token::Punct(b'.'))?;
        self.word("empty")?;self.need(Token::Punct(b'<'))?;self.word("Point")?;self.need(Token::Punct(b'>'))?;self.need(Token::Punct(b'('))?;self.need(Token::Punct(b')'))?;self.need(Token::Punct(b';'))?;
        let mut steps=Vec::new();let mut names=BTreeMap::new();
        while !self.word_is("return") {
            if steps.len()>=32{return Err(err());}
            let step=if self.word_is("new") {
                self.word("new")?;self.word("Point")?;self.need(Token::Punct(b'('))?;let x=self.number()?;
                self.need(Token::Punct(b','))?;let y=self.number()?;self.need(Token::Punct(b')'))?;self.word("at")?;self.word("s")?;Step::Construct(x,y)
            } else if self.word_is("ref!") {
                self.op("ref")?;self.word("p")?;self.word("to")?;self.word("s")?;Step::Bind
            } else if self.word_is("val") {
                self.word("val")?;let n=self.ident()?;
                let i=names.len();if names.insert(n,i).is_some(){return Err(err());}
                self.need(Token::Punct(b'='))?;self.need(Token::Punct(b'('))?;self.op("deref")?;self.word("p")?;self.need(Token::Punct(b')'))?;self.need(Token::Punct(b'.'))?;
                if self.word_is("x"){self.word("x")?;Step::CopyField(0)}else{self.word("y")?;Step::CopyField(1)}
            } else if self.word_is("unref!") {self.op("unref")?;self.word("p")?;Step::Detach}
            else if self.word_is("destroy!"){self.op("destroy")?;self.word("s")?;Step::Destroy}
            else if self.word_is("free!"){self.op("free")?;self.word("s")?;Step::Free}
            else{return Err(err());};
            self.need(Token::Punct(b';'))?;steps.push(step);
        }
        self.word("return")?;let mut result=Vec::new();
        loop {
            if result.len()==2{return Err(err());}
            let n=self.ident()?;result.push(*names.get(&n).ok_or_else(||err())?);
            if !self.p.take(Token::Punct(b'+'))?{break;}
        }
        self.need(Token::Punct(b';'))?;self.need(Token::Punct(b'}'))?;self.need(Token::End)?;
        
        let p=Plan{steps,result};p.verify()?;Ok(p)
    }
}

pub(crate) fn parse(text:&str)->Result<Plan,String>{
    super::validate_source_envelope(text)?;
    let mut p=Parser::new(text)?;
    p.reusable_slot_profile=true;
    PointParser{p}.parse()
}
