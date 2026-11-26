//! Restricted nonescaping Option<int> + trap! + exhaustive scalar match.
//! No nominal Error allocation, general ADT ABI, or host evaluation.
//! The unobservable tag/error envelope is erased only after full typed admission.
use super::{
    Expr, MAX_DEPTH, Parser, Program, Token, integer_digits, is_java_v1_reserved_parameter_name,
    validate_ast_depth,
};

fn identifier(p: &mut Parser<'_>) -> Result<String, String> {
    let name = p.name()?;
    if is_java_v1_reserved_parameter_name(&name)
        || matches!(
            name.as_str(),
            "int"
                | "main"
                | "Option"
                | "Result"
                | "Some"
                | "None"
                | "Ok"
                | "Err"
                | "Error"
                | "on"
                | "over"
                | "unwrap"
                | "Integer"
                | "seq"
                | "sync"
                | "task"
                | "infer"
                | "trap!"
        )
    {
        return Err("E_CLOSED_OPTION_NAME".into());
    }
    Ok(name)
}

// Deliberately separate expression grammar: only signed integer literals,
// immutable entry parameters, and checked arithmetic. No implicit coercion,
// locals, calls, effects, casts, ownership operators, or aggregate escape.
fn scalar(p: &mut Parser<'_>, min: u8, depth: usize) -> Result<Expr, String> {
    if depth > MAX_DEPTH {
        return Err("E_CLOSED_OPTION_DEPTH".into());
    }
    let mut lhs = if p.take(Token::Punct(b'-'))? {
        if let Token::Number(n) = p.current {
            if integer_digits(n)? == "9223372036854775808" {
                p.next()?;
                p.node(Expr::Literal(i64::MIN))?
            } else {
                let v = scalar(p, 5, depth + 1)?;
                p.node(Expr::Neg(Box::new(v)))?
            }
        } else {
            let v = scalar(p, 5, depth + 1)?;
            p.node(Expr::Neg(Box::new(v)))?
        }
    } else if p.take(Token::Punct(b'('))? {
        let value = scalar(p, 0, depth + 1)?;
        p.expect(Token::Punct(b')'))?;
        value
    } else {
        match p.current {
            Token::Number(n) => {
                let value = integer_digits(n)?
                    .parse::<i64>()
                    .map_err(|_| "E_CLOSED_OPTION_LITERAL")?;
                p.next()?;
                p.node(Expr::Literal(value))?
            }
            Token::Word(name) => {
                let index = p
                    .parameters
                    .iter()
                    .position(|n| n == name)
                    .ok_or("E_CLOSED_OPTION_SCALAR_NAME")?;
                p.next()?;
                p.node(Expr::Param(index))?
            }
            _ => return Err("E_CLOSED_OPTION_SCALAR".into()),
        }
    };
    loop {
        let (bp, op) = match p.current {
            Token::Punct(op @ (b'+' | b'-')) => (1, op),
            Token::Punct(op @ (b'*' | b'/')) => (3, op),
            _ => break,
        };
        if bp < min {
            break;
        }
        p.next()?;
        let rhs = scalar(p, bp + 1, depth + 1)?;
        let (a, b) = (Box::new(lhs), Box::new(rhs));
        lhs = p.node(match op {
            b'+' => Expr::Add(a, b),
            b'-' => Expr::Sub(a, b),
            b'*' => Expr::Mul(a, b),
            b'/' => Expr::Div(a, b),
            _ => unreachable!(),
        })?;
    }
    Ok(lhs)
}

// A single unused, declared scalar refinement is admitted structurally.
// It has no runtime representation or authority in this closed profile.
fn signed_bound(p: &mut Parser<'_>) -> Result<i64, String> {
    let negative = p.take(Token::Punct(b'-'))?;
    let Token::Number(n) = p.current else { return Err("E_CLOSED_REFINEMENT_BOUND".into()); };
    let digits = integer_digits(n)?;
    let bound = if negative && digits == "9223372036854775808" {
        i64::MIN
    } else {
        let value = digits.parse::<i64>().map_err(|_| "E_CLOSED_REFINEMENT_BOUND")?;
        if negative { -value } else { value }
    };
    p.next()?;
    Ok(bound)
}

fn refinement(p: &mut Parser<'_>) -> Result<Option<String>, String> {
    if !p.take(Token::Word("declare"))? { return Ok(None); }
    p.expect(Token::Word("type"))?;
    let name = identifier(p)?;
    if matches!(name.as_str(), "i32" | "i64" | "bool" | "string" | "void" |
        "Integer" | "Array" | "Map" | "Tuple" | "Record" | "Entry" | "Pair" |
        "Future" | "Error" | "self" | "where" | "declare" | "type") {
        return Err("E_CLOSED_REFINEMENT_NAME".into());
    }
    p.expect(Token::Punct(b'='))?;
    p.expect(Token::Word("int"))?;
    p.expect(Token::Word("where"))?;
    p.expect(Token::Word("self"))?;
    // The shared lexer records the end of the current token. Require
    // adjacent >= and <= rather than accepting two unrelated operators.
    if p.current != Token::Punct(b'>') || p.input.as_bytes().get(p.pos) != Some(&b'=') {
        return Err("E_CLOSED_REFINEMENT_LOWER".into());
    }
    p.next()?;
    p.expect(Token::Punct(b'='))?;
    let lo = signed_bound(p)?;
    p.expect(Token::Word("and"))?;
    p.expect(Token::Word("self"))?;
    if p.current != Token::Punct(b'<') || p.input.as_bytes().get(p.pos) != Some(&b'=') {
        return Err("E_CLOSED_REFINEMENT_UPPER".into());
    }
    p.next()?;
    p.expect(Token::Punct(b'='))?;
    let hi = signed_bound(p)?;
    if lo > hi { return Err("E_CLOSED_REFINEMENT_EMPTY".into()); }
    p.expect(Token::Punct(b';'))?;
    Ok(Some(name))
}

fn variant(p: &mut Parser<'_>) -> Result<Option<Expr>, String> {
    if p.take(Token::Word("None"))? {
        return Ok(None);
    }
    p.expect(Token::Word("Some"))?;
    p.expect(Token::Punct(b'('))?;
    let value = scalar(p, 0, 0)?;
    validate_ast_depth(&value)?;
    p.expect(Token::Punct(b')'))?;
    Ok(Some(value))
}

enum OptionValue {
    Closed(Option<Expr>),
    Conditional(Box<Expr>, Option<Box<Expr>>, Option<Box<Expr>>),
}

pub(super) fn parse(text: &str) -> Result<Program, String> {
    // Original shared lexer controls all source envelope and authority checks.
    // Mode switch happens only after the first token; no other parser uses it.
    let mut p = Parser::new(text)?;
    p.closed_option_profile = true;
    let declared = refinement(&mut p)?;
    p.take(Token::Word("pub"))?;
    p.expect(Token::Word("fnc"))?;
    p.expect(Token::Word("main"))?;
    p.expect(Token::Punct(b'('))?;
    if !p.take(Token::Punct(b')'))? {
        let type_first = p.take(Token::Word("int"))?;
        let name = identifier(&mut p)?;
        if declared.as_ref() == Some(&name) {
            return Err("E_CLOSED_REFINEMENT_SHADOW".into());
        }
        if !type_first {
            p.expect(Token::Punct(b':'))?;
            p.expect(Token::Word("int"))?;
        }
        p.parameters.push(name);
        p.expect(Token::Punct(b')'))?;
    }
    if !p.take(Token::Punct(b':'))? {
        p.expect(Token::Punct(b'-'))?;
        p.expect(Token::Punct(b'>'))?;
    }
    p.expect(Token::Word("int"))?;
    p.expect(Token::Punct(b'{'))?;
    p.expect(Token::Word("val"))?;
    p.expect(Token::Word("Option"))?;
    p.expect(Token::Punct(b'<'))?;
    p.expect(Token::Word("int"))?;
    p.expect(Token::Punct(b'>'))?;
    let binding = identifier(&mut p)?;
    if p.parameters.contains(&binding) || declared.as_ref() == Some(&binding) {
        return Err("E_CLOSED_OPTION_SHADOW".into());
    }
    p.expect(Token::Punct(b'='))?;
    let option = if matches!(p.current, Token::Word("Some" | "None")) {
        OptionValue::Closed(variant(&mut p)?)
    } else {
        let left = scalar(&mut p, 0, 0)?;
        p.expect(Token::Word("eq"))?;
        let right = scalar(&mut p, 0, 0)?;
        let condition = p.node(Expr::Compare(0, Box::new(left), Box::new(right)))?;
        validate_ast_depth(&condition)?;
        p.expect(Token::Punct(b'?'))?;
        let yes = variant(&mut p)?;
        p.expect(Token::Punct(b':'))?;
        let no = variant(&mut p)?;
        if yes.is_some() == no.is_some() {
            return Err("E_CLOSED_OPTION_TAGS".into());
        }
        OptionValue::Conditional(Box::new(condition), yes.map(Box::new), no.map(Box::new))
    };
    p.expect(Token::Punct(b';'))?;
    p.expect(Token::Word("return"))?;
    p.expect(Token::Word("trap!"))?;
    if p.name()? != binding {
        return Err("E_CLOSED_OPTION_SUBJECT".into());
    }
    p.expect(Token::Punct(b'.'))?;
    p.expect(Token::Word("unwrap"))?;
    p.expect(Token::Punct(b'('))?;
    p.expect(Token::Punct(b')'))?;
    p.expect(Token::Punct(b'|'))?;
    p.expect(Token::Punct(b'>'))?;
    p.expect(Token::Word("match"))?;
    p.expect(Token::Punct(b'<'))?;
    p.expect(Token::Word("int"))?;
    p.expect(Token::Punct(b'>'))?;
    p.expect(Token::Word("over"))?;
    p.expect(Token::Word("on"))?;
    p.expect(Token::Word("Ok"))?;
    p.expect(Token::Punct(b'('))?;
    let payload = identifier(&mut p)?;
    if payload == binding || p.parameters.contains(&payload) || declared.as_ref() == Some(&payload) {
        return Err("E_CLOSED_OPTION_PATTERN_SHADOW".into());
    }
    p.expect(Token::Punct(b')'))?;
    p.expect(Token::Punct(b':'))?;
    p.expect(Token::Word("int"))?;
    p.expect(Token::Punct(b'-'))?;
    p.expect(Token::Punct(b'>'))?;
    if p.name()? != payload {
        return Err("E_CLOSED_OPTION_PAYLOAD".into());
    }
    p.expect(Token::Punct(b','))?;
    p.expect(Token::Word("on"))?;
    p.expect(Token::Word("Err"))?;
    p.expect(Token::Punct(b'('))?;
    p.expect(Token::Punct(b')'))?;
    p.expect(Token::Punct(b':'))?;
    p.expect(Token::Word("int"))?;
    p.expect(Token::Punct(b'-'))?;
    p.expect(Token::Punct(b'>'))?;
    let fallback = scalar(&mut p, 0, 0)?;
    validate_ast_depth(&fallback)?;
    p.take(Token::Punct(b','))?;
    p.expect(Token::Word("end"))?;
    p.expect(Token::Punct(b';'))?;
    p.expect(Token::Punct(b'}'))?;
    p.expect(Token::End)?;
    // All source is admitted first, including the discarded arm. Lower only
    // nonescaping, unobservable sums. The existing checked native CFG handles
    // conditional laziness and arithmetic errors; trap! does not catch those.
    let result = match option {
        OptionValue::Closed(Some(value)) => value,
        OptionValue::Closed(None) => fallback,
        OptionValue::Conditional(cond, Some(value), None) => {
            p.node(Expr::Choose(cond, value, Box::new(fallback)))?
        }
        OptionValue::Conditional(cond, None, Some(value)) => {
            p.node(Expr::Choose(cond, Box::new(fallback), value))?
        }
        _ => return Err("E_CLOSED_OPTION_TAGS".into()),
    };
    validate_ast_depth(&result)?;
    Ok(Program {
        parameters: p.parameters,
        locals: vec![],
        result,
        trace_steps: vec![],
    })
}
