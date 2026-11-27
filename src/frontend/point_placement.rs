use super::{Parser,Token};
use super::physical_layout::{self, FieldType, Limits, Owner, Storage};
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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FieldKind { Scalar(Kind), Inline(usize), ScalarArray(Kind,u64), InlineArray(usize,u64) }
#[derive(Clone, Debug)]
struct Decl { name: String, fields: Vec<(String,FieldKind)> }
#[derive(Clone, Debug)]
struct Leaf { name: String, path: Vec<usize>, kind: Kind }
fn leaves(owners:&[Decl],root:usize)->Result<Vec<Leaf>,String>{
    fn visit(owners:&[Decl],id:usize,prefix:&str,path:&mut Vec<usize>,out:&mut Vec<Leaf>)->Result<(),String>{
        if path.len()>=8{return Err(err());}
        let owner=owners.get(id).ok_or_else(err)?;
        for (index,(name,kind)) in owner.fields.iter().enumerate(){
            path.push(index);
            let name=if prefix.is_empty(){name.clone()}else{format!("{prefix}.{name}")};
            match kind {
                FieldKind::Scalar(k)=>{
                    if out.len()>=8{return Err(err());}
                    out.push(Leaf{name,path:path.clone(),kind:*k});
                }
                FieldKind::ScalarArray(k,n)=>{
                    if *n==0||*n>8||path.len()>=8{return Err(err());}
                    for cell in 0..*n{
                        if out.len()>=8{return Err(err());}
                        path.push(usize::try_from(cell).map_err(|_|err())?);
                        out.push(Leaf{name:format!("{name}[{cell}]"),path:path.clone(),kind:*k});
                        path.pop();
                    }
                }
                FieldKind::InlineArray(i,n) if *i<id=>{
                    if *n==0||*n>8||path.len()>=8{return Err(err());}
                    for cell in 0..*n{
                        path.push(usize::try_from(cell).map_err(|_|err())?);
                        visit(owners,*i,&format!("{name}[{cell}]"),path,out)?;
                        path.pop();
                    }
                }
                FieldKind::InlineArray(_,_)=>return Err(err()),
                FieldKind::Inline(i) if *i<id=>visit(owners,*i,&name,path,out)?,
                FieldKind::Inline(_)=>return Err(err()),
            }
            path.pop();
        }
        Ok(())
    }
    let mut result=Vec::new();visit(owners,root,"",&mut Vec::new(),&mut result)?;
    if result.is_empty(){return Err(err());}Ok(result)
}
#[derive(Clone, Copy, Debug)]
pub(crate) enum Operand { Literal(i64), Parameter(usize) }
#[derive(Clone, Debug)]
pub(crate) enum Step {
    Construct(Vec<Operand>), Bind, CopyField(u8), Detach, Destroy, Free,
}
#[derive(Clone, Debug)]
pub(crate) struct Plan {
    owners: Vec<Decl>,
    root: usize,
    parameters: Vec<(String,Kind)>,
    pub steps: Vec<Step>,
    pub result: Vec<(usize, bool)>, // (copy index, subtract from preceding result)
}
fn err() -> String { "E_POINTER_PLACEMENT: invalid closed typed nested struct placement profile".into() }
impl Plan {
    pub(crate) fn parameter_names(&self)->Vec<String>{self.parameters.iter().map(|(n,_)|n.clone()).collect()}
    pub(crate) fn parameter_sizes(&self)->Vec<u64>{self.parameters.iter().map(|(_,k)|k.size()).collect()}
    fn leaves(&self) -> Result<Vec<Leaf>,String> { leaves(&self.owners,self.root) }
    pub(crate) fn physical_slots(&self) -> Result<(Storage,Vec<(u64,Storage)>),String> {
        let leaves=self.leaves()?;
        let owners=self.owners.iter().map(|o|Owner {identity:o.name.clone(),fields:o.fields.iter().map(|(n,k)|{
            let field=match k {FieldKind::Scalar(k)=>{let size=k.size();FieldType::Scalar(Storage{size,align:size})},FieldKind::Inline(i)=>FieldType::Inline(*i),FieldKind::ScalarArray(k,n)=>{let size=k.size();FieldType::ScalarArray(Storage{size,align:size},*n)},FieldKind::InlineArray(i,n)=>FieldType::InlineArray(*i,*n)};
            (n.clone(),field)
        }).collect()}).collect::<Vec<_>>();
        let layouts=physical_layout::plan(&owners,Limits{max_size:65536,max_align:8,max_owners:8,max_fields:64})?;
        let root=layouts.get(self.root).ok_or_else(err)?;
        let slots=leaves.iter().map(|leaf|root.project(&layouts,&leaf.path)).collect::<Result<Vec<_>,_>>()?;
        Ok((root.storage,slots))
    }
    pub(crate) fn verify(&self) -> Result<usize,String> {
        self.physical_slots()?;
        let leaves=self.leaves()?;
        let bad=err;
        if self.steps.is_empty() || self.steps.len()>32 || self.result.is_empty() || self.result.len()>8 || self.result[0].1 {
            return Err(bad());
        }
        #[derive(PartialEq,Eq)]
        enum State { Empty, Live, Freed }
        let mut state=State::Empty;
        let mut generation=0u32;
        let mut bound=None;
        if self.parameters.len()>4{return Err(bad());}
        let mut fields=vec![None;leaves.len()];
        let mut copies=Vec::new();
        for step in &self.steps {
            match step {
                Step::Construct(values) if state==State::Empty && bound.is_none()
                    && values.len()==leaves.len()
                    && values.iter().zip(&leaves).all(|(v,l)|match v{Operand::Literal(n)=>l.kind.fits(*n),Operand::Parameter(i)=>self.parameters.get(*i).is_some_and(|(_,k)|*k==l.kind)}) => {
                    generation=generation.checked_add(1).ok_or_else(bad)?;
                    fields=values.iter().map(|v|match v{Operand::Literal(n)=>Some(*n),Operand::Parameter(_)=>None}).collect(); state=State::Live;
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
        let mut sum=Some(0i64);
        for &(i,subtract) in &self.result {
            let value=*copies.get(i).ok_or_else(bad)?;
            sum=match (sum,value){
                (Some(a),Some(b))=>Some(if subtract{a.checked_sub(b)}else{a.checked_add(b)}.ok_or_else(bad)?),
                _=>None,
            };
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
    fn kind(&mut self,owners:&[Decl])->Result<FieldKind,String>{
        let name=self.p.name()?;
        if name=="Array"{
            self.need(Token::Punct(b'<'))?;
            let element=self.p.name()?;
            self.need(Token::Punct(b','))?;
            let n=u64::try_from(self.number()?).map_err(|_|err())?;
            if !(1..=8).contains(&n){return Err(err());}
            self.need(Token::Punct(b'>'))?;
            if let Some(k)=Kind::parse(&element){return Ok(FieldKind::ScalarArray(k,n));}
            return owners.iter().position(|o|o.name==element).map(|i|FieldKind::InlineArray(i,n)).ok_or_else(err);
        }
        if let Some(k)=Kind::parse(&name){return Ok(FieldKind::Scalar(k));}
        owners.iter().position(|o|o.name==name).map(FieldKind::Inline).ok_or_else(err)
    }
    fn declaration(&mut self,owners:&[Decl])->Result<Decl,String>{
        self.word("define")?;self.word("struct")?;
        let owner=self.p.aggregate_name()?;
        if owner.len()>64 || !owner.starts_with(|c:char|c.is_ascii_uppercase())
            || !owner.bytes().all(|b|b.is_ascii_alphanumeric()||b==b'_')
            || matches!(owner.as_str(),"Storage"|"RawPointer")
            || owners.iter().any(|o|o.name==owner){return Err(err());}
        self.need(Token::Punct(b'{'))?;
        let mut fields=Vec::<(String,FieldKind)>::new();
        while self.p.current!=Token::Punct(b'}'){
            if fields.len()>=8{return Err(err());}
            let kind=self.kind(owners)?;let name=self.ident()?;
            if fields.iter().any(|(n,_)|n==&name){return Err(err());}
            self.need(Token::Punct(b';'))?;fields.push((name,kind));
        }
        if fields.is_empty(){return Err(err());}
        self.need(Token::Punct(b'}'))?;self.word("with")?;self.need(Token::Punct(b'{'))?;self.word("constructor")?;
        self.need(Token::Punct(b'('))?;
        for (i,(f,k)) in fields.iter().enumerate(){
            if i>0{self.need(Token::Punct(b','))?;}
            if self.kind(owners)?!=*k{return Err(err());}self.word(f)?;
        }
        self.need(Token::Punct(b')'))?;self.need(Token::Punct(b'{'))?;
        for (f,_) in &fields{self.word("self")?;self.need(Token::Punct(b'.'))?;self.word(f)?;self.need(Token::Punct(b'='))?;self.word(f)?;self.need(Token::Punct(b';'))?;}
        self.need(Token::Punct(b'}'))?;self.need(Token::Punct(b'}'))?;
        Ok(Decl{name:owner,fields})
    }
    fn arguments(&mut self,owners:&[Decl],id:usize,depth:usize,parameters:&[(String,Kind)],values:&mut Vec<Operand>)->Result<(),String>{
        if depth>=8{return Err(err());}
        let owner=owners.get(id).ok_or_else(err)?;
        self.word(&owner.name)?;
        // Placement root retains new T(...) at storage. Only nested value
        // expressions admit the two constructor aliases of the resolved owner.
        if depth>0 && self.p.take(Token::Punct(b'.'))? {
            if self.word_is("make"){self.word("make")?;}
            else{self.word("new")?;}
        }
        self.need(Token::Punct(b'('))?;
        for (i,(_,kind)) in owner.fields.iter().enumerate(){
            if i>0{self.need(Token::Punct(b','))?;}
            match kind{
                FieldKind::Scalar(k)=>{if values.len()>=8{return Err(err());}let v=if matches!(self.p.current,Token::Word(_)){let name=self.ident()?;let i=parameters.iter().position(|(n,ty)|n==&name&&ty==k).ok_or_else(err)?;Operand::Parameter(i)}else{let n=self.number()?;if !k.fits(n){return Err(err());}Operand::Literal(n)};values.push(v);}
                FieldKind::ScalarArray(k,n)=>{
                    self.word("Array")?;self.need(Token::Punct(b'.'))?;self.word("of")?;self.need(Token::Punct(b'('))?;
                    for cell in 0..*n{
                        if cell>0{self.need(Token::Punct(b','))?;}
                        if values.len()>=8{return Err(err());}
                        let v=if matches!(self.p.current,Token::Word(_)){let name=self.ident()?;let i=parameters.iter().position(|(n,ty)|n==&name&&ty==k).ok_or_else(err)?;Operand::Parameter(i)}else{let v=self.number()?;if !k.fits(v){return Err(err());}Operand::Literal(v)};values.push(v);
                    }
                    self.need(Token::Punct(b')'))?;
                }
                FieldKind::InlineArray(i,n) if *i<id=>{
                    self.word("Array")?;self.need(Token::Punct(b'.'))?;self.word("of")?;self.need(Token::Punct(b'('))?;
                    for cell in 0..*n{
                        if cell>0{self.need(Token::Punct(b','))?;}
                        self.arguments(owners,*i,depth+1,parameters,values)?;
                    }
                    self.need(Token::Punct(b')'))?;
                }
                FieldKind::InlineArray(_,_)=>return Err(err()),
                FieldKind::Inline(n) if *n<id=>self.arguments(owners,*n,depth+1,parameters,values)?,
                FieldKind::Inline(_)=>return Err(err()),
            }
        }
        self.need(Token::Punct(b')'))
    }
    fn parse(&mut self)->Result<Plan,String>{
        let mut owners=Vec::new();
        while self.word_is("define"){
            if owners.len()>=8{return Err(err());}
            let declaration=self.declaration(&owners)?;owners.push(declaration);
            leaves(&owners,owners.len()-1)?;
        }
        let root=owners.len().checked_sub(1).ok_or_else(err)?;
        let owner=owners[root].name.clone();
        let fields=&owners[root].fields;
        let flat=leaves(&owners,root)?;
        self.word("pub")?;self.word("fnc")?;self.word("main")?;self.need(Token::Punct(b'('))?;
        let mut parameters=Vec::new();
        if !self.p.take(Token::Punct(b')'))? {
            loop {
                if parameters.len()>=4{return Err(err());}
                let kind=Kind::parse(&self.p.name()?).ok_or_else(err)?;
                let name=self.ident()?;
                if fields.iter().any(|(f,_)|f==&name)||parameters.iter().any(|(n,_)|n==&name){return Err(err());}
                parameters.push((name,kind));
                if self.p.take(Token::Punct(b')'))?{break;}
                self.need(Token::Punct(b','))?;
            }
        }
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
                self.word("new")?;let mut values=Vec::new();
                self.arguments(&owners,root,0,&parameters,&mut values)?;
                self.word("at")?;self.word("s")?;Step::Construct(values)
            } else if self.word_is("ref!") {
                self.op("ref")?;self.word("p")?;self.word("to")?;self.word("s")?;Step::Bind
            } else if self.word_is("val") {
                self.word("val")?;let n=self.ident()?;
                let i=names.len();if fields.iter().any(|(f,_)|f==&n)||parameters.iter().any(|(p,_)|p==&n)||names.insert(n,i).is_some(){return Err(err());}
                self.need(Token::Punct(b'='))?;self.need(Token::Punct(b'('))?;self.op("deref")?;self.word("p")?;self.need(Token::Punct(b')'))?;self.need(Token::Punct(b'.'))?;
                let mut field=self.p.name()?;
                for _ in 1..8{
                    if self.p.take(Token::Punct(b'.'))?{field.push('.');field.push_str(&self.p.name()?);}
                    else if self.p.take(Token::Punct(b'['))?{
                        let cell=u64::try_from(self.number()?).map_err(|_|err())?;
                        if cell>=8{return Err(err());}
                        self.need(Token::Punct(b']'))?;
                        field.push_str(&format!("[{cell}]"));
                    }else{break;}
                }
                Step::CopyField(u8::try_from(flat.iter().position(|l|l.name==field).ok_or_else(err)?).map_err(|_|err())?)
            } else if self.word_is("unref!") {self.op("unref")?;self.word("p")?;Step::Detach}
            else if self.word_is("destroy!"){self.op("destroy")?;self.word("s")?;Step::Destroy}
            else if self.word_is("free!"){self.op("free")?;self.word("s")?;Step::Free}
            else{return Err(err());};
            self.need(Token::Punct(b';'))?;steps.push(step);
        }
        self.word("return")?;let mut result=Vec::new();let mut subtract=false;
        loop {
            if result.len()==8{return Err(err());}
            let n=self.ident()?;result.push((*names.get(&n).ok_or_else(err)?,subtract));
            if self.p.take(Token::Punct(b'+'))?{subtract=false;}
            else if self.p.take(Token::Punct(b'-'))?{subtract=true;}
            else{break;}
        }
        self.need(Token::Punct(b';'))?;self.need(Token::Punct(b'}'))?;self.need(Token::End)?;
        
        let p=Plan{owners,root,parameters,steps,result};p.verify()?;Ok(p)
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
