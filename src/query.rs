//! A small jq-compatible query engine for the agent CLI.
//!
//! The design goal is: the agent writes *real jq* (the DSL it is most fluent
//! in) against the agent state JSON, and gets standard filter/map/select/pipe
//! semantics — no invented grammar. To keep this dependency-free and robust
//! (the canonical Rust jq crate `jaq` is still alpha/beta and unstable to
//! embed), we implement a **focused subset of jq syntax and semantics** rather
//! than the full language. Unsupported constructs fail with a clear parse or
//! runtime error instead of silently misbehaving.
//!
//! Supported:
//!   * paths: `.`, `.name`, `.a.b`, `.arr[0]`, `.obj["k"]`, `.[]`, `.a[]`,
//!     `.[1:3]`, optional `.a?`
//!   * bare identifiers as field access (`cities` == `.cities`)
//!   * pipeline `|`, array `[expr]`, object `{a: expr, b: expr}` / `{a, b}`
//!   * `select(expr)`, `map(expr)`, `map_values(expr)`
//!   * builtins: `length`, `keys`, `sort`, `sort_by(expr)`, `reverse`, `type`,
//!     `add`, `unique`, `first`, `first(expr)`, `contains`, `startswith`,
//!     `endswith`, `empty`
//!   * operators: `== != < <= > >=`, `and or not`, `+ - * /`, `//`
//!   * literals: strings, numbers, `true`, `false`, `null`, arrays
//!
//! Filters are evaluated as **streams** (a filter maps one input to zero or
//! more outputs), matching jq. `|` is the concatenation of the right filter
//! over every output of the left. Use `[expr]` to collect a stream into an
//! array, or leave it as a stream to get one output per line.

use serde_json::Value;

#[derive(Debug, Clone)]
pub struct QueryError {
    pub message: String,
}

impl QueryError {
    fn new(message: impl Into<String>) -> Self {
        QueryError {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for QueryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

/// Parse `filter` and apply it to `input`, returning the output stream.
pub fn apply(input: &Value, filter: &str) -> Result<Vec<Value>, QueryError> {
    let ast = parse(filter)?;
    Ok(eval(&ast, input))
}

/// Convenience: render `input` through `filter` and format each output on its
/// own line (JSON Lines, like jq's default output). Errors are surfaced as
/// `Err`.
pub fn apply_lines(input: &Value, filter: &str) -> Result<String, QueryError> {
    let out = apply(input, filter)?;
    Ok(out
        .iter()
        .map(|v| serde_json::to_string(v).expect("value is serializable"))
        .collect::<Vec<_>>()
        .join("\n"))
}

// --- lexer ------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(f64),
    Str(String),
    Ident(String),
    Dot,
    LBracket,
    RBracket,
    LBrace,
    RBrace,
    LParen,
    RParen,
    Comma,
    Colon,
    Pipe,
    Plus,
    Minus,
    Star,
    Slash,
    EqEq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
    Not,
    Alt, // //
    Question,
    End,
}

fn lex(src: &str) -> Result<Vec<Tok>, QueryError> {
    let b = src.as_bytes();
    let mut i = 0;
    let mut out = Vec::new();
    while i < b.len() {
        let c = b[i] as char;
        match c {
            ' ' | '\t' | '\r' | '\n' => i += 1,
            '.' => {
                out.push(Tok::Dot);
                i += 1;
            }
            '[' => {
                out.push(Tok::LBracket);
                i += 1;
            }
            ']' => {
                out.push(Tok::RBracket);
                i += 1;
            }
            '{' => {
                out.push(Tok::LBrace);
                i += 1;
            }
            '}' => {
                out.push(Tok::RBrace);
                i += 1;
            }
            '(' => {
                out.push(Tok::LParen);
                i += 1;
            }
            ')' => {
                out.push(Tok::RParen);
                i += 1;
            }
            ',' => {
                out.push(Tok::Comma);
                i += 1;
            }
            ':' => {
                out.push(Tok::Colon);
                i += 1;
            }
            '|' => {
                out.push(Tok::Pipe);
                i += 1;
            }
            '+' => {
                out.push(Tok::Plus);
                i += 1;
            }
            '-' => {
                out.push(Tok::Minus);
                i += 1;
            }
            '*' => {
                out.push(Tok::Star);
                i += 1;
            }
            '/' => {
                if i + 1 < b.len() && b[i + 1] == b'/' {
                    out.push(Tok::Alt);
                    i += 2;
                } else {
                    out.push(Tok::Slash);
                    i += 1;
                }
            }
            '?' => {
                out.push(Tok::Question);
                i += 1;
            }
            '=' => {
                if i + 1 < b.len() && b[i + 1] == b'=' {
                    out.push(Tok::EqEq);
                    i += 2;
                } else {
                    return Err(QueryError::new(format!("unexpected '=' at {}", i)));
                }
            }
            '!' => {
                if i + 1 < b.len() && b[i + 1] == b'=' {
                    out.push(Tok::Ne);
                    i += 2;
                } else {
                    return Err(QueryError::new(format!("unexpected '!' at {}", i)));
                }
            }
            '<' => {
                if i + 1 < b.len() && b[i + 1] == b'=' {
                    out.push(Tok::Le);
                    i += 2;
                } else {
                    out.push(Tok::Lt);
                    i += 1;
                }
            }
            '>' => {
                if i + 1 < b.len() && b[i + 1] == b'=' {
                    out.push(Tok::Ge);
                    i += 2;
                } else {
                    out.push(Tok::Gt);
                    i += 1;
                }
            }
            '"' => {
                let (s, next) = lex_string(src, i)?;
                out.push(Tok::Str(s));
                i = next;
            }
            '0'..='9' => {
                let (n, next) = lex_number(src, i);
                out.push(Tok::Num(n));
                i = next;
            }
            'a'..='z' | 'A'..='Z' | '_' => {
                let (s, next) = lex_ident(src, i);
                i = next;
                let kw = match s.as_str() {
                    "and" => Some(Tok::And),
                    "or" => Some(Tok::Or),
                    "not" => Some(Tok::Not),
                    _ => None,
                };
                out.push(kw.unwrap_or(Tok::Ident(s)));
            }
            _ => return Err(QueryError::new(format!("unexpected character '{c}' at {}", i))),
        }
    }
    out.push(Tok::End);
    Ok(out)
}

fn lex_string(src: &str, start: usize) -> Result<(String, usize), QueryError> {
    let bytes = src.as_bytes();
    let mut i = start + 1;
    let mut s = String::new();
    while i < bytes.len() {
        match bytes[i] {
            b'"' => return Ok((s, i + 1)),
            b'\\' => {
                i += 1;
                if i >= bytes.len() {
                    return Err(QueryError::new("unterminated string escape"));
                }
                let e = bytes[i] as char;
                s.push(match e {
                    'n' => '\n',
                    't' => '\t',
                    'r' => '\r',
                    '"' => '"',
                    '\\' => '\\',
                    other => other,
                });
                i += 1;
            }
            _ => {
                // Decode one full UTF-8 code point from the remainder.
                let ch = src[i..].chars().next().expect("valid utf-8");
                s.push(ch);
                i += ch.len_utf8();
            }
        }
    }
    Err(QueryError::new("unterminated string"))
}

fn lex_number(src: &str, start: usize) -> (f64, usize) {
    let b = src.as_bytes();
    let mut i = start;
    while i < b.len() && (b[i].is_ascii_digit() || b[i] == b'.') {
        i += 1;
    }
    let text = &src[start..i];
    let n = text.parse::<f64>().unwrap_or(0.0);
    (n, i)
}

fn lex_ident(src: &str, start: usize) -> (String, usize) {
    let b = src.as_bytes();
    let mut i = start;
    while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
        i += 1;
    }
    (src[start..i].to_string(), i)
}

// --- parser -----------------------------------------------------------------

#[derive(Debug, Clone)]
enum Filter {
    Identity,
    Path(Vec<PathSeg>),
    Pipe(Box<Filter>, Box<Filter>),
    Array(Option<Box<Filter>>),
    Object(Vec<(String, Filter)>), // (key, value-filter); key is static in v1
    Func(String, Vec<Filter>),
    Arith(Box<Filter>, Box<Filter>, BinOp),
    Compare(Box<Filter>, Box<Filter>, CmpOp),
    Bool(Box<Filter>, Box<Filter>, BoolOp),
    Not(Box<Filter>),
    Alt(Box<Filter>, Box<Filter>),
    Lit(Value),
}

#[derive(Debug, Clone)]
enum PathSeg {
    Field(String),
    Index(u64),
    Slice(Option<i64>, Option<i64>),
    Iterate,
    Dynamic(Box<Filter>),
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
}
#[derive(Debug, Clone, Copy, PartialEq)]
enum CmpOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}
#[derive(Debug, Clone, Copy, PartialEq)]
enum BoolOp {
    And,
    Or,
}

struct Parser {
    toks: Vec<Tok>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> &Tok {
        &self.toks[self.pos]
    }
    fn next(&mut self) -> Tok {
        let t = self.toks[self.pos].clone();
        if self.pos < self.toks.len() - 1 {
            self.pos += 1;
        }
        t
    }
    fn eat(&mut self, t: &Tok) -> bool {
        if self.peek() == t {
            self.next();
            true
        } else {
            false
        }
    }
    fn expect(&mut self, t: &Tok, what: &str) -> Result<(), QueryError> {
        if self.eat(t) {
            Ok(())
        } else {
            Err(QueryError::new(format!(
                "expected {what}, found {:?}",
                self.peek()
            )))
        }
    }

    /// filter := pipe ( '|' pipe )*
    fn filter(&mut self) -> Result<Filter, QueryError> {
        let mut lhs = self.pipe()?;
        while self.eat(&Tok::Pipe) {
            let rhs = self.pipe()?;
            lhs = Filter::Pipe(Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    /// pipe := or ('//' or)*          // alternative
    fn pipe(&mut self) -> Result<Filter, QueryError> {
        let mut lhs = self.or()?;
        while self.eat(&Tok::Alt) {
            let rhs = self.or()?;
            lhs = Filter::Alt(Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn or(&mut self) -> Result<Filter, QueryError> {
        let mut lhs = self.and()?;
        while self.eat(&Tok::Or) {
            let rhs = self.and()?;
            lhs = Filter::Bool(Box::new(lhs), Box::new(rhs), BoolOp::Or);
        }
        Ok(lhs)
    }

    fn and(&mut self) -> Result<Filter, QueryError> {
        let mut lhs = self.not()?;
        while self.eat(&Tok::And) {
            let rhs = self.not()?;
            lhs = Filter::Bool(Box::new(lhs), Box::new(rhs), BoolOp::And);
        }
        Ok(lhs)
    }

    fn not(&mut self) -> Result<Filter, QueryError> {
        if self.eat(&Tok::Not) {
            let inner = self.not()?;
            return Ok(Filter::Not(Box::new(inner)));
        }
        self.compare()
    }

    fn compare(&mut self) -> Result<Filter, QueryError> {
        let lhs = self.additive()?;
        let op = match self.peek() {
            Tok::EqEq => Some(CmpOp::Eq),
            Tok::Ne => Some(CmpOp::Ne),
            Tok::Lt => Some(CmpOp::Lt),
            Tok::Le => Some(CmpOp::Le),
            Tok::Gt => Some(CmpOp::Gt),
            Tok::Ge => Some(CmpOp::Ge),
            _ => None,
        };
        if let Some(op) = op {
            self.next();
            let rhs = self.additive()?;
            return Ok(Filter::Compare(Box::new(lhs), Box::new(rhs), op));
        }
        Ok(lhs)
    }

    fn additive(&mut self) -> Result<Filter, QueryError> {
        let mut lhs = self.multiplicative()?;
        loop {
            let op = match self.peek() {
                Tok::Plus => BinOp::Add,
                Tok::Minus => BinOp::Sub,
                _ => break,
            };
            self.next();
            let rhs = self.multiplicative()?;
            lhs = Filter::Arith(Box::new(lhs), Box::new(rhs), op);
        }
        Ok(lhs)
    }

    fn multiplicative(&mut self) -> Result<Filter, QueryError> {
        let mut lhs = self.unary()?;
        loop {
            let op = match self.peek() {
                Tok::Star => BinOp::Mul,
                Tok::Slash => BinOp::Div,
                _ => break,
            };
            self.next();
            let rhs = self.unary()?;
            lhs = Filter::Arith(Box::new(lhs), Box::new(rhs), op);
        }
        Ok(lhs)
    }

    fn unary(&mut self) -> Result<Filter, QueryError> {
        if self.eat(&Tok::Minus) {
            let inner = self.unary()?;
            return Ok(Filter::Arith(
                Box::new(Filter::Lit(Value::from(0.0))),
                Box::new(inner),
                BinOp::Sub,
            ));
        }
        self.postfix()
    }

    fn postfix(&mut self) -> Result<Filter, QueryError> {
        let mut f = self.primary()?;
        loop {
            match self.peek() {
                Tok::Dot => {
                    let seg = self.parse_dot_field_seg()?;
                    f = push_seg(f, seg)?;
                }
                Tok::LBracket => {
                    let seg = self.parse_bracket()?;
                    f = push_seg(f, seg)?;
                }
                Tok::Question => {
                    self.next();
                    // optional: our path eval is already lenient, so ignore.
                }
                _ => break,
            }
        }
        Ok(f)
    }

    /// Consume `.field` and return the field segment.
    fn parse_dot_field_seg(&mut self) -> Result<PathSeg, QueryError> {
        self.expect(&Tok::Dot, "'.'")?;
        if let Tok::Ident(name) = self.next() {
            Ok(PathSeg::Field(name))
        } else {
            Err(QueryError::new("expected field name after '.'"))
        }
    }

    fn parse_bracket(&mut self) -> Result<PathSeg, QueryError> {
        self.expect(&Tok::LBracket, "'['")?;
        // '[]' -> iterate
        if self.eat(&Tok::RBracket) {
            return Ok(PathSeg::Iterate);
        }
        // '[:]' / '[a:b]' slice
        let first = self.slice_bound()?;
        if self.eat(&Tok::Colon) {
            let second = self.slice_bound()?;
            self.expect(&Tok::RBracket, "']'")?;
            return Ok(PathSeg::Slice(first, second));
        }
        // single index (number literal or dynamic expr)
        if let Some(n) = first {
            self.expect(&Tok::RBracket, "']'")?;
            return Ok(PathSeg::Index(n as u64));
        }
        // dynamic: parse a filter, then ']'
        let inner = self.filter()?;
        self.expect(&Tok::RBracket, "']'")?;
        Ok(PathSeg::Dynamic(Box::new(inner)))
    }

    fn slice_bound(&mut self) -> Result<Option<i64>, QueryError> {
        if let Tok::Num(n) = self.peek() {
            let n = *n;
            self.next();
            Ok(Some(n as i64))
        } else if self.eat(&Tok::Minus) {
            // negative bound
            let n = match self.peek() {
                Tok::Num(n) => *n as i64,
                _ => 0,
            };
            if let Tok::Num(_) = self.peek() {
                self.next();
            }
            Ok(Some(-n))
        } else {
            Ok(None)
        }
    }

    fn primary(&mut self) -> Result<Filter, QueryError> {
        match self.peek().clone() {
            Tok::Dot => {
                self.next();
                // Could be `.`, `.foo`, `.[...]`, `.foo[]` etc.
                // Peek: if next is Ident -> field; if next is LBracket -> path segs
                let mut segs = Vec::new();
                match self.peek().clone() {
                    Tok::Ident(_) => {
                        let name = match self.next() {
                            Tok::Ident(n) => n,
                            _ => unreachable!(),
                        };
                        segs.push(PathSeg::Field(name));
                    }
                    Tok::LBracket => {
                        // handled by postfix loop by re-entering primary? simpler: parse the
                        // first bracket here, then postfix handles the rest.
                        let seg = self.parse_bracket()?;
                        segs.push(seg);
                    }
                    _ => {
                        // bare '.'
                        return Ok(Filter::Identity);
                    }
                }
                Ok(Filter::Path(segs))
            }
            Tok::Ident(_) => {
                let name = match self.next() {
                    Tok::Ident(n) => n,
                    _ => unreachable!(),
                };
                if self.eat(&Tok::LParen) {
                    // function call
                    let args = self.func_args()?;
                    return Ok(Filter::Func(name, args));
                }
                // bare identifier: builtin-of-zero-args, or field access.
                match name.as_str() {
                    "length" | "keys" | "keys_unsorted" | "sort" | "reverse" | "type" | "add"
                    | "unique" | "empty" => Ok(Filter::Func(name, vec![])),
                    _ => Ok(Filter::Path(vec![PathSeg::Field(name)])),
                }
            }
            Tok::Str(s) => {
                self.next();
                Ok(Filter::Lit(Value::String(s)))
            }
            Tok::Num(n) => {
                self.next();
                if n != 0.0 && self.eat(&Tok::Dot) {
                    // trailing dot after a number is unusual; ignore for safety
                }
                Ok(Filter::Lit(Value::from(n)))
            }
            Tok::LParen => {
                self.next();
                let inner = self.filter()?;
                self.expect(&Tok::RParen, "')'")?;
                Ok(inner)
            }
            Tok::LBracket => {
                self.next();
                if self.eat(&Tok::RBracket) {
                    return Ok(Filter::Lit(Value::Array(vec![])));
                }
                let inner = self.filter()?;
                self.expect(&Tok::RBracket, "']'")?;
                Ok(Filter::Array(Some(Box::new(inner))))
            }
            Tok::LBrace => {
                self.next();
                let mut fields = Vec::new();
                if self.eat(&Tok::RBrace) {
                    return Ok(Filter::Object(vec![]));
                }
                loop {
                    // key: string or ident
                    let key = match self.next() {
                        Tok::Ident(k) | Tok::Str(k) => k,
                        other => return Err(QueryError::new(format!("invalid object key {other:?}"))),
                    };
                    if self.eat(&Tok::Colon) {
                        let val = self.filter()?;
                        fields.push((key, val));
                    } else {
                        // shorthand {name} => value = .name
                        let field = key.clone();
                        fields.push((key, Filter::Path(vec![PathSeg::Field(field)])));
                    }
                    if !self.eat(&Tok::Comma) {
                        break;
                    }
                }
                self.expect(&Tok::RBrace, "'}'")?;
                Ok(Filter::Object(fields))
            }
            other => Err(QueryError::new(format!("unexpected token {other:?}"))),
        }
    }

    fn func_args(&mut self) -> Result<Vec<Filter>, QueryError> {
        // '(' already consumed
        let mut args = Vec::new();
        if self.eat(&Tok::RParen) {
            return Ok(args);
        }
        loop {
            let f = self.filter()?;
            args.push(f);
            if self.eat(&Tok::RParen) {
                break;
            }
            self.expect(&Tok::Comma, "','")?;
        }
        Ok(args)
    }
}

/// Append a path segment to `f`. If `f` is already a path, extend it; otherwise
/// wrap as `f | <seg>` so postfix indexing/field-access works on any expression.
fn push_seg(f: Filter, seg: PathSeg) -> Result<Filter, QueryError> {
    match f {
        Filter::Path(mut p) => {
            p.push(seg);
            Ok(Filter::Path(p))
        }
        other => Ok(Filter::Pipe(Box::new(other), Box::new(Filter::Path(vec![seg])))),
    }
}

fn parse(src: &str) -> Result<Filter, QueryError> {
    let toks = lex(src)?;
    let mut p = Parser { toks, pos: 0 };
    let f = p.filter()?;
    if p.peek() != &Tok::End {
        return Err(QueryError::new(format!(
            "unexpected trailing token {:?}",
            p.peek()
        )));
    }
    Ok(f)
}

// --- evaluator ---------------------------------------------------------------

fn is_truthy(v: &Value) -> bool {
    !(v.is_null() || v == &Value::Bool(false))
}

fn eval(f: &Filter, input: &Value) -> Vec<Value> {
    match f {
        Filter::Identity => vec![input.clone()],
        Filter::Lit(v) => vec![v.clone()],
        Filter::Path(segs) => eval_path(input, segs),
        Filter::Pipe(l, r) => {
            let mut out = Vec::new();
            for v in eval(l, input) {
                out.extend(eval(r, &v));
            }
            out
        }
        Filter::Array(inner) => vec![Value::Array(match inner {
            Some(arg) => eval(arg, input),
            None => vec![],
        })],
        Filter::Object(fields) => {
            // Cross-product over fields. Start with one empty object; for each
            // field, expand by combining with each value of the field filter.
            let mut acc: Vec<serde_json::Map<String, Value>> = vec![default_map()];
            for (key, valf) in fields {
                let vals = eval(valf, input);
                let mut next = Vec::new();
                for obj in &acc {
                    for v in &vals {
                        let mut o = obj.clone();
                        o.insert(key.clone(), v.clone());
                        next.push(o);
                    }
                }
                acc = next;
            }
            acc.into_iter().map(Value::Object).collect()
        }
        Filter::Not(inner) => {
            let outs = eval(inner, input);
            let b = outs.iter().any(is_truthy);
            vec![Value::Bool(!b)]
        }
        Filter::Bool(l, r, op) => {
            let lv = first_truthy(eval(l, input));
            let rv = first_truthy(eval(r, input));
            let res = match op {
                BoolOp::And => lv && rv,
                BoolOp::Or => lv || rv,
            };
            vec![Value::Bool(res)]
        }
        Filter::Compare(l, r, op) => {
            let lv = eval(l, input);
            let rv = eval(r, input);
            if lv.is_empty() || rv.is_empty() {
                return vec![Value::Bool(false)];
            }
            let a = &lv[0];
            let b = &rv[0];
            let res = match op {
                CmpOp::Eq => value_eq(a, b),
                CmpOp::Ne => !value_eq(a, b),
                CmpOp::Lt => cmp(a, b).map(|o| o == std::cmp::Ordering::Less).unwrap_or(false),
                CmpOp::Le => cmp(a, b)
                    .map(|o| o != std::cmp::Ordering::Greater)
                    .unwrap_or(false),
                CmpOp::Gt => cmp(a, b).map(|o| o == std::cmp::Ordering::Greater).unwrap_or(false),
                CmpOp::Ge => cmp(a, b)
                    .map(|o| o != std::cmp::Ordering::Less)
                    .unwrap_or(false),
            };
            vec![Value::Bool(res)]
        }
        Filter::Arith(l, r, op) => {
            let lv = eval(l, input);
            let rv = eval(r, input);
            if lv.is_empty() || rv.is_empty() {
                return vec![Value::Null];
            }
            let a = &lv[0];
            let b = &rv[0];
            let res = arith(a, b, *op);
            vec![res]
        }
        Filter::Alt(l, r) => {
            let outs = eval(l, input);
            if outs.iter().any(is_truthy) {
                outs
            } else {
                eval(r, input)
            }
        }
        Filter::Func(name, args) => eval_func(name, args, input),
    }
}

fn first_truthy(v: Vec<Value>) -> bool {
    v.first().map(is_truthy).unwrap_or(false)
}

fn default_map() -> serde_json::Map<String, Value> {
    serde_json::Map::new()
}

fn eval_path(input: &Value, segs: &[PathSeg]) -> Vec<Value> {
    let mut cur = vec![input.clone()];
    for seg in segs {
        let mut next = Vec::new();
        for v in &cur {
            match seg {
                PathSeg::Field(name) => match v {
                    Value::Object(m) => next.push(m.get(name).cloned().unwrap_or(Value::Null)),
                    // jq leniency: indexing a non-object yields null (with lenient eval)
                    _ => next.push(Value::Null),
                },
                PathSeg::Index(i) => match v {
                    Value::Array(a) => {
                        next.push(a.get(*i as usize).cloned().unwrap_or(Value::Null))
                    }
                    Value::Object(m) => next.push(
                        m.get(&i.to_string()).cloned().unwrap_or(Value::Null),
                    ),
                    _ => next.push(Value::Null),
                },
                PathSeg::Slice(a, b) => match v {
                    Value::Array(arr) => {
                        let start = a.map(|x| x as usize).unwrap_or(0).min(arr.len());
                        let end = b
                            .map(|y| {
                                let y = if y < 0 { arr.len() as i64 + y } else { y } as usize;
                                y.min(arr.len())
                            })
                            .unwrap_or(arr.len());
                        if start <= end && end <= arr.len() {
                            next.push(Value::Array(arr[start..end].to_vec()));
                        } else {
                            next.push(Value::Array(vec![]));
                        }
                    }
                    _ => next.push(Value::Array(vec![])),
                },
                PathSeg::Iterate => match v {
                    Value::Array(arr) => next.extend(arr.iter().cloned()),
                    Value::Object(m) => next.extend(m.values().cloned()),
                    _ => {} // empty
                },
                PathSeg::Dynamic(f) => {
                    let outs = eval(f, v);
                    if let Some(k) = outs.first() {
                        match v {
                            Value::Object(m) => {
                                if let Value::String(s) = k {
                                    next.push(m.get(s).cloned().unwrap_or(Value::Null));
                                }
                            }
                            Value::Array(a) => {
                                if let Some(n) = k.as_f64() {
                                    next.push(a.get(n as usize).cloned().unwrap_or(Value::Null));
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
        cur = next;
    }
    cur
}

fn cmp(a: &Value, b: &Value) -> Option<std::cmp::Ordering> {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64().partial_cmp(&y.as_f64()),
        (Value::String(x), Value::String(y)) => Some(x.cmp(y)),
        _ => None,
    }
}

/// Equality that treats numbers by value, so `2 == 2.0` (jq semantics). serde_json
/// otherwise distinguishes an integer `Number(2)` from a float `Number(2.0)`.
fn value_eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
        _ => a == b,
    }
}

fn arith(a: &Value, b: &Value, op: BinOp) -> Value {
    match op {
        BinOp::Add => match (a, b) {
            (Value::Number(x), Value::Number(y)) => {
                Value::from(x.as_f64().unwrap_or(0.0) + y.as_f64().unwrap_or(0.0))
            }
            (Value::Array(x), Value::Array(y)) => {
                let mut v = x.clone();
                v.extend(y.iter().cloned());
                Value::Array(v)
            }
            (Value::String(x), Value::String(y)) => Value::String(format!("{x}{y}")),
            (Value::Object(x), Value::Object(y)) => {
                let mut m = x.clone();
                for (k, v) in y {
                    m.insert(k.clone(), v.clone());
                }
                Value::Object(m)
            }
            _ => Value::Null,
        },
        BinOp::Sub => match (a, b) {
            (Value::Number(x), Value::Number(y)) => Value::from(
                x.as_f64().unwrap_or(0.0) - y.as_f64().unwrap_or(0.0),
            ),
            _ => Value::Null,
        },
        BinOp::Mul => match (a, b) {
            (Value::Number(x), Value::Number(y)) => Value::from(
                x.as_f64().unwrap_or(0.0) * y.as_f64().unwrap_or(0.0),
            ),
            _ => Value::Null,
        },
        BinOp::Div => match (a, b) {
            (Value::Number(x), Value::Number(y)) => Value::from(
                x.as_f64().unwrap_or(0.0) / y.as_f64().unwrap_or(0.0),
            ),
            _ => Value::Null,
        },
    }
}

fn eval_func(name: &str, args: &[Filter], input: &Value) -> Vec<Value> {
    match name {
        "length" => {
            let v = match input {
                Value::String(s) => Value::from(s.chars().count() as i64),
                Value::Array(a) => Value::from(a.len() as i64),
                Value::Object(m) => Value::from(m.len() as i64),
                Value::Number(n) => Value::from(n.as_f64().unwrap_or(0.0).abs()),
                _ => Value::from(0),
            };
            vec![v]
        }
        "keys" => keys_of(input),
        "keys_unsorted" => keys_of(input),
        "type" => vec![Value::String(type_name(input).to_string())],
        "sort" => sort_input(input),
        "reverse" => match input {
            Value::Array(a) => {
                let mut v = a.clone();
                v.reverse();
                vec![Value::Array(v)]
            }
            _ => vec![Value::Null],
        },
        "unique" => unique_input(input),
        "add" => match input {
            Value::Array(a) => {
                let mut acc = Value::Null;
                for v in a {
                    acc = arith(&acc, v, BinOp::Add);
                }
                vec![acc]
            }
            _ => vec![input.clone()],
        },
        "empty" => vec![],
        "first" => {
            if args.is_empty() {
                match input {
                    Value::Array(a) => vec![a.first().cloned().unwrap_or(Value::Null)],
                    _ => vec![input.clone()],
                }
            } else {
                let outs = eval(&args[0], input);
                vec![outs.first().cloned().unwrap_or(Value::Null)]
            }
        }
        "select" => eval_select(args, input),
        "map" => {
            if let Some(arg) = args.first() {
                let arr = match input {
                    Value::Array(a) => a.clone(),
                    _ => return vec![Value::Array(vec![])],
                };
                let mut out = Vec::new();
                for el in &arr {
                    out.extend(eval(arg, el));
                }
                vec![Value::Array(out)]
            } else {
                vec![Value::Null]
            }
        }
        "map_values" => {
            if let Some(arg) = args.first() {
                let obj = match input {
                    Value::Object(m) => m.clone(),
                    _ => return vec![Value::Object(default_map())],
                };
                let mut map = serde_json::Map::new();
                for (k, v) in obj {
                    let outs = eval(arg, &v);
                    let nv = outs.first().cloned().unwrap_or(Value::Null);
                    map.insert(k, nv);
                }
                vec![Value::Object(map)]
            } else {
                vec![Value::Null]
            }
        }
        "sort_by" => sort_by_input(args, input),
        "contains" => {
            let arg = args.first().cloned().unwrap_or(Filter::Lit(Value::Null));
            let outs = eval(&arg, input);
            let needle = outs.first().cloned().unwrap_or(Value::Null);
            vec![Value::Bool(contains(input, &needle))]
        }
        "startswith" | "endswith" => {
            let needle = args
                .first()
                .map(|f| {
                    let outs = eval(f, input);
                    outs.first().cloned().unwrap_or(Value::Null)
                })
                .unwrap_or(Value::Null);
            let b = string_match(input, &needle, name == "startswith");
            vec![Value::Bool(b)]
        }
        _ => vec![Value::Null],
    }
}

fn eval_select(args: &[Filter], input: &Value) -> Vec<Value> {
    if let Some(pred) = args.first() {
        let outs = eval(pred, input);
        if outs.iter().any(is_truthy) {
            vec![input.clone()]
        } else {
            vec![]
        }
    } else {
        vec![input.clone()]
    }
}

fn keys_of(input: &Value) -> Vec<Value> {
    match input {
        Value::Object(m) => {
            let ks: Vec<Value> = m.keys().map(|k| Value::String(k.clone())).collect();
            // jq `keys` sorts them
            sort_values(ks)
        }
        Value::Array(a) => {
            let ks: Vec<Value> = (0..a.len()).map(|i| Value::from(i as f64)).collect();
            sort_values(ks)
        }
        _ => vec![Value::Array(vec![])],
    }
}

fn sort_values(mut v: Vec<Value>) -> Vec<Value> {
    v.sort_by(|a, b| cmp(a, b).unwrap_or(std::cmp::Ordering::Equal));
    vec![Value::Array(v)]
}

fn sort_input(input: &Value) -> Vec<Value> {
    match input {
        Value::Array(a) => {
            let mut v = a.clone();
            v.sort_by(|x, y| cmp(x, y).unwrap_or(std::cmp::Ordering::Equal));
            vec![Value::Array(v)]
        }
        _ => vec![Value::Null],
    }
}

fn sort_by_input(args: &[Filter], input: &Value) -> Vec<Value> {
    let arg = args.first().cloned().unwrap_or(Filter::Identity);
    let mut keyed: Vec<(Value, Value)> = match input {
        Value::Array(a) => a
            .iter()
            .map(|el| {
                let k = eval(&arg, el).into_iter().next().unwrap_or(Value::Null);
                (k, el.clone())
            })
            .collect(),
        _ => return vec![Value::Null],
    };
    keyed.sort_by(|(ka, _), (kb, _)| cmp(ka, kb).unwrap_or(std::cmp::Ordering::Equal));
    vec![Value::Array(keyed.into_iter().map(|(_, v)| v).collect())]
}

fn unique_input(input: &Value) -> Vec<Value> {
    match input {
        Value::Array(a) => {
            let mut v = a.clone();
            v.sort_by(|x, y| cmp(x, y).unwrap_or(std::cmp::Ordering::Equal));
            v.dedup();
            vec![Value::Array(v)]
        }
        _ => vec![Value::Null],
    }
}

fn type_name(input: &Value) -> &'static str {
    match input {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn contains(hay: &Value, needle: &Value) -> bool {
    match (hay, needle) {
        (Value::String(h), Value::String(n)) => h.contains(n),
        (Value::Array(h), Value::Array(n)) => n.iter().all(|nitem| h.iter().any(|hitem| hitem == nitem)),
        (Value::Object(h), Value::Object(n)) => n.iter().all(|(k, nv)| {
            h.get(k)
                .map(|hv| hv == nv)
                .unwrap_or(false)
        }),
        _ => hay == needle,
    }
}

fn string_match(hay: &Value, needle: &Value, prefix: bool) -> bool {
    match (hay, needle) {
        (Value::String(h), Value::String(n)) => {
            if prefix {
                h.starts_with(n)
            } else {
                h.ends_with(n)
            }
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn run(filter: &str, input: &Value) -> Vec<Value> {
        apply(input, filter).expect("query should parse and run")
    }

    #[test]
    fn field_access_and_pipe() {
        let v = json!({"cities": [{"name": "A", "owner": "中国"}, {"name": "B", "owner": "美国"}]});
        assert_eq!(run(".cities | length", &v), vec![Value::from(2)]);
        assert_eq!(run(".cities[0].name", &v), vec![Value::String("A".into())]);
    }

    #[test]
    fn select_equal() {
        let v = json!({"cities": [{"name": "A", "owner": "中国"}, {"name": "B", "owner": "美国"}]});
        let out = run(".cities[] | select(.owner == \"中国\") | .name", &v);
        assert_eq!(out, vec![Value::String("A".into())]);
    }

    #[test]
    fn project_object() {
        let v = json!({"cities": [{"name": "A", "population": 10, "owner": "中国"}]});
        let out = run(".cities[] | {name, population}", &v);
        assert_eq!(
            out,
            vec![json!({"name": "A", "population": 10})]
        );
    }

    #[test]
    fn array_collect_map_sort_by() {
        let v = json!({"cities": [
            {"name": "A", "population": 30},
            {"name": "B", "population": 10},
        ]});
        let out = run("[ .cities[] ] | map(.population) | sort", &v);
        assert_eq!(out, vec![json!([10, 30])]);
        let out2 = run("[ .cities[] ] | sort_by(.population)[0].name", &v);
        assert_eq!(out2, vec![Value::String("B".into())]);
    }

    #[test]
    fn comparison_and_not() {
        let v = json!({"n": 5});
        assert_eq!(run(".n > 3 and .n < 10", &v), vec![Value::Bool(true)]);
        assert_eq!(run("not (.n > 3)", &v), vec![Value::Bool(false)]);
    }

    #[test]
    fn alt_and_arith() {
        let v = json!({"a": 2, "b": 3});
        assert_eq!(run(".a + .b", &v), vec![Value::from(5.0)]);
        assert_eq!(run(".missing // 42", &v), vec![Value::from(42.0)]);
    }

    #[test]
    fn iterate_object_values() {
        let v = json!({"m": {"x": 1, "y": 2}});
        let out = run(".m[] | .", &v);
        assert_eq!(out, vec![Value::from(1), Value::from(2)]);
    }
}
