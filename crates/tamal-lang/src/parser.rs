//! Parser: tokens → the AST — a `Module` of `const` items, `fn` items and a
//! `Test` of statements (`pass`, `fail N`, `send`/`crc_region`, or a verbatim
//! raw instruction), plus the compile-time `Expr` grammar (`[bytes]`, `++`,
//! `^`, builtin and `fn` calls).

use crate::lexer::{Tok, Token};
use tamal_asm::{Diagnostic, Span};

/// A parsed `.tam` module: `const` items, `fn` items, `proc` items, and tests.
#[derive(Debug, Clone)]
pub struct Module {
    pub consts: Vec<Const>,
    pub fns: Vec<FnDef>,
    pub procs: Vec<ProcDef>,
    pub tests: Vec<Test>,
}

/// A `const NAME = expr` item.
#[derive(Debug, Clone)]
pub struct Const {
    pub name: String,
    pub name_span: Span,
    pub value: Expr,
}

/// A single test = one program entry point.
#[derive(Debug, Clone)]
pub struct Test {
    pub name: String,
    pub name_span: Span,
    pub stmts: Vec<Stmt>,
}

/// A Plan-1 statement.
#[derive(Debug, Clone)]
pub enum Stmt {
    /// `pass` → `halt 0x00`.
    Pass,
    /// `fail N` → `halt N`. `code` is the verbatim number lexeme.
    Fail { code: String, span: Span },
    /// A verbatim instruction: `mnemonic op, op, …` emitted 1:1 to asm text.
    Raw {
        mnemonic: String,
        operands: Vec<String>,
        span: Span,
    },
    /// `send <bytes-expr>` optionally followed by `+ crc8`.
    Send {
        bytes: Expr,
        append_crc: bool,
        span: Span,
    },
    /// `crc_region { send <expr> … }` — appends a CRC-8 over exactly the bytes
    /// the block emits, in order.
    CrcRegion { sends: Vec<Expr>, span: Span },
    /// `config role, io, sck, alert` → `set_config …` (keywords pass through
    /// verbatim; the assembler validates them and the v1 restriction).
    Config {
        role: String,
        io: String,
        sck: String,
        alert: String,
        span: Span,
    },
    /// `frame { … }` — a CS scope: `cs_assert` before the body, `cs_deassert`
    /// on every exit (D9). The body reuses the ordinary statement grammar; the
    /// emitter enforces which statements are legal inside a frame.
    Frame { body: Vec<Stmt>, span: Span },
    /// `recv a, b, _` (names / discards) or `recv N` (N discards) → one
    /// `get_byte` per target into an allocated register.
    Recv {
        targets: Vec<RecvTarget>,
        span: Span,
    },
    /// `wait_state [name]` — poll past WAIT_STATE; consumes the response-code
    /// byte (D12). `name` binds the terminal (non-WAIT_STATE) byte.
    WaitState { bind: Option<String>, span: Span },
    /// `expect crc else <byte>` — consume the trailing CRC byte, latch the RX
    /// residue, and (via the enclosing `frame`) branch to a `fail <byte>` after
    /// CS deasserts (D9/D12). Only legal inside a `frame`.
    Expect { else_code: Expr, span: Span },
    /// `name(args)` — a `proc` call, expanded inline at this point.
    Call {
        name: String,
        name_span: Span,
        args: Vec<Arg>,
        span: Span,
    },
    /// `repeat N { … }` — a compile-time unroll: the body is emitted `N` times.
    /// There is no loop counter and no branch.
    Repeat {
        count: Expr,
        body: Vec<Stmt>,
        span: Span,
    },
}

/// One destination of a `recv`: a named binding or a `_` discard.
#[derive(Debug, Clone)]
pub enum RecvTarget {
    /// Bind the received byte to a named register for the enclosing scope.
    Name(String),
    /// Read and discard (a scratch register, freed immediately).
    Discard,
}

/// One argument at a call site: positional, or `name = value` (named).
#[derive(Debug, Clone)]
pub struct Arg {
    /// `Some(name)` for a named argument (`command(ndata = 0)`), `None` when
    /// the argument is positional.
    pub name: Option<String>,
    /// The argument's value expression.
    pub value: Expr,
    /// The span covering the whole argument (the name too, when named).
    pub span: Span,
}

/// A compile-time parameter or return type. (`bool` and `reg` arrive with the
/// conditionals in Plan 4b.)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Type {
    /// One wire byte, `0..=255`.
    Byte,
    /// A compile-time integer.
    Int,
    /// A compile-time byte string (the packet type).
    Bytes,
}

impl Type {
    /// The spelling used in source, and in diagnostics.
    pub fn name(self) -> &'static str {
        match self {
            Type::Byte => "byte",
            Type::Int => "int",
            Type::Bytes => "bytes",
        }
    }
}

/// One declared parameter of a `fn`/`proc`: `name: type`, with an optional
/// default (`err: byte = 0x11`).
#[derive(Debug, Clone)]
pub struct Param {
    /// The parameter name, bound in the callee's scope.
    pub name: String,
    /// Its declared type; every bound value is checked against it.
    pub ty: Type,
    /// The default used when a call does not bind this parameter.
    pub default: Option<Expr>,
    /// Where a diagnostic about this parameter is anchored: the name, extended
    /// through the default expression when there is one. `bind_args` anchors
    /// its default-type-check here, so with a default the caret covers the
    /// value being complained about; without one it collapses to just the name.
    pub span: Span,
}

/// A `fn NAME(params) -> type { expr }` item: a pure, compile-time function.
///
/// A call is replaced by the value the body evaluates to — the tamal ISA has no
/// `call`/`ret` and no stack, so there is no runtime linkage to invent (D2).
#[derive(Debug, Clone)]
pub struct FnDef {
    pub name: String,
    pub name_span: Span,
    pub params: Vec<Param>,
    /// The declared return type; the body's value is checked against it.
    pub ret: Type,
    /// The body — a single expression.
    pub body: Expr,
}

/// A `proc NAME(params) { stmts }` item: a procedure that emits bus activity.
///
/// It is **inlined** at every call site — the tamal ISA has no `call`/`ret`, no
/// stack and no data memory, so there is no runtime linkage to invent (D2).
#[derive(Debug, Clone)]
pub struct ProcDef {
    pub name: String,
    pub name_span: Span,
    pub params: Vec<Param>,
    pub body: Vec<Stmt>,
}

/// A compile-time expression.
#[derive(Debug, Clone)]
pub enum Expr {
    /// An integer literal (already parsed from its lexeme).
    Int { value: i64, span: Span },
    /// A reference to a `const` by name.
    Name { name: String, span: Span },
    /// A byte-string literal `[e, e, …]` (each element is a byte).
    Bytes { elems: Vec<Expr>, span: Span },
    /// A call: a builtin (`crc8(e)`, `len`, `lo`, `hi`) or a user `fn`, with
    /// positional and/or named arguments.
    Call {
        func: String,
        args: Vec<Arg>,
        span: Span,
    },
    /// A binary operation (`^` on ints, `++` on bytes).
    Binary {
        op: BinOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
        span: Span,
    },
}

/// The binary operators available in the Plan-2 subset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    /// `^` — bitwise xor on integers (for deliberate-wrong CRCs).
    Xor,
    /// `++` — bytes concatenation.
    Concat,
}

impl Expr {
    /// The source span covering this expression.
    pub fn span(&self) -> Span {
        match self {
            Expr::Int { span, .. }
            | Expr::Name { span, .. }
            | Expr::Bytes { span, .. }
            | Expr::Call { span, .. }
            | Expr::Binary { span, .. } => span.clone(),
        }
    }
}

/// Parse a numeric lexeme (`0x..`, `0b..`, decimal, `_` separators allowed).
fn parse_number(lexeme: &str) -> Option<i64> {
    let s: String = lexeme.chars().filter(|&c| c != '_').collect();
    if let Some(h) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        i64::from_str_radix(h, 16).ok()
    } else if let Some(b) = s.strip_prefix("0b").or_else(|| s.strip_prefix("0B")) {
        i64::from_str_radix(b, 2).ok()
    } else {
        s.parse::<i64>().ok()
    }
}

/// The words `parse_stmt` matches before it would ever consider a `proc` call
/// or a raw instruction.
///
/// A callable may not take one of these names: the statement grammar wins, so
/// `proc send() { … }` would be *definable but uncallable* — `send()` parses as
/// the `send` statement and fails on the empty expression, with nothing to
/// suggest the definition is unreachable. The driver rejects such a name up
/// front instead.
///
/// This list must mirror `parse_stmt`'s match arms, but only *one* direction is
/// checked automatically. `stmt_keywords_shadow_a_call` iterates this list, so
/// it fails when a word here stops being a statement — i.e. when a `parse_stmt`
/// arm is **removed** and the list is not updated. The other direction is
/// manual: **adding** an arm without listing the word here passes the whole
/// suite silently, leaving a callable name that is definable but uncallable.
/// Add the word by hand, and pin it with a test (see `a_proc_may_not_be_named_*`
/// in the driver).
pub const STMT_KEYWORDS: &[&str] = &[
    "pass",
    "fail",
    "send",
    "crc_region",
    "config",
    "frame",
    "recv",
    "wait_state",
    "expect",
    "repeat",
];

/// Parse tokens into a [`Module`], or return diagnostics.
pub fn parse(src: &str, toks: &[Token]) -> Result<Module, Vec<Diagnostic>> {
    let mut p = P { src, toks, i: 0 };
    let mut consts = Vec::new();
    let mut fns = Vec::new();
    let mut procs = Vec::new();
    let mut tests = Vec::new();
    p.skip_newlines();
    while p.peek() != Tok::Eof {
        let kw = p.expect_ident()?;
        match p.lexeme(&kw) {
            "const" => consts.push(p.parse_const()?),
            "fn" => fns.push(p.parse_fn()?),
            "proc" => procs.push(p.parse_proc()?),
            "test" => tests.push(p.parse_test()?),
            other => {
                return Err(vec![Diagnostic::error(
                    kw,
                    format!("expected `const`, `fn`, `proc`, or `test`, found `{other}`"),
                )]);
            }
        }
        p.skip_newlines();
    }
    Ok(Module {
        consts,
        fns,
        procs,
        tests,
    })
}

struct P<'a> {
    src: &'a str,
    toks: &'a [Token],
    i: usize,
}

impl<'a> P<'a> {
    fn peek(&self) -> Tok {
        self.toks.get(self.i).map(|t| t.kind).unwrap_or(Tok::Eof)
    }

    /// The token `n` positions ahead (`peek_at(0) == peek()`).
    fn peek_at(&self, n: usize) -> Tok {
        self.toks
            .get(self.i + n)
            .map(|t| t.kind)
            .unwrap_or(Tok::Eof)
    }

    fn span(&self) -> Span {
        self.toks
            .get(self.i)
            .map(|t| t.span.clone())
            .unwrap_or(self.src.len()..self.src.len())
    }

    fn lexeme(&self, span: &Span) -> &'a str {
        &self.src[span.clone()]
    }

    fn skip_newlines(&mut self) {
        while self.peek() == Tok::Newline {
            self.i += 1;
        }
    }

    fn expect(&mut self, kind: Tok, what: &str) -> Result<Token, Vec<Diagnostic>> {
        if self.peek() == kind {
            let t = self.toks[self.i].clone();
            self.i += 1;
            Ok(t)
        } else {
            Err(vec![Diagnostic::error(
                self.span(),
                format!("expected {what}"),
            )])
        }
    }

    fn expect_ident(&mut self) -> Result<Span, Vec<Diagnostic>> {
        Ok(self.expect(Tok::Ident, "an identifier")?.span)
    }

    /// Parse statements up to and including the matching `}` (the `{` is
    /// already consumed); returns the body and the closing brace's end offset.
    /// `what` names the construct in the unterminated-block diagnostic.
    fn parse_block(&mut self, what: &str) -> Result<(Vec<Stmt>, usize), Vec<Diagnostic>> {
        let mut body = Vec::new();
        loop {
            self.skip_newlines();
            match self.peek() {
                Tok::RBrace => {
                    let end = self.span().end;
                    self.i += 1;
                    return Ok((body, end));
                }
                Tok::Eof => {
                    return Err(vec![Diagnostic::error(
                        self.span(),
                        format!("unexpected end of file: missing `}}` for `{what}`"),
                    )]);
                }
                _ => body.push(self.parse_stmt()?),
            }
        }
    }

    fn parse_test(&mut self) -> Result<Test, Vec<Diagnostic>> {
        let name_span = self.expect_ident()?;
        let name = self.lexeme(&name_span).to_string();
        self.expect(Tok::LBrace, "`{`")?;
        let (stmts, _) = self.parse_block("test")?;
        Ok(Test {
            name,
            name_span,
            stmts,
        })
    }

    fn parse_const(&mut self) -> Result<Const, Vec<Diagnostic>> {
        let name_span = self.expect_ident()?;
        let name = self.lexeme(&name_span).to_string();
        self.expect(Tok::Eq, "`=`")?;
        let value = self.parse_expr()?;
        self.end_stmt()?;
        Ok(Const {
            name,
            name_span,
            value,
        })
    }

    fn parse_fn(&mut self) -> Result<FnDef, Vec<Diagnostic>> {
        let name_span = self.expect_ident()?;
        let name = self.lexeme(&name_span).to_string();
        self.expect(Tok::LParen, "`(`")?;
        let params = self.parse_params()?;
        self.expect(Tok::RParen, "`)`")?;
        self.expect(Tok::Arrow, "`->` and a return type")?;
        let ret = self.parse_type()?;
        self.expect(Tok::LBrace, "`{`")?;
        self.skip_newlines();
        let body = self.parse_expr()?;
        self.skip_newlines();
        self.expect(Tok::RBrace, "`}`")?;
        self.end_stmt()?;
        Ok(FnDef {
            name,
            name_span,
            params,
            ret,
            body,
        })
    }

    fn parse_proc(&mut self) -> Result<ProcDef, Vec<Diagnostic>> {
        let name_span = self.expect_ident()?;
        let name = self.lexeme(&name_span).to_string();
        self.expect(Tok::LParen, "`(`")?;
        let params = self.parse_params()?;
        self.expect(Tok::RParen, "`)`")?;
        self.expect(Tok::LBrace, "`{`")?;
        let (body, _) = self.parse_block("proc")?;
        self.end_stmt()?;
        Ok(ProcDef {
            name,
            name_span,
            params,
            body,
        })
    }

    /// Parse a parameter list up to but not including the closing `)` (the `(`
    /// is already consumed). Newlines inside the parens are not statement
    /// terminators and a trailing comma is allowed.
    ///
    /// Rejects a repeated parameter name: that is `bind_args`' documented
    /// precondition, and without this check a named argument would bind the
    /// first of the pair while the second went silently unchecked. The scan is
    /// over `params`, a `Vec`, in declaration order — a hash set would report
    /// whichever name it happened to visit first, and that ordering must never
    /// reach a diagnostic.
    fn parse_params(&mut self) -> Result<Vec<Param>, Vec<Diagnostic>> {
        let mut params: Vec<Param> = Vec::new();
        self.skip_newlines();
        while self.peek() != Tok::RParen {
            let name_span = self.expect_ident()?;
            let name = self.lexeme(&name_span).to_string();
            // Checked before the type and default are parsed, so a repeated
            // name is reported as such rather than being pre-empted by an
            // unrelated later problem (`fn f(x: int, x: word)` must blame the
            // duplicate `x`, not the unknown type `word`).
            if params.iter().any(|p| p.name == name) {
                // Anchored on the second declaration — the one to delete.
                return Err(vec![Diagnostic::error(
                    name_span,
                    format!("duplicate parameter `{name}`"),
                )]);
            }
            self.expect(Tok::Colon, "`:` and a type")?;
            let ty = self.parse_type()?;
            let mut end = name_span.end;
            let mut default = None;
            if self.peek() == Tok::Eq {
                self.i += 1;
                let e = self.parse_expr()?;
                end = e.span().end;
                default = Some(e);
            }
            params.push(Param {
                name,
                ty,
                default,
                span: name_span.start..end,
            });
            self.skip_newlines();
            if self.peek() == Tok::Comma {
                self.i += 1;
                self.skip_newlines();
            } else {
                break;
            }
        }
        Ok(params)
    }

    fn parse_type(&mut self) -> Result<Type, Vec<Diagnostic>> {
        let sp = self.expect_ident()?;
        match self.lexeme(&sp) {
            "byte" => Ok(Type::Byte),
            "int" => Ok(Type::Int),
            "bytes" => Ok(Type::Bytes),
            other => Err(vec![
                Diagnostic::error(sp, format!("unknown type `{other}`"))
                    .with_help("the types are byte, int, bytes"),
            ]),
        }
    }

    /// Parse one statement.
    ///
    /// The words in [`STMT_KEYWORDS`] are matched here first; anything else is
    /// a `proc` call (when followed by `(`) or a raw instruction. Keep the two
    /// in step — see [`STMT_KEYWORDS`].
    fn parse_stmt(&mut self) -> Result<Stmt, Vec<Diagnostic>> {
        let head = self.expect_ident()?;
        let word = self.lexeme(&head).to_string();
        match word.as_str() {
            "pass" => {
                self.end_stmt()?;
                Ok(Stmt::Pass)
            }
            "fail" => {
                let n = self.expect(Tok::Number, "a verdict number")?;
                let code = self.lexeme(&n.span).to_string();
                let span = head.start..n.span.end;
                self.end_stmt()?;
                Ok(Stmt::Fail { code, span })
            }
            "send" => {
                let bytes = self.parse_expr()?;
                let mut end = bytes.span().end;
                let mut append_crc = false;
                if self.peek() == Tok::Plus {
                    self.i += 1;
                    let kw = self.expect_ident()?;
                    if self.lexeme(&kw) != "crc8" {
                        return Err(vec![Diagnostic::error(
                            kw,
                            "expected `crc8` after `+` in a `send`",
                        )]);
                    }
                    append_crc = true;
                    end = kw.end;
                }
                self.end_stmt()?;
                Ok(Stmt::Send {
                    bytes,
                    append_crc,
                    span: head.start..end,
                })
            }
            "crc_region" => {
                self.expect(Tok::LBrace, "`{`")?;
                let mut sends = Vec::new();
                let end = loop {
                    self.skip_newlines();
                    match self.peek() {
                        Tok::RBrace => {
                            let e = self.span().end;
                            self.i += 1;
                            break e;
                        }
                        Tok::Eof => {
                            return Err(vec![Diagnostic::error(
                                self.span(),
                                "unexpected end of file: missing `}` for `crc_region`",
                            )]);
                        }
                        _ => {
                            let kw = self.expect_ident()?;
                            if self.lexeme(&kw) != "send" {
                                return Err(vec![Diagnostic::error(
                                    kw,
                                    "only `send` statements are allowed in a `crc_region`",
                                )]);
                            }
                            sends.push(self.parse_expr()?);
                            self.end_stmt()?;
                        }
                    }
                };
                self.end_stmt()?;
                Ok(Stmt::CrcRegion {
                    sends,
                    span: head.start..end,
                })
            }
            "config" => {
                let role = self.expect_ident()?;
                self.expect(Tok::Comma, "`,`")?;
                let io = self.expect_ident()?;
                self.expect(Tok::Comma, "`,`")?;
                let sck = self.expect_ident()?;
                self.expect(Tok::Comma, "`,`")?;
                let alert = self.expect_ident()?;
                let span = head.start..alert.end;
                self.end_stmt()?;
                Ok(Stmt::Config {
                    role: self.lexeme(&role).to_string(),
                    io: self.lexeme(&io).to_string(),
                    sck: self.lexeme(&sck).to_string(),
                    alert: self.lexeme(&alert).to_string(),
                    span,
                })
            }
            "frame" => {
                self.expect(Tok::LBrace, "`{`")?;
                let (body, end) = self.parse_block("frame")?;
                self.end_stmt()?;
                Ok(Stmt::Frame {
                    body,
                    span: head.start..end,
                })
            }
            "recv" => {
                let mut targets = Vec::new();
                let end;
                if self.peek() == Tok::Number {
                    // `recv N` — N discards.
                    let n = self.expect(Tok::Number, "a byte count")?;
                    let count = parse_number(self.lexeme(&n.span))
                        .filter(|&c| c >= 0)
                        .ok_or_else(|| {
                            vec![Diagnostic::error(n.span.clone(), "invalid recv count")]
                        })?;
                    for _ in 0..count {
                        targets.push(RecvTarget::Discard);
                    }
                    end = n.span.end;
                } else {
                    // `recv a, _, b` — a name/discard list.
                    let mut last;
                    loop {
                        let sp = self.expect_ident()?;
                        last = sp.end;
                        let name = self.lexeme(&sp).to_string();
                        targets.push(if name == "_" {
                            RecvTarget::Discard
                        } else {
                            RecvTarget::Name(name)
                        });
                        if self.peek() == Tok::Comma {
                            self.i += 1;
                            self.skip_newlines();
                            continue;
                        }
                        break;
                    }
                    end = last;
                }
                self.end_stmt()?;
                Ok(Stmt::Recv {
                    targets,
                    span: head.start..end,
                })
            }
            "wait_state" => {
                let mut end = head.end;
                let mut bind = None;
                if self.peek() == Tok::Ident {
                    let sp = self.span();
                    self.i += 1;
                    bind = Some(self.lexeme(&sp).to_string());
                    end = sp.end;
                }
                self.end_stmt()?;
                Ok(Stmt::WaitState {
                    bind,
                    span: head.start..end,
                })
            }
            "expect" => {
                let crc = self.expect_ident()?;
                if self.lexeme(&crc) != "crc" {
                    return Err(vec![Diagnostic::error(
                        crc,
                        "expected `crc` after `expect` (the only check in v1)",
                    )]);
                }
                let els = self.expect_ident()?;
                if self.lexeme(&els) != "else" {
                    return Err(vec![Diagnostic::error(
                        els,
                        "expected `else <byte>` after `expect crc`",
                    )]);
                }
                let else_code = self.parse_expr()?;
                let span = head.start..else_code.span().end;
                self.end_stmt()?;
                Ok(Stmt::Expect { else_code, span })
            }
            "repeat" => {
                let count = self.parse_expr()?;
                self.expect(Tok::LBrace, "`{`")?;
                let (body, end) = self.parse_block("repeat")?;
                self.end_stmt()?;
                Ok(Stmt::Repeat {
                    count,
                    body,
                    span: head.start..end,
                })
            }
            _ => {
                // `name(...)` is a `proc` call, not a raw instruction: an asm
                // operand never starts with `(`.
                if self.peek() == Tok::LParen {
                    self.i += 1;
                    let args = self.parse_args()?;
                    let close = self.expect(Tok::RParen, "`)`")?;
                    let span = head.start..close.span.end;
                    self.end_stmt()?;
                    return Ok(Stmt::Call {
                        name: word,
                        name_span: head,
                        args,
                        span,
                    });
                }
                let mut operands = Vec::new();
                let mut end = head.end;
                if !self.at_stmt_end() {
                    loop {
                        let op = self.parse_operand()?;
                        end = op.end;
                        operands.push(self.lexeme(&op).to_string());
                        if self.peek() == Tok::Comma {
                            self.i += 1;
                            continue;
                        }
                        break;
                    }
                }
                self.end_stmt()?;
                Ok(Stmt::Raw {
                    mnemonic: word,
                    operands,
                    span: head.start..end,
                })
            }
        }
    }

    fn parse_operand(&mut self) -> Result<Span, Vec<Diagnostic>> {
        match self.peek() {
            Tok::Number | Tok::Ident => {
                let s = self.span();
                self.i += 1;
                Ok(s)
            }
            _ => Err(vec![Diagnostic::error(
                self.span(),
                "expected an operand (number or register/name)",
            )]),
        }
    }

    /// Parse an expression: `++`-concat (lowest) over `^`-xor over primaries.
    fn parse_expr(&mut self) -> Result<Expr, Vec<Diagnostic>> {
        self.parse_concat()
    }

    fn parse_concat(&mut self) -> Result<Expr, Vec<Diagnostic>> {
        let mut lhs = self.parse_xor()?;
        while self.peek() == Tok::PlusPlus {
            self.i += 1;
            let rhs = self.parse_xor()?;
            let span = lhs.span().start..rhs.span().end;
            lhs = Expr::Binary {
                op: BinOp::Concat,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
                span,
            };
        }
        Ok(lhs)
    }

    fn parse_xor(&mut self) -> Result<Expr, Vec<Diagnostic>> {
        let mut lhs = self.parse_primary()?;
        while self.peek() == Tok::Caret {
            self.i += 1;
            let rhs = self.parse_primary()?;
            let span = lhs.span().start..rhs.span().end;
            lhs = Expr::Binary {
                op: BinOp::Xor,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
                span,
            };
        }
        Ok(lhs)
    }

    fn parse_primary(&mut self) -> Result<Expr, Vec<Diagnostic>> {
        match self.peek() {
            Tok::Number => {
                let sp = self.span();
                self.i += 1;
                let value = parse_number(self.lexeme(&sp))
                    .ok_or_else(|| vec![Diagnostic::error(sp.clone(), "invalid number literal")])?;
                Ok(Expr::Int { value, span: sp })
            }
            Tok::Ident => {
                let sp = self.span();
                self.i += 1;
                let name = self.lexeme(&sp).to_string();
                if self.peek() == Tok::LParen {
                    self.i += 1;
                    let args = self.parse_args()?;
                    let close = self.expect(Tok::RParen, "`)`")?;
                    Ok(Expr::Call {
                        func: name,
                        args,
                        span: sp.start..close.span.end,
                    })
                } else {
                    Ok(Expr::Name { name, span: sp })
                }
            }
            Tok::LBracket => {
                let start = self.span().start;
                self.i += 1;
                // Newlines inside `[...]` are not statement terminators, and a
                // trailing comma before `]` is allowed.
                let mut elems = Vec::new();
                self.skip_newlines();
                while self.peek() != Tok::RBracket {
                    elems.push(self.parse_expr()?);
                    self.skip_newlines();
                    if self.peek() == Tok::Comma {
                        self.i += 1;
                        self.skip_newlines();
                    } else {
                        break;
                    }
                }
                let close = self.expect(Tok::RBracket, "`]`")?;
                Ok(Expr::Bytes {
                    elems,
                    span: start..close.span.end,
                })
            }
            Tok::LParen => {
                self.i += 1;
                self.skip_newlines();
                let e = self.parse_expr()?;
                self.skip_newlines();
                self.expect(Tok::RParen, "`)`")?;
                Ok(e)
            }
            _ => Err(vec![Diagnostic::error(
                self.span(),
                "expected an expression",
            )]),
        }
    }

    /// Parse a call's argument list, up to but not including the closing `)`
    /// (the `(` is already consumed). Newlines *between arguments* are not
    /// statement terminators and a trailing comma before `)` is allowed.
    fn parse_args(&mut self) -> Result<Vec<Arg>, Vec<Diagnostic>> {
        let mut args = Vec::new();
        self.skip_newlines();
        while self.peek() != Tok::RParen {
            args.push(self.parse_arg()?);
            self.skip_newlines();
            if self.peek() == Tok::Comma {
                self.i += 1;
                self.skip_newlines();
            } else {
                break;
            }
        }
        Ok(args)
    }

    /// One argument: `name = expr` when an identifier is directly followed by
    /// `=`, else a positional expression.
    fn parse_arg(&mut self) -> Result<Arg, Vec<Diagnostic>> {
        if self.peek() == Tok::Ident && self.peek_at(1) == Tok::Eq {
            let sp = self.span();
            let name = self.lexeme(&sp).to_string();
            self.i += 2; // the name and the `=`
            self.skip_newlines();
            let value = self.parse_expr()?;
            let span = sp.start..value.span().end;
            Ok(Arg {
                name: Some(name),
                value,
                span,
            })
        } else {
            let value = self.parse_expr()?;
            let span = value.span();
            Ok(Arg {
                name: None,
                value,
                span,
            })
        }
    }

    fn at_stmt_end(&self) -> bool {
        matches!(self.peek(), Tok::Newline | Tok::RBrace | Tok::Eof)
    }

    fn end_stmt(&mut self) -> Result<(), Vec<Diagnostic>> {
        match self.peek() {
            Tok::Newline => {
                self.i += 1;
                Ok(())
            }
            Tok::RBrace | Tok::Eof => Ok(()),
            _ => Err(vec![Diagnostic::error(
                self.span(),
                "expected end of statement (newline or `}`)",
            )]),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::lex;

    fn parse_ok(src: &str) -> Module {
        let toks = lex(src).unwrap();
        parse(src, &toks).unwrap()
    }

    #[test]
    fn parses_pass() {
        let m = parse_ok("test smoke {\n  pass\n}\n");
        assert_eq!(m.tests.len(), 1);
        assert_eq!(m.tests[0].name, "smoke");
        assert!(matches!(m.tests[0].stmts.as_slice(), [Stmt::Pass]));
    }

    #[test]
    fn parses_fail_with_code() {
        let m = parse_ok("test t {\n  fail 0x11\n}\n");
        match &m.tests[0].stmts[0] {
            Stmt::Fail { code, .. } => assert_eq!(code, "0x11"),
            other => panic!("expected Fail, got {other:?}"),
        }
    }

    #[test]
    fn parses_raw_instruction_with_operands() {
        let m = parse_ok("test probe {\n  mark 1, x1\n  pass\n}\n");
        match &m.tests[0].stmts[0] {
            Stmt::Raw {
                mnemonic, operands, ..
            } => {
                assert_eq!(mnemonic, "mark");
                assert_eq!(operands, &["1".to_string(), "x1".to_string()]);
            }
            other => panic!("expected Raw, got {other:?}"),
        }
        assert!(matches!(m.tests[0].stmts[1], Stmt::Pass));
    }

    #[test]
    fn parses_zero_operand_instruction() {
        let m = parse_ok("test t {\n  cs_assert\n  pass\n}\n");
        match &m.tests[0].stmts[0] {
            Stmt::Raw {
                mnemonic, operands, ..
            } => {
                assert_eq!(mnemonic, "cs_assert");
                assert!(operands.is_empty());
            }
            other => panic!("expected Raw, got {other:?}"),
        }
    }

    #[test]
    fn missing_close_brace_is_an_error() {
        let toks = lex("test t {\n  pass\n").unwrap();
        let err = parse("test t {\n  pass\n", &toks).unwrap_err();
        assert!(err[0].message.contains("missing `}`"));
    }

    #[test]
    fn missing_test_keyword_is_an_error() {
        let toks = lex("smoke {\n  pass\n}\n").unwrap();
        let err = parse("smoke {\n  pass\n}\n", &toks).unwrap_err();
        // Names `fn` explicitly so dropping it from the item match is caught;
        // still open-ended enough for Task 6, which extends the list to
        // ``expected `const`, `fn`, `proc`, or `test``.
        assert!(
            err[0].message.contains("expected `const`, `fn`,"),
            "got: {}",
            err[0].message
        );
    }

    #[test]
    fn parses_const_item() {
        let m = parse_ok("const PUT_IORD1 = 0x44\ntest t {\n  pass\n}\n");
        assert_eq!(m.consts.len(), 1);
        assert_eq!(m.consts[0].name, "PUT_IORD1");
        assert_eq!(m.tests.len(), 1);
    }

    #[test]
    fn parses_fn_item() {
        let m = parse_ok(
            "fn iord_hdr(op: byte, addr: int) -> bytes { [op, hi(addr), lo(addr)] }\ntest t {\n pass\n}\n",
        );
        assert_eq!(m.fns.len(), 1);
        let f = &m.fns[0];
        assert_eq!(f.name, "iord_hdr");
        assert_eq!(f.ret, Type::Bytes);
        assert_eq!(f.params.len(), 2);
        assert_eq!(f.params[0].name, "op");
        assert_eq!(f.params[0].ty, Type::Byte);
        assert_eq!(f.params[1].ty, Type::Int);
        assert!(f.params.iter().all(|p| p.default.is_none()));
    }

    #[test]
    fn parses_fn_param_default() {
        let src = "fn f(err: byte = 0x11) -> byte { err }\ntest t {\n pass\n}\n";
        let m = parse_ok(src);
        let p = &m.fns[0].params[0];
        assert!(p.default.is_some());
        // The recorded span must reach the END of the default expression, not
        // stop at the name: `bind_args` anchors its default-type-check
        // diagnostic on `Param::span`, so a shrinking caret would point the
        // author at the name when the value is what needs fixing.
        assert_eq!(&src[p.span.clone()], "err: byte = 0x11");
    }

    #[test]
    fn rejects_an_unknown_param_type() {
        let src = "fn f(x: word) -> byte { x }\ntest t {\n pass\n}\n";
        let toks = lex(src).unwrap();
        let err = parse(src, &toks).unwrap_err();
        assert!(err[0].message.contains("unknown type `word`"));
    }

    #[test]
    fn rejects_a_parameter_with_no_type_annotation() {
        // Every parameter is typed; the `:` is not optional.
        let src = "fn f(x int) -> byte { x }\ntest t {\n pass\n}\n";
        let toks = lex(src).unwrap();
        let err = parse(src, &toks).unwrap_err();
        assert!(
            err[0].message.contains("expected `:` and a type"),
            "got: {}",
            err[0].message
        );
    }

    #[test]
    fn rejects_a_fn_with_no_return_type() {
        // A `fn` returns a value, so its return type is mandatory.
        let src = "fn f(x: int) byte { x }\ntest t {\n pass\n}\n";
        let toks = lex(src).unwrap();
        let err = parse(src, &toks).unwrap_err();
        assert!(
            err[0].message.contains("expected `->` and a return type"),
            "got: {}",
            err[0].message
        );
    }

    #[test]
    fn rejects_duplicate_parameter_names() {
        // `bind_args` documents unique parameter names as a precondition: with a
        // duplicate, a named argument binds the first of the pair and the
        // defaults loop then treats *both* as bound, so the second is never
        // type-checked and never reported. Rejecting it here is what makes that
        // precondition hold.
        let src = "fn f(x: int, x: bytes) -> int { x }\ntest t {\n pass\n}\n";
        let toks = lex(src).unwrap();
        let err = parse(src, &toks).unwrap_err();
        assert!(
            err[0].message.contains("duplicate parameter `x`"),
            "got: {}",
            err[0].message
        );
        // Anchored on the SECOND declaration — the one the author must delete.
        let second_x = src.match_indices('x').nth(1).unwrap().0;
        assert_eq!(
            err[0].primary,
            second_x..second_x + 1,
            "anchored on the second `x`"
        );
    }

    #[test]
    fn rejects_a_non_adjacent_duplicate_parameter() {
        // The scan must cover every parameter declared so far, not just the
        // previous one: a duplicate need not be adjacent to its twin.
        let src = "fn f(x: int, y: int, x: bytes) -> int { y }\ntest t {\n pass\n}\n";
        let toks = lex(src).unwrap();
        let err = parse(src, &toks).unwrap_err();
        assert!(
            err[0].message.contains("duplicate parameter `x`"),
            "got: {}",
            err[0].message
        );
    }

    #[test]
    fn a_duplicate_parameter_is_reported_before_its_type() {
        // The scan runs BEFORE the type is parsed, so the duplicate is what
        // gets blamed — not an unrelated later problem on the same parameter.
        let src = "fn f(x: int, x: word) -> int { x }\ntest t {\n pass\n}\n";
        let toks = lex(src).unwrap();
        let err = parse(src, &toks).unwrap_err();
        assert!(
            err[0].message.contains("duplicate parameter `x`"),
            "the duplicate must win over `unknown type`; got: {}",
            err[0].message
        );
    }

    #[test]
    fn parses_a_multiline_parameter_list_with_a_trailing_comma() {
        // Newlines inside the parens are not statement terminators, and a
        // trailing comma before `)` is allowed — the same rule as `[...]` and
        // call arguments.
        let m = parse_ok(
            "fn f(\n  op: byte,\n  addr: int,\n) -> bytes { [op, lo(addr)] }\ntest t {\n pass\n}\n",
        );
        assert_eq!(m.fns[0].params.len(), 2);
        assert_eq!(m.fns[0].params[1].name, "addr");
    }

    #[test]
    fn rejects_two_items_on_one_line() {
        // A `fn` is a statement-terminated item: the closing `}` must be
        // followed by a newline (or EOF), not by the next item.
        let src = "fn f(n: int) -> int { n } fn g(n: int) -> int { n }\ntest t {\n pass\n}\n";
        let toks = lex(src).unwrap();
        let err = parse(src, &toks).unwrap_err();
        assert!(
            err[0].message.contains("expected end of statement"),
            "got: {}",
            err[0].message
        );
    }

    #[test]
    fn parses_proc_item() {
        let m = parse_ok(
            "proc command(pkt: bytes, ndata: int, err: byte = 0x11) {\n send pkt + crc8\n tar 2\n}\ntest t {\n pass\n}\n",
        );
        assert_eq!(m.procs.len(), 1);
        let p = &m.procs[0];
        assert_eq!(p.name, "command");
        assert_eq!(p.params.len(), 3);
        assert!(p.params[2].default.is_some());
        assert_eq!(p.body.len(), 2);
    }

    #[test]
    fn parses_a_call_statement() {
        let m = parse_ok("test t {\n command(pkt = [0x44], 0)\n pass\n}\n");
        match &m.tests[0].stmts[0] {
            Stmt::Call { name, args, .. } => {
                assert_eq!(name, "command");
                assert_eq!(args.len(), 2);
                assert_eq!(args[0].name.as_deref(), Some("pkt"));
                assert!(args[1].name.is_none());
            }
            s => panic!("expected Call, got {s:?}"),
        }
    }

    #[test]
    fn rejects_two_call_statements_on_one_line() {
        // A `proc` call is statement-terminated like every other statement: the
        // closing `)` must be followed by a newline (or `}`), not the next call.
        let src = "test t {\n p() q()\n pass\n}\n";
        let toks = lex(src).unwrap();
        let err = parse(src, &toks).unwrap_err();
        assert!(
            err[0].message.contains("expected end of statement"),
            "got: {}",
            err[0].message
        );
    }

    #[test]
    fn stmt_keywords_shadow_a_call() {
        // The property the driver's name check depends on: for every word in
        // STMT_KEYWORDS, `word()` is NOT parsed as a `proc` call — the
        // statement grammar claims it first (usually by failing outright).
        // Drop an arm from `parse_stmt` without updating the list and this
        // fails, because the word would start parsing as a call.
        for kw in STMT_KEYWORDS {
            let src = format!("test t {{\n {kw}()\n pass\n}}\n");
            let toks = lex(&src).unwrap();
            match parse(&src, &toks) {
                // A parse error is the common outcome and is fine — the point
                // is only that it never becomes a callable reference.
                Err(_) => {}
                Ok(m) => assert!(
                    !matches!(m.tests[0].stmts[0], Stmt::Call { .. }),
                    "`{kw}` parsed as a call: it is no longer a statement keyword, \
                     so remove it from STMT_KEYWORDS"
                ),
            }
        }
    }

    fn parse_expr_ok(src: &str) -> Expr {
        let toks = lex(src).unwrap();
        let mut p = P {
            src,
            toks: &toks,
            i: 0,
        };
        p.parse_expr().unwrap()
    }

    #[test]
    fn parses_int_literal() {
        match parse_expr_ok("0x44") {
            Expr::Int { value, .. } => assert_eq!(value, 0x44),
            e => panic!("expected Int, got {e:?}"),
        }
    }

    #[test]
    fn parses_name() {
        match parse_expr_ok("PUT_IORD1") {
            Expr::Name { name, .. } => assert_eq!(name, "PUT_IORD1"),
            e => panic!("expected Name, got {e:?}"),
        }
    }

    #[test]
    fn parses_bytes_literal() {
        match parse_expr_ok("[0x44, 0x00, 0x64]") {
            Expr::Bytes { elems, .. } => assert_eq!(elems.len(), 3),
            e => panic!("expected Bytes, got {e:?}"),
        }
    }

    #[test]
    fn parses_call() {
        match parse_expr_ok("crc8(pkt)") {
            Expr::Call { func, .. } => assert_eq!(func, "crc8"),
            e => panic!("expected Call, got {e:?}"),
        }
    }

    #[test]
    fn parses_call_with_several_positional_args() {
        match parse_expr_ok("iord_hdr(0x44, 0x0064)") {
            Expr::Call { func, args, .. } => {
                assert_eq!(func, "iord_hdr");
                assert_eq!(args.len(), 2);
                assert!(args.iter().all(|a| a.name.is_none()));
            }
            e => panic!("expected Call, got {e:?}"),
        }
    }

    #[test]
    fn parses_named_arguments() {
        match parse_expr_ok("command(pkt = [0x44], ndata = 0)") {
            Expr::Call { args, .. } => {
                assert_eq!(args.len(), 2);
                assert_eq!(args[0].name.as_deref(), Some("pkt"));
                assert!(matches!(&args[0].value, Expr::Bytes { elems, .. } if elems.len() == 1));
                assert_eq!(args[1].name.as_deref(), Some("ndata"));
                assert!(matches!(args[1].value, Expr::Int { value: 0, .. }));
            }
            e => panic!("expected Call, got {e:?}"),
        }
    }

    #[test]
    fn parses_mixed_positional_and_named_arguments() {
        // The shape `bind_args` will consume: positional first, then named.
        match parse_expr_ok("command([0x44], ndata = 0)") {
            Expr::Call { args, .. } => {
                assert_eq!(args.len(), 2);
                assert!(args[0].name.is_none());
                assert!(matches!(&args[0].value, Expr::Bytes { elems, .. } if elems.len() == 1));
                assert_eq!(args[1].name.as_deref(), Some("ndata"));
                assert!(matches!(args[1].value, Expr::Int { value: 0, .. }));
            }
            e => panic!("expected Call, got {e:?}"),
        }
    }

    #[test]
    fn an_argument_span_covers_its_name_and_value() {
        // `builtin_arg` anchors its named-argument diagnostic on this span, and
        // Task 3's binder will too — so the extent is load-bearing.
        let src = "command(pkt = [0x44], 0x64)";
        match parse_expr_ok(src) {
            Expr::Call { args, .. } => {
                assert_eq!(&src[args[0].span.clone()], "pkt = [0x44]");
                assert_eq!(&src[args[1].span.clone()], "0x64");
            }
            e => panic!("expected Call, got {e:?}"),
        }
    }

    #[test]
    fn parses_multiline_call_with_trailing_comma() {
        match parse_expr_ok("command(\n  pkt = [0x44],\n  ndata = 0,\n)") {
            Expr::Call { args, .. } => assert_eq!(args.len(), 2),
            e => panic!("expected Call, got {e:?}"),
        }
    }

    #[test]
    fn parses_zero_argument_call() {
        match parse_expr_ok("controller()") {
            Expr::Call { func, args, .. } => {
                assert_eq!(func, "controller");
                assert!(args.is_empty());
            }
            e => panic!("expected Call, got {e:?}"),
        }
    }

    #[test]
    fn parses_parenthesized() {
        match parse_expr_ok("(0x05)") {
            Expr::Int { value, .. } => assert_eq!(value, 5),
            e => panic!("expected Int, got {e:?}"),
        }
    }

    #[test]
    fn rejects_bad_number() {
        let toks = lex("0xZZ").unwrap();
        let mut p = P {
            src: "0xZZ",
            toks: &toks,
            i: 0,
        };
        assert!(p.parse_expr().is_err());
    }

    #[test]
    fn parses_concat() {
        match parse_expr_ok("a ++ b") {
            Expr::Binary {
                op: BinOp::Concat, ..
            } => {}
            e => panic!("expected Concat, got {e:?}"),
        }
    }

    #[test]
    fn parses_xor() {
        match parse_expr_ok("0x16 ^ 0xFF") {
            Expr::Binary {
                op: BinOp::Xor,
                lhs,
                rhs,
                ..
            } => {
                assert!(matches!(*lhs, Expr::Int { value: 0x16, .. }));
                assert!(matches!(*rhs, Expr::Int { value: 0xFF, .. }));
            }
            e => panic!("expected Xor, got {e:?}"),
        }
    }

    #[test]
    fn concat_is_lower_precedence_than_xor() {
        // `a ++ b ^ c` parses as `a ++ (b ^ c)`
        match parse_expr_ok("a ++ b ^ c") {
            Expr::Binary {
                op: BinOp::Concat,
                rhs,
                ..
            } => assert!(matches!(*rhs, Expr::Binary { op: BinOp::Xor, .. })),
            e => panic!("expected top-level Concat, got {e:?}"),
        }
    }

    #[test]
    fn parses_send_with_crc() {
        let m = parse_ok("test t {\n  send [0x44] + crc8\n  pass\n}\n");
        match &m.tests[0].stmts[0] {
            Stmt::Send { append_crc, .. } => assert!(append_crc),
            s => panic!("expected Send, got {s:?}"),
        }
    }

    #[test]
    fn parses_crc_region() {
        let m =
            parse_ok("test t {\n crc_region {\n  send [0x44]\n  send [0x00, 0x64]\n }\n pass\n}\n");
        match &m.tests[0].stmts[0] {
            Stmt::CrcRegion { sends, .. } => assert_eq!(sends.len(), 2),
            s => panic!("expected CrcRegion, got {s:?}"),
        }
    }

    #[test]
    fn parses_config() {
        let m = parse_ok("test t {\n config controller, x1, sck20, alert_pin\n pass\n}\n");
        match &m.tests[0].stmts[0] {
            Stmt::Config {
                role,
                io,
                sck,
                alert,
                ..
            } => {
                assert_eq!(role, "controller");
                assert_eq!(io, "x1");
                assert_eq!(sck, "sck20");
                assert_eq!(alert, "alert_pin");
            }
            s => panic!("expected Config, got {s:?}"),
        }
    }

    #[test]
    fn parses_multiline_bytes_literal() {
        // Newlines inside `[...]` are not statement terminators; a trailing
        // comma is allowed. (The motivating snippet from the Plan-3 prep.)
        match parse_expr_ok("[\n  0x44,\n  0x00,\n  0x64,\n]") {
            Expr::Bytes { elems, .. } => assert_eq!(elems.len(), 3),
            e => panic!("expected Bytes, got {e:?}"),
        }
    }

    #[test]
    fn parses_bytes_with_trailing_comma() {
        // A trailing comma before the closing `]` is allowed even on one line.
        match parse_expr_ok("[0x44, 0x00,]") {
            Expr::Bytes { elems, .. } => assert_eq!(elems.len(), 2),
            e => panic!("expected Bytes, got {e:?}"),
        }
    }

    #[test]
    fn parses_multiline_call_arg() {
        // Newlines after `(` and before `)` are not statement terminators.
        match parse_expr_ok("crc8(\n  pkt\n)") {
            Expr::Call { func, .. } => assert_eq!(func, "crc8"),
            e => panic!("expected Call, got {e:?}"),
        }
    }

    #[test]
    fn parses_multiline_parenthesized() {
        match parse_expr_ok("(\n  0x05\n)") {
            Expr::Int { value, .. } => assert_eq!(value, 5),
            e => panic!("expected Int, got {e:?}"),
        }
    }

    #[test]
    fn parses_multiline_send_statement() {
        // A `send` whose bytes literal spans multiple lines (with a trailing
        // comma) still parses, and `+ crc8` after the `]` is picked up.
        let m = parse_ok(
            "test t {\n  send [\n    0x44,\n    0x00,\n    0x64,\n  ] + crc8\n  pass\n}\n",
        );
        match &m.tests[0].stmts[0] {
            Stmt::Send {
                bytes, append_crc, ..
            } => {
                assert!(append_crc);
                match bytes {
                    Expr::Bytes { elems, .. } => assert_eq!(elems.len(), 3),
                    e => panic!("expected Bytes, got {e:?}"),
                }
            }
            s => panic!("expected Send, got {s:?}"),
        }
    }

    #[test]
    fn parses_frame_with_body() {
        let m = parse_ok("test t {\n frame {\n  send [0x44]\n  tar 2\n }\n pass\n}\n");
        match &m.tests[0].stmts[0] {
            Stmt::Frame { body, .. } => assert_eq!(body.len(), 2),
            s => panic!("expected Frame, got {s:?}"),
        }
    }

    #[test]
    fn parses_recv_names_and_discard() {
        let m = parse_ok("test t {\n recv data, _, status\n pass\n}\n");
        match &m.tests[0].stmts[0] {
            Stmt::Recv { targets, .. } => {
                assert!(matches!(targets[0], RecvTarget::Name(ref n) if n == "data"));
                assert!(matches!(targets[1], RecvTarget::Discard));
                assert!(matches!(targets[2], RecvTarget::Name(ref n) if n == "status"));
            }
            s => panic!("expected Recv, got {s:?}"),
        }
    }

    #[test]
    fn parses_recv_count() {
        let m = parse_ok("test t {\n recv 3\n pass\n}\n");
        match &m.tests[0].stmts[0] {
            Stmt::Recv { targets, .. } => {
                assert_eq!(targets.len(), 3);
                assert!(targets.iter().all(|t| matches!(t, RecvTarget::Discard)));
            }
            s => panic!("expected Recv, got {s:?}"),
        }
    }

    #[test]
    fn parses_wait_state_bare_and_named() {
        let m = parse_ok("test t {\n wait_state\n wait_state term\n pass\n}\n");
        assert!(matches!(
            m.tests[0].stmts[0],
            Stmt::WaitState { bind: None, .. }
        ));
        match &m.tests[0].stmts[1] {
            Stmt::WaitState { bind: Some(n), .. } => assert_eq!(n, "term"),
            s => panic!("expected named WaitState, got {s:?}"),
        }
    }

    #[test]
    fn parses_expect_crc_else() {
        let m = parse_ok("test t {\n frame {\n  expect crc else 0x11\n }\n pass\n}\n");
        match &m.tests[0].stmts[0] {
            Stmt::Frame { body, .. } => {
                assert!(matches!(body[0], Stmt::Expect { .. }));
            }
            s => panic!("expected Frame, got {s:?}"),
        }
    }

    #[test]
    fn parses_repeat() {
        let m = parse_ok("test t {\n repeat 3 {\n  recv _\n }\n pass\n}\n");
        match &m.tests[0].stmts[0] {
            Stmt::Repeat { body, .. } => assert_eq!(body.len(), 1),
            s => panic!("expected Repeat, got {s:?}"),
        }
    }
}
