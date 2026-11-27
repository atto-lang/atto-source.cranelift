use super::{Parser,Token};
use super::physical_layout::{self, FieldType, Layout, Limits, Owner, Storage};
use std::collections::BTreeMap;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind { I8, I16, I32, I64 }
impl Kind {
    fn parse(name: &str) -> Option<Self> {
        match name { "i8" => Some(Self::I8), "i16" => Some(Self::I16), "i32" => Some(Self::I32),
            "int" | "i64" => Some(Self::I64), _ => None }
    }
    fn size(self) -> u64 {
        match self { Self::I8 => 1, Self::I16 => 2, Self::I32 => 4, Self::I64 => 8 }
    }
    fn fits(self, n: i64) -> bool {
        match self { Self::I8 => i8::try_from(n).is_ok(), Self::I16 => i16::try_from(n).is_ok(),
            Self::I32 => i32::try_from(n).is_ok(), Self::I64 => true }
    }
}
#[derive(Clone, Debug)]
pub(crate) enum Step {
    Construct(Vec<i64>), Bind, CopyField(u8), Detach, Destroy, Free,
}
#[derive(Clone, Debug)]
pub(crate) struct Plan {
    owner: String,
    fields: Vec<(String, Kind)>,
    pub steps: Vec<Step>,
    pub result: Vec<usize>,
}
fn err() -> String { "E_POINTER_PLACEMENT: invalid closed typed flat struct placement profile".into() }
impl Plan {
    pub(crate) fn layout(&self) -> Result<Layout,String> {
        if self.fields.is_empty() || self.fields.len() > 8 { return Err(err()); }
        let owner = Owner { identity: self.owner.clone(), fields: self.fields.iter().map(|(n,k)| {
            let size=k.size(); (n.clone(),FieldType::Scalar(Storage {size,align:size}))
        }).collect() };
        physical_layout::plan(&[owner],Limits {max_size:65536,max_align:8,max_owners:1,max_fields:8})?
            .pop().ok_or_else(err)
    }
    pub(crate) fn verify(&self) -> Result<usize,String> {
        self.layout()?;
        let bad=err;
        if self.steps.is_empty() || self.steps.len()>32 || self.result.is_empty() || self.result.len()>8 {
            return Err(bad());
        }
        #[derive(PartialEq,Eq)]
        enum State { Empty, Live, Freed }
        let mut state=State::Empty;
        let mut generation=0u32;
        let mut bound=None;
        let mut fields=vec![0i64;self.fields.len()];
        let mut copies=Vec::new();
        for step in &self.steps {
            match step {
                Step::Construct(values) if state==State::Empty && bound.is_none()
                    && values.len()==self.fields.len()
                    && values.iter().zip(&self.fields).all(|(&v,(_,k))| k.fits(v)) => {
                    generation=generation.checked_add(1).ok_or_else(bad)?;
                    fields.clone_from(values); state=State::Live;
                }
                Step::Bind if state==State::Live && bound.is_none() => bound=Some(generation),
                Step::CopyField(f) if state==State::Live && bound==Some(generation) && usize::from(*f)<fields.len() => copies.push(fields[usize::from(*f)]),
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
        let negative=self.p.take(Token::Punct(b'-'))?;
        let Token::Number(n)=self.p.current else{return Err(err());};
        if n.len()>19 || !n.bytes().all(|b|b.is_ascii_digit()){return Err(err());}
        let signed=if negative {format!("-{n}")} else {n.to_owned()};
        let v=signed.parse::<i64>().map_err(|_|err())?;self.p.next()?;Ok(v)
    }
    fn ident(&mut self)->Result<String,String>{
        let Token::Word(n)=self.p.current else{return Err(err());};
        if n.len()>64||!n.starts_with(|c:char|c.is_ascii_lowercase())||
            !n.chars().all(|c|c.is_ascii_lowercase()||c.is_ascii_digit()||c=='_')||
            matches!(n,"s"|"p"|"main"|"int"|"pub"|"fnc"|"let"|"val"|"mut"|"return"|"alloc"|"destroy"|"free"|"ref"|"deref"|"unref"|"new"|"at"|"to"|"empty"|"null"|"true"|"false"|"self"|"constructor"|"define"|"struct"|"with"|"await"|"suspends"|"rt"|"if"|"else"|"end"|"while"|"for"|"class"|"actor"|"task"|"select"|"defer"|"recover"|"copy"|"take"|"borrow"|"share"|"drop"|"move"|"pin"|"import"|"export"|"use"|"namespace"|"module"|"const"|"unsafe"|"void"|"panic"|"trap"|"throw"|"raise") {return Err(err());}
        self.p.name()
    }
    fn parse(&mut self)->Result<Plan,String> {
        self.word("define")?;self.word("struct")?;
        let owner=self.p.aggregate_name()?;
        if owner.len()>64 || !owner.starts_with(|c:char|c.is_ascii_uppercase())
            || !owner.bytes().all(|b|b.is_ascii_alphanumeric()||b==b'_')
            || matches!(owner.as_str(),"Storage"|"RawPointer") {return Err(err());}
        self.need(Token::Punct(b'{'))?;
        let mut fields=Vec::<(String,Kind)>::new();
        while self.p.current!=Token::Punct(b'}') {
            if fields.len()>=8 {return Err(err());}
            let ty=self.p.name()?;let kind=Kind::parse(&ty).ok_or_else(err)?;
            let name=self.ident()?;
            if fields.iter().any(|(n,_)|n==&name){return Err(err());}
            self.need(Token::Punct(b';'))?;fields.push((name,kind));
        }
        if fields.is_empty(){return Err(err());}
        self.need(Token::Punct(b'}'))?;self.word("with")?;self.need(Token::Punct(b'{'))?;self.word("constructor")?;
        self.need(Token::Punct(b'('))?;
        for (i,(f,k)) in fields.iter().enumerate() {
            if i>0 {self.need(Token::Punct(b','))?;}
            if Kind::parse(&self.p.name()?)!=Some(*k){return Err(err());}
            self.word(f)?;
        }
        self.need(Token::Punct(b')'))?;self.need(Token::Punct(b'{'))?;
        // This bounded profile accepts only complete, identity constructor assignments.
        for (f,_) in &fields {self.word("self")?;self.need(Token::Punct(b'.'))?;self.word(f)?;self.need(Token::Punct(b'='))?;self.word(f)?;self.need(Token::Punct(b';'))?;}
        self.need(Token::Punct(b'}'))?;self.need(Token::Punct(b'}'))?;
        self.word("pub")?;self.word("fnc")?;self.word("main")?;self.need(Token::Punct(b'('))?;self.need(Token::Punct(b')'))?;
        self.need(Token::Punct(b':'))?;self.word("int")?;self.need(Token::Punct(b'{'))?;
        self.word("let")?;self.word("Storage")?;self.need(Token::Punct(b'<'))?;self.word(&owner)?;self.need(Token::Punct(b'>'))?;
        self.word("s")?;self.need(Token::Punct(b'='))?;self.op("alloc")?;self.word(&owner)?;self.need(Token::Punct(b';'))?;
        self.word("let")?;self.word("mut")?;self.word("RawPointer")?;self.need(Token::Punct(b'<'))?;self.word(&owner)?;
        self.need(Token::Punct(b'>'))?;self.word("p")?;self.need(Token::Punct(b'='))?;self.word("RawPointer")?;self.need(Token::Punct(b'.'))?;
        self.word("empty")?;self.need(Token::Punct(b'<'))?;self.word(&owner)?;self.need(Token::Punct(b'>'))?;self.need(Token::Punct(b'('))?;self.need(Token::Punct(b')'))?;self.need(Token::Punct(b';'))?;
        let mut steps=Vec::new();let mut names=BTreeMap::new();
        while !self.word_is("return") {
            if steps.len()>=32{return Err(err());}
            let step=if self.word_is("new") {
                self.word("new")?;self.word(&owner)?;self.need(Token::Punct(b'('))?;
                let mut values=Vec::new();
                for (i,(_,k)) in fields.iter().enumerate(){
                    if i>0 {self.need(Token::Punct(b','))?;}
                    let v=self.number()?;if !k.fits(v){return Err(err());}values.push(v);
                }
                self.need(Token::Punct(b')'))?;self.word("at")?;self.word("s")?;Step::Construct(values)
            } else if self.word_is("ref!") {
                self.op("ref")?;self.word("p")?;self.word("to")?;self.word("s")?;Step::Bind
            } else if self.word_is("val") {
                self.word("val")?;let n=self.ident()?;
                let i=names.len();if fields.iter().any(|(f,_)|f==&n)||names.insert(n,i).is_some(){return Err(err());}
                self.need(Token::Punct(b'='))?;self.need(Token::Punct(b'('))?;self.op("deref")?;self.word("p")?;self.need(Token::Punct(b')'))?;self.need(Token::Punct(b'.'))?;
                let field=self.p.name()?;
                Step::CopyField(u8::try_from(fields.iter().position(|(n,_)|n==&field).ok_or_else(err)?).map_err(|_|err())?)
            } else if self.word_is("unref!") {self.op("unref")?;self.word("p")?;Step::Detach}
            else if self.word_is("destroy!"){self.op("destroy")?;self.word("s")?;Step::Destroy}
            else if self.word_is("free!"){self.op("free")?;self.word("s")?;Step::Free}
            else{return Err(err());};
            self.need(Token::Punct(b';'))?;steps.push(step);
        }
        self.word("return")?;let mut result=Vec::new();
        loop {
            if result.len()==8{return Err(err());}
            let n=self.ident()?;result.push(*names.get(&n).ok_or_else(err)?);
            if !self.p.take(Token::Punct(b'+'))?{break;}
        }
        self.need(Token::Punct(b';'))?;self.need(Token::Punct(b'}'))?;self.need(Token::End)?;
        
        let p=Plan{owner,fields,steps,result};p.verify()?;Ok(p)
    }
}

pub(crate) fn parse(text:&str)->Result<Plan,String>{
    super::validate_source_envelope_impl(text, false)?;
    let mut p=Parser::new(text)?;
    p.reusable_slot_profile=true;
    // Only this complete closed placement grammar can admit `new`.
    // Ordinary scalar and raw-pointer slot parsers keep the class-allocation denial.
    p.point_placement_profile=true;
    PointParser{p}.parse()
}
