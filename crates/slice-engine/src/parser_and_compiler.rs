//! Recursive-Descent ECMAScript/TypeScript Lexer, Parser, `SLICESNP`/`SLICEBOA` Compiler, and Disassembler.

use crate::boa_realm::SNP_MODULE_CACHE;
use crate::value_and_ast::{
    BinOp, BytecodeModule, BytecodeOp, Expr, JsFunction, JsValue, Stmt, UnaryOp, SNAPSHOT_MAGIC,
};
use anyhow::{bail, Result};
use rustc_hash::FxHashMap;
use slice_cas::strip_typescript_and_normalize;

// ============================================================================
// Lexer & Recursive-Descent JS/TS Parser
// ============================================================================

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Token {
    Num(f64),
    Str(String),
    Ident(String),
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Semi,
    Comma,
    Dot,
    Colon,
    Question,
    Arrow, // =>
    Plus,
    Minus,
    Star,
    StarStar, // **
    Slash,
    Percent,
    PlusPlus,
    MinusMinus,
    PlusEq,
    MinusEq,
    StarEq,
    SlashEq,
    Eq,
    EqEq,
    EqEqEq,
    Bang,
    BangEq,
    BangEqEq,
    Lt,
    Lte,
    Gt,
    Gte,
    AmpAmp,
    PipePipe,
    Amp,
    Pipe,
    Caret,
    Shl,
    Shr,
    Eof,
}

pub(crate) fn tokenize_js(input: &str) -> Vec<Token> {
    let bytes = input.as_bytes();
    let mut tokens = Vec::new();
    let mut i = 0;

    while i < bytes.len() {
        let b = bytes[i];
        if b.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        // Line comment
        if b == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        // Block comment
        if b == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i += 2;
            continue;
        }
        // String literal
        if b == b'"' || b == b'\'' || b == b'`' {
            let quote = b;
            i += 1;
            let mut s = String::new();
            while i < bytes.len() && bytes[i] != quote {
                if bytes[i] == b'\\' && i + 1 < bytes.len() {
                    i += 1;
                    match bytes[i] {
                        b'n' => s.push('\n'),
                        b'r' => s.push('\r'),
                        b't' => s.push('\t'),
                        other => s.push(other as char),
                    }
                } else {
                    s.push(bytes[i] as char);
                }
                i += 1;
            }
            i += 1;
            tokens.push(Token::Str(s));
            continue;
        }
        // Number literal
        if b.is_ascii_digit() {
            let start = i;
            if b == b'0' && i + 1 < bytes.len() && (bytes[i + 1] == b'x' || bytes[i + 1] == b'X') {
                i += 2;
                let hex_start = i;
                while i < bytes.len() && bytes[i].is_ascii_hexdigit() {
                    i += 1;
                }
                let hex_str = &input[hex_start..i];
                let val = u64::from_str_radix(hex_str, 16).unwrap_or(0) as f64;
                tokens.push(Token::Num(val));
                continue;
            }
            while i < bytes.len()
                && (bytes[i].is_ascii_digit() || bytes[i] == b'.' || bytes[i] == b'_')
            {
                i += 1;
            }
            if i < bytes.len() && (bytes[i] == b'e' || bytes[i] == b'E') {
                i += 1;
                if i < bytes.len() && (bytes[i] == b'+' || bytes[i] == b'-') {
                    i += 1;
                }
                while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'_') {
                    i += 1;
                }
            }
            let num_str: String = input[start..i].chars().filter(|&c| c != '_').collect();
            let val = num_str.parse::<f64>().unwrap_or(0.0);
            tokens.push(Token::Num(val));
            continue;
        }
        // Identifier or keyword
        if b.is_ascii_alphabetic() || b == b'_' || b == b'$' {
            let start = i;
            while i < bytes.len()
                && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'$')
            {
                i += 1;
            }
            tokens.push(Token::Ident(input[start..i].to_string()));
            continue;
        }

        // Multi-char operators
        let rem = &bytes[i..];
        if rem.starts_with(b"===") {
            tokens.push(Token::EqEqEq);
            i += 3;
        } else if rem.starts_with(b"!==") {
            tokens.push(Token::BangEqEq);
            i += 3;
        } else if rem.starts_with(b"=>") {
            tokens.push(Token::Arrow);
            i += 2;
        } else if rem.starts_with(b"==") {
            tokens.push(Token::EqEq);
            i += 2;
        } else if rem.starts_with(b"!=") {
            tokens.push(Token::BangEq);
            i += 2;
        } else if rem.starts_with(b"<=") {
            tokens.push(Token::Lte);
            i += 2;
        } else if rem.starts_with(b">=") {
            tokens.push(Token::Gte);
            i += 2;
        } else if rem.starts_with(b"<<") {
            tokens.push(Token::Shl);
            i += 2;
        } else if rem.starts_with(b">>") {
            tokens.push(Token::Shr);
            i += 2;
        } else if rem.starts_with(b"&&") {
            tokens.push(Token::AmpAmp);
            i += 2;
        } else if rem.starts_with(b"||") {
            tokens.push(Token::PipePipe);
            i += 2;
        } else if rem.starts_with(b"++") {
            tokens.push(Token::PlusPlus);
            i += 2;
        } else if rem.starts_with(b"--") {
            tokens.push(Token::MinusMinus);
            i += 2;
        } else if rem.starts_with(b"+=") {
            tokens.push(Token::PlusEq);
            i += 2;
        } else if rem.starts_with(b"-=") {
            tokens.push(Token::MinusEq);
            i += 2;
        } else if rem.starts_with(b"*=") {
            tokens.push(Token::StarEq);
            i += 2;
        } else if rem.starts_with(b"/=") {
            tokens.push(Token::SlashEq);
            i += 2;
        } else if rem.starts_with(b"**") {
            tokens.push(Token::StarStar);
            i += 2;
        } else {
            match b {
                b'(' => tokens.push(Token::LParen),
                b')' => tokens.push(Token::RParen),
                b'{' => tokens.push(Token::LBrace),
                b'}' => tokens.push(Token::RBrace),
                b'[' => tokens.push(Token::LBracket),
                b']' => tokens.push(Token::RBracket),
                b';' => tokens.push(Token::Semi),
                b',' => tokens.push(Token::Comma),
                b'.' => tokens.push(Token::Dot),
                b':' => tokens.push(Token::Colon),
                b'?' => tokens.push(Token::Question),
                b'+' => tokens.push(Token::Plus),
                b'-' => tokens.push(Token::Minus),
                b'*' => tokens.push(Token::Star),
                b'/' => tokens.push(Token::Slash),
                b'%' => tokens.push(Token::Percent),
                b'=' => tokens.push(Token::Eq),
                b'!' => tokens.push(Token::Bang),
                b'<' => tokens.push(Token::Lt),
                b'>' => tokens.push(Token::Gt),
                b'&' => tokens.push(Token::Amp),
                b'|' => tokens.push(Token::Pipe),
                b'^' => tokens.push(Token::Caret),
                _ => {}
            }
            i += 1;
        }
    }

    tokens.push(Token::Eof);
    tokens
}

pub(crate) struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    pub(crate) fn new(tokens: Vec<Token>) -> Self {
        Self { tokens, pos: 0 }
    }

    fn peek(&self) -> &Token {
        self.tokens.get(self.pos).unwrap_or(&Token::Eof)
    }

    fn next(&mut self) -> Token {
        let tok = self.peek().clone();
        if self.pos < self.tokens.len() {
            self.pos += 1;
        }
        tok
    }

    fn eat_semi(&mut self) {
        while matches!(self.peek(), Token::Semi) {
            self.next();
        }
    }

    /// Skip TypeScript `: Type` annotation if present after an identifier or `)`
    fn skip_optional_ts_type(&mut self) {
        if matches!(self.peek(), Token::Colon) {
            self.next();
            let mut angle_depth = 0;
            while !matches!(self.peek(), Token::Eof) {
                match self.peek() {
                    Token::Lt => {
                        angle_depth += 1;
                        self.next();
                    }
                    Token::Gt if angle_depth > 0 => {
                        angle_depth -= 1;
                        self.next();
                    }
                    Token::LBracket => {
                        self.next();
                        if matches!(self.peek(), Token::RBracket) {
                            self.next();
                        }
                    }
                    Token::Ident(_) | Token::Pipe => {
                        self.next();
                    }
                    _ if angle_depth > 0 => {
                        self.next();
                    }
                    _ => break,
                }
            }
        }
    }

    pub(crate) fn parse_program(&mut self) -> Vec<Stmt> {
        let mut stmts = Vec::new();
        while !matches!(self.peek(), Token::Eof) {
            self.eat_semi();
            if matches!(self.peek(), Token::Eof) {
                break;
            }
            if let Some(stmt) = self.parse_stmt() {
                stmts.push(stmt);
            } else {
                self.next();
            }
            self.eat_semi();
        }
        stmts
    }

    fn parse_stmt(&mut self) -> Option<Stmt> {
        match self.peek().clone() {
            Token::Ident(kw) if kw == "interface" || kw == "type" => {
                self.skip_type_declaration();
                None
            }
            Token::Ident(kw) if kw == "export" => {
                self.next();
                if let Token::Ident(next_kw) = self.peek().clone() {
                    if next_kw == "interface" || next_kw == "type" {
                        self.skip_type_declaration();
                        return None;
                    }
                    if next_kw == "const" || next_kw == "let" || next_kw == "var" {
                        self.next();
                        return self.parse_var_decl(true);
                    }
                    if next_kw == "function" {
                        self.next();
                        return self.parse_func_decl();
                    }
                }
                self.parse_stmt()
            }
            Token::Ident(kw) if kw == "import" => self.parse_import(),
            Token::Ident(kw) if kw == "var" || kw == "let" || kw == "const" => {
                self.next();
                self.parse_var_decl(false)
            }
            Token::Ident(kw) if kw == "function" => {
                self.next();
                self.parse_func_decl()
            }
            Token::Ident(kw) if kw == "if" => {
                self.next();
                self.parse_if()
            }
            Token::Ident(kw) if kw == "while" => {
                self.next();
                self.parse_while()
            }
            Token::Ident(kw) if kw == "for" => {
                self.next();
                self.parse_for()
            }
            Token::Ident(kw) if kw == "return" => {
                self.next();
                if matches!(self.peek(), Token::Semi | Token::RBrace | Token::Eof) {
                    Some(Stmt::Return(None))
                } else {
                    Some(Stmt::Return(Some(self.parse_expr())))
                }
            }
            Token::Ident(kw) if kw == "break" => {
                self.next();
                Some(Stmt::Break)
            }
            Token::Ident(kw) if kw == "continue" => {
                self.next();
                Some(Stmt::Continue)
            }
            Token::LBrace => Some(Stmt::Block(self.parse_block())),
            _ => Some(Stmt::ExprStmt(self.parse_expr())),
        }
    }

    fn skip_type_declaration(&mut self) {
        let kw = self.next();
        if kw == Token::Ident("interface".to_string()) {
            while !matches!(self.peek(), Token::LBrace | Token::Eof) {
                self.next();
            }
            if matches!(self.peek(), Token::LBrace) {
                self.next();
                let mut depth = 1;
                while depth > 0 && !matches!(self.peek(), Token::Eof) {
                    match self.next() {
                        Token::LBrace => depth += 1,
                        Token::RBrace => depth -= 1,
                        _ => {}
                    }
                }
            }
        } else {
            while !matches!(self.peek(), Token::Semi | Token::Eof) {
                self.next();
            }
        }
    }

    fn parse_import(&mut self) -> Option<Stmt> {
        self.next(); // consume 'import'
        let mut bindings = Vec::new();
        if matches!(self.peek(), Token::LBrace) {
            self.next();
            while !matches!(self.peek(), Token::RBrace | Token::Eof) {
                if let Token::Ident(name) = self.next() {
                    bindings.push(name);
                }
                if matches!(self.peek(), Token::Comma) {
                    self.next();
                }
            }
            if matches!(self.peek(), Token::RBrace) {
                self.next();
            }
        } else if let Token::Ident(name) = self.peek().clone() {
            if name != "from" {
                self.next();
                bindings.push(name);
            }
        }
        if matches!(self.peek(), Token::Ident(s) if s == "from") {
            self.next();
        }
        if let Token::Str(spec) = self.next() {
            return Some(Stmt::ImportCas {
                bindings,
                specifier: spec,
            });
        }
        None
    }

    fn parse_var_decl(&mut self, is_export: bool) -> Option<Stmt> {
        let name = match self.next() {
            Token::Ident(s) => s,
            _ => return None,
        };
        self.skip_optional_ts_type();
        let init = if matches!(self.peek(), Token::Eq) {
            self.next();
            Some(self.parse_expr())
        } else {
            None
        };
        Some(Stmt::VarDecl {
            name,
            init,
            is_export,
        })
    }

    fn parse_func_decl(&mut self) -> Option<Stmt> {
        let name = match self.next() {
            Token::Ident(s) => s,
            _ => String::new(),
        };
        if matches!(self.peek(), Token::LParen) {
            self.next();
        }
        let mut params = Vec::new();
        while !matches!(self.peek(), Token::RParen | Token::Eof) {
            if let Token::Ident(p) = self.next() {
                params.push(p);
                self.skip_optional_ts_type();
            }
            if matches!(self.peek(), Token::Comma) {
                self.next();
            }
        }
        if matches!(self.peek(), Token::RParen) {
            self.next();
        }
        self.skip_optional_ts_type();
        let body = self.parse_block();
        Some(Stmt::FuncDecl { name, params, body })
    }

    fn parse_block(&mut self) -> Vec<Stmt> {
        if !matches!(self.peek(), Token::LBrace) {
            return self.parse_stmt().into_iter().collect();
        }
        self.next(); // consume '{'
        let mut stmts = Vec::new();
        while !matches!(self.peek(), Token::RBrace | Token::Eof) {
            self.eat_semi();
            if matches!(self.peek(), Token::RBrace | Token::Eof) {
                break;
            }
            if let Some(s) = self.parse_stmt() {
                stmts.push(s);
            } else {
                self.next();
            }
            self.eat_semi();
        }
        if matches!(self.peek(), Token::RBrace) {
            self.next();
        }
        stmts
    }

    fn parse_if(&mut self) -> Option<Stmt> {
        if matches!(self.peek(), Token::LParen) {
            self.next();
        }
        let cond = self.parse_expr();
        if matches!(self.peek(), Token::RParen) {
            self.next();
        }
        let then_branch = self.parse_block();
        let else_branch = if matches!(self.peek(), Token::Ident(s) if s == "else") {
            self.next();
            self.parse_block()
        } else {
            Vec::new()
        };
        Some(Stmt::If {
            cond,
            then_branch,
            else_branch,
        })
    }

    fn parse_while(&mut self) -> Option<Stmt> {
        if matches!(self.peek(), Token::LParen) {
            self.next();
        }
        let cond = self.parse_expr();
        if matches!(self.peek(), Token::RParen) {
            self.next();
        }
        let body = self.parse_block();
        Some(Stmt::While { cond, body })
    }

    fn parse_for(&mut self) -> Option<Stmt> {
        if matches!(self.peek(), Token::LParen) {
            self.next();
        }
        let init = if matches!(self.peek(), Token::Semi) {
            self.next();
            None
        } else {
            let s = self.parse_stmt().map(Box::new);
            if matches!(self.peek(), Token::Semi) {
                self.next();
            }
            s
        };

        let cond = if matches!(self.peek(), Token::Semi) {
            self.next();
            None
        } else {
            let e = self.parse_expr();
            if matches!(self.peek(), Token::Semi) {
                self.next();
            }
            Some(e)
        };

        let step = if matches!(self.peek(), Token::RParen) {
            self.next();
            None
        } else {
            let e = self.parse_expr();
            if matches!(self.peek(), Token::RParen) {
                self.next();
            }
            Some(e)
        };

        let body = self.parse_block();
        Some(Stmt::For {
            init,
            cond,
            step,
            body,
        })
    }

    fn parse_expr(&mut self) -> Expr {
        self.parse_assign()
    }

    fn parse_assign(&mut self) -> Expr {
        let lhs = self.parse_ternary();
        match self.peek().clone() {
            Token::Eq | Token::PlusEq | Token::MinusEq | Token::StarEq | Token::SlashEq => {
                let op_tok = self.next();
                let assign_op = match op_tok {
                    Token::PlusEq => Some(BinOp::Add),
                    Token::MinusEq => Some(BinOp::Sub),
                    Token::StarEq => Some(BinOp::Mul),
                    Token::SlashEq => Some(BinOp::Div),
                    _ => None,
                };
                let rhs = self.parse_assign();
                match lhs {
                    Expr::Ident(name) => Expr::Assign {
                        name,
                        op: assign_op,
                        value: Box::new(rhs),
                    },
                    Expr::Member { object, property } => {
                        let chain = expr_to_dotted_chain(&object);
                        Expr::PropAssign {
                            target_chain: chain,
                            prop: property,
                            value: Box::new(rhs),
                        }
                    }
                    Expr::Index { object, index } => {
                        if let Expr::Ident(target) = *object {
                            Expr::IndexAssign {
                                target,
                                index,
                                value: Box::new(rhs),
                            }
                        } else {
                            rhs
                        }
                    }
                    _ => rhs,
                }
            }
            _ => lhs,
        }
    }

    fn parse_ternary(&mut self) -> Expr {
        let cond = self.parse_logical_or();
        if matches!(self.peek(), Token::Question) {
            self.next();
            let then_expr = self.parse_expr();
            if matches!(self.peek(), Token::Colon) {
                self.next();
            }
            let else_expr = self.parse_ternary();
            Expr::Ternary {
                cond: Box::new(cond),
                then_expr: Box::new(then_expr),
                else_expr: Box::new(else_expr),
            }
        } else {
            cond
        }
    }

    fn parse_logical_or(&mut self) -> Expr {
        let mut left = self.parse_logical_and();
        while matches!(self.peek(), Token::PipePipe) {
            self.next();
            let right = self.parse_logical_and();
            left = Expr::Binary {
                op: BinOp::Or,
                lhs: Box::new(left),
                rhs: Box::new(right),
            };
        }
        left
    }

    fn parse_logical_and(&mut self) -> Expr {
        let mut left = self.parse_bitwise();
        while matches!(self.peek(), Token::AmpAmp) {
            self.next();
            let right = self.parse_bitwise();
            left = Expr::Binary {
                op: BinOp::And,
                lhs: Box::new(left),
                rhs: Box::new(right),
            };
        }
        left
    }

    fn parse_bitwise(&mut self) -> Expr {
        let mut left = self.parse_equality();
        loop {
            let op = match self.peek() {
                Token::Amp => BinOp::BitAnd,
                Token::Pipe => BinOp::BitOr,
                Token::Caret => BinOp::BitXor,
                _ => break,
            };
            self.next();
            let right = self.parse_equality();
            left = Expr::Binary {
                op,
                lhs: Box::new(left),
                rhs: Box::new(right),
            };
        }
        left
    }

    fn parse_equality(&mut self) -> Expr {
        let mut left = self.parse_relational();
        loop {
            let op = match self.peek() {
                Token::EqEq => BinOp::Eq,
                Token::BangEq => BinOp::Neq,
                Token::EqEqEq => BinOp::StrictEq,
                Token::BangEqEq => BinOp::StrictNeq,
                _ => break,
            };
            self.next();
            let right = self.parse_relational();
            left = Expr::Binary {
                op,
                lhs: Box::new(left),
                rhs: Box::new(right),
            };
        }
        left
    }

    fn parse_relational(&mut self) -> Expr {
        let mut left = self.parse_shift();
        loop {
            let op = match self.peek() {
                Token::Lt => BinOp::Lt,
                Token::Lte => BinOp::Lte,
                Token::Gt => BinOp::Gt,
                Token::Gte => BinOp::Gte,
                _ => break,
            };
            self.next();
            let right = self.parse_shift();
            left = Expr::Binary {
                op,
                lhs: Box::new(left),
                rhs: Box::new(right),
            };
        }
        left
    }

    fn parse_shift(&mut self) -> Expr {
        let mut left = self.parse_additive();
        loop {
            let op = match self.peek() {
                Token::Shl => BinOp::Shl,
                Token::Shr => BinOp::Shr,
                _ => break,
            };
            self.next();
            let right = self.parse_additive();
            left = Expr::Binary {
                op,
                lhs: Box::new(left),
                rhs: Box::new(right),
            };
        }
        left
    }

    fn parse_additive(&mut self) -> Expr {
        let mut left = self.parse_multiplicative();
        loop {
            let op = match self.peek() {
                Token::Plus => BinOp::Add,
                Token::Minus => BinOp::Sub,
                _ => break,
            };
            self.next();
            let right = self.parse_multiplicative();
            left = Expr::Binary {
                op,
                lhs: Box::new(left),
                rhs: Box::new(right),
            };
        }
        left
    }

    fn parse_multiplicative(&mut self) -> Expr {
        let mut left = self.parse_exponent();
        loop {
            let op = match self.peek() {
                Token::Star => BinOp::Mul,
                Token::Slash => BinOp::Div,
                Token::Percent => BinOp::Rem,
                _ => break,
            };
            self.next();
            let right = self.parse_exponent();
            left = Expr::Binary {
                op,
                lhs: Box::new(left),
                rhs: Box::new(right),
            };
        }
        left
    }

    fn parse_exponent(&mut self) -> Expr {
        let left = self.parse_unary();
        if matches!(self.peek(), Token::StarStar) {
            self.next();
            let right = self.parse_exponent();
            Expr::Binary {
                op: BinOp::Pow,
                lhs: Box::new(left),
                rhs: Box::new(right),
            }
        } else {
            left
        }
    }

    fn parse_unary(&mut self) -> Expr {
        match self.peek().clone() {
            Token::Minus => {
                self.next();
                Expr::Unary {
                    op: UnaryOp::Neg,
                    expr: Box::new(self.parse_unary()),
                }
            }
            Token::Plus => {
                self.next();
                Expr::Unary {
                    op: UnaryOp::Pos,
                    expr: Box::new(self.parse_unary()),
                }
            }
            Token::Bang => {
                self.next();
                Expr::Unary {
                    op: UnaryOp::Not,
                    expr: Box::new(self.parse_unary()),
                }
            }
            Token::Ident(kw) if kw == "typeof" => {
                self.next();
                Expr::Unary {
                    op: UnaryOp::TypeOf,
                    expr: Box::new(self.parse_unary()),
                }
            }
            Token::Ident(kw) if kw == "new" => {
                self.next();
                self.parse_postfix()
            }
            Token::PlusPlus => {
                self.next();
                if let Expr::Ident(name) = self.parse_postfix() {
                    Expr::Update {
                        name,
                        increment: true,
                    }
                } else {
                    Expr::Literal(JsValue::Undefined)
                }
            }
            Token::MinusMinus => {
                self.next();
                if let Expr::Ident(name) = self.parse_postfix() {
                    Expr::Update {
                        name,
                        increment: false,
                    }
                } else {
                    Expr::Literal(JsValue::Undefined)
                }
            }
            _ => self.parse_postfix(),
        }
    }

    fn parse_postfix(&mut self) -> Expr {
        let mut expr = self.parse_primary();
        loop {
            match self.peek() {
                Token::LParen => {
                    self.next();
                    let mut args = Vec::new();
                    while !matches!(self.peek(), Token::RParen | Token::Eof) {
                        args.push(self.parse_expr());
                        if matches!(self.peek(), Token::Comma) {
                            self.next();
                        }
                    }
                    if matches!(self.peek(), Token::RParen) {
                        self.next();
                    }
                    expr = Expr::Call {
                        callee: Box::new(expr),
                        args,
                    };
                }
                Token::Dot => {
                    self.next();
                    if let Token::Ident(prop) = self.next() {
                        expr = Expr::Member {
                            object: Box::new(expr),
                            property: prop,
                        };
                    }
                }
                Token::LBracket => {
                    self.next();
                    let idx = self.parse_expr();
                    if matches!(self.peek(), Token::RBracket) {
                        self.next();
                    }
                    expr = Expr::Index {
                        object: Box::new(expr),
                        index: Box::new(idx),
                    };
                }
                Token::PlusPlus => {
                    self.next();
                    if let Expr::Ident(name) = expr {
                        expr = Expr::Update {
                            name,
                            increment: true,
                        };
                    }
                }
                Token::MinusMinus => {
                    self.next();
                    if let Expr::Ident(name) = expr {
                        expr = Expr::Update {
                            name,
                            increment: false,
                        };
                    }
                }
                _ => break,
            }
        }
        expr
    }

    fn parse_primary(&mut self) -> Expr {
        match self.next() {
            Token::Num(n) => Expr::Literal(JsValue::Number(n)),
            Token::Str(s) => Expr::Literal(JsValue::str(s)),
            Token::Ident(s) => match s.as_str() {
                "true" => Expr::Literal(JsValue::Bool(true)),
                "false" => Expr::Literal(JsValue::Bool(false)),
                "null" => Expr::Literal(JsValue::Null),
                "undefined" => Expr::Literal(JsValue::Undefined),
                "NaN" => Expr::Literal(JsValue::Number(f64::NAN)),
                "Infinity" => Expr::Literal(JsValue::Number(f64::INFINITY)),
                "function" => {
                    // Anonymous or named function expression
                    let mut fn_name = String::new();
                    if let Token::Ident(id) = self.peek().clone() {
                        self.next();
                        fn_name = id;
                    }
                    if matches!(self.peek(), Token::LParen) {
                        self.next();
                    }
                    let mut params = Vec::new();
                    while !matches!(self.peek(), Token::RParen | Token::Eof) {
                        if let Token::Ident(p) = self.next() {
                            params.push(p);
                            self.skip_optional_ts_type();
                        }
                        if matches!(self.peek(), Token::Comma) {
                            self.next();
                        }
                    }
                    if matches!(self.peek(), Token::RParen) {
                        self.next();
                    }
                    self.skip_optional_ts_type();
                    let body = self.parse_block();
                    let mut js_fn = JsFunction {
                        name: fn_name,
                        params,
                        body,
                        compiled_numeric: None,
                    };
                    js_fn.compiled_numeric = crate::numeric_vm::try_compile_numeric_func(&js_fn);
                    Expr::Literal(JsValue::Function(Box::new(js_fn)))
                }
                _ => {
                    // Check single-param arrow function `x => expr`
                    if matches!(self.peek(), Token::Arrow) {
                        self.next();
                        let body = if matches!(self.peek(), Token::LBrace) {
                            self.parse_block()
                        } else {
                            vec![Stmt::Return(Some(self.parse_expr()))]
                        };
                        Expr::ArrowFunc {
                            params: vec![s],
                            body,
                        }
                    } else {
                        Expr::Ident(s)
                    }
                }
            },
            Token::LParen => {
                // Check if `(a, b) => ...` arrow function vs `(expr)`
                let saved_pos = self.pos;
                let mut params = Vec::new();
                let mut is_arrow_params = true;
                while !matches!(self.peek(), Token::RParen | Token::Eof) {
                    if let Token::Ident(p) = self.next() {
                        params.push(p);
                        self.skip_optional_ts_type();
                        if matches!(self.peek(), Token::Comma) {
                            self.next();
                        } else if !matches!(self.peek(), Token::RParen) {
                            is_arrow_params = false;
                            break;
                        }
                    } else {
                        is_arrow_params = false;
                        break;
                    }
                }
                if is_arrow_params && matches!(self.peek(), Token::RParen) {
                    self.next();
                    self.skip_optional_ts_type();
                    if matches!(self.peek(), Token::Arrow) {
                        self.next();
                        let body = if matches!(self.peek(), Token::LBrace) {
                            self.parse_block()
                        } else {
                            vec![Stmt::Return(Some(self.parse_expr()))]
                        };
                        return Expr::ArrowFunc { params, body };
                    }
                }
                // Rewind and parse as parenthesized expression
                self.pos = saved_pos;
                let e = self.parse_expr();
                if matches!(self.peek(), Token::RParen) {
                    self.next();
                }
                e
            }
            Token::LBracket => {
                let mut items = Vec::new();
                while !matches!(self.peek(), Token::RBracket | Token::Eof) {
                    items.push(self.parse_expr());
                    if matches!(self.peek(), Token::Comma) {
                        self.next();
                    }
                }
                if matches!(self.peek(), Token::RBracket) {
                    self.next();
                }
                Expr::ArrayLit(items)
            }
            Token::LBrace => {
                let mut props = Vec::new();
                while !matches!(self.peek(), Token::RBrace | Token::Eof) {
                    let key = match self.next() {
                        Token::Ident(k) | Token::Str(k) => k,
                        Token::Num(n) => n.to_string(),
                        _ => break,
                    };
                    if matches!(self.peek(), Token::Colon) {
                        self.next();
                        let val = self.parse_expr();
                        props.push((key, val));
                    } else {
                        // Shorthand `{ x }`
                        props.push((key.clone(), Expr::Ident(key)));
                    }
                    if matches!(self.peek(), Token::Comma) {
                        self.next();
                    }
                }
                if matches!(self.peek(), Token::RBrace) {
                    self.next();
                }
                Expr::ObjectLit(props)
            }
            _ => Expr::Literal(JsValue::Undefined),
        }
    }
}

pub(crate) fn expr_to_dotted_chain(expr: &Expr) -> String {
    match expr {
        Expr::Ident(s) => s.clone(),
        Expr::Member { object, property } => {
            format!("{}.{}", expr_to_dotted_chain(object), property)
        }
        _ => String::new(),
    }
}

// ============================================================================
// Compiler: JS/TS Source -> SLICESNP Bytecode Snapshot
// ============================================================================

fn update_backtick_state_on_line(line: &str, mut in_backtick: bool) -> bool {
    let bytes = line.as_bytes();
    let len = bytes.len();
    let mut i = 0usize;
    while i < len {
        let b = bytes[i];
        if b == b'\\' {
            i += 2;
            continue;
        }
        if in_backtick {
            if b == b'`' {
                in_backtick = false;
            }
            i += 1;
            continue;
        }
        if b == b'\'' || b == b'"' {
            let q = b;
            let mut j = i + 1;
            let mut found_close = None;
            while j < len {
                if bytes[j] == b'\\' {
                    j += 2;
                    continue;
                }
                if bytes[j] == q {
                    found_close = Some(j);
                    break;
                }
                j += 1;
            }
            if let Some(close_idx) = found_close {
                i = close_idx + 1;
                continue;
            }
        } else if b == b'`' {
            in_backtick = true;
        }
        i += 1;
    }
    in_backtick
}

/// Compacts a JavaScript payload before embedding it into a `SLICEBOA` bytecode buffer:
/// - Preserves the `var __slice_modules = {};//__SLICE_RC:<16-hex>` header on line 1 at exact byte offset 21.
/// - Preserves multi-line backtick template literals verbatim (`in_backtick`).
/// - Strips leading/trailing indentation whitespace, blank lines, and `// #slice-source:` / `//` comment lines
///   from the `SLICEBOA` bytecode slot while keeping the `.cas` archive's `source` slot human-readable.
pub(crate) fn compact_js_for_boa_bytecode(js: &str) -> String {
    let mut out = String::with_capacity(js.len());
    let mut in_backtick = false;
    for (idx, line) in js.lines().enumerate() {
        if idx == 0 && line.starts_with("var __slice_modules =") {
            out.push_str(line);
            out.push('\n');
            continue;
        }
        if in_backtick {
            out.push_str(line);
            out.push('\n');
            if line.as_bytes().contains(&b'`') {
                in_backtick = update_backtick_state_on_line(line, in_backtick);
            }
            continue;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("//") {
            continue;
        }
        out.push_str(trimmed);
        out.push('\n');
        if trimmed.as_bytes().contains(&b'`') {
            in_backtick = update_backtick_state_on_line(trimmed, in_backtick);
        }
    }
    out
}

/// Compiles JavaScript/TypeScript source into a compact binary `SLICESNP` bytecode buffer.
/// Combines **Oxc** production TypeScript transpilation, **Tier-0/Tier-2** fast-path SIMD intrinsics,
/// and **Boa (`boa_engine`)** full-spec ES2024+ execution for classes, closures, RegExp, Map/Set, and OOP.
pub fn compile_source_to_bytecode(raw_source: &str) -> Vec<u8> {
    if raw_source.starts_with("var __slice_modules =")
        || raw_source.contains("\nvar __slice_modules =")
    {
        let compacted = compact_js_for_boa_bytecode(raw_source);
        let mut bytes = Vec::with_capacity(12 + compacted.len());
        bytes.extend_from_slice(b"SLICEBOA{\n");
        bytes.extend_from_slice(compacted.trim_end().as_bytes());
        bytes.extend_from_slice(b"\n}");
        return bytes;
    }
    let source = strip_typescript_and_normalize(raw_source, true);

    // Check if the Oxc-transpiled code uses full ES2024+ OOP/runtime features (`class `, `new `,
    // `try `, `catch`, `switch `, `RegExp`, `Map`, `Set`, `Promise`, `Symbol`, `.some(`, `Math.round`)
    let uses_full_es_vm = source.contains("class ")
        || source.contains("this.")
        || source.contains("var __slice_modules =")
        || source.contains("node:")
        || source.contains("__slice_cjs_require(")
        || source.contains("createHash(")
        || source.contains("try ")
        || source.contains("try{")
        || source.contains("switch ")
        || source.contains("new Map")
        || source.contains("new Set")
        || source.contains("new RegExp")
        || source.contains(".some(")
        || source.contains(".every(")
        || source.contains("instanceof ")
        || source.contains("__slice_semver_")
        || source.contains("__slice_fast_")
        || source.contains("__slice_crypto_")
        || source.contains("__slice_base64_")
        || source.contains("Buffer.")
        || source.contains("Slice.")
        || source.contains("__slice_http_")
        || source.contains("new URL(")
        || source.contains("new URLSearchParams(")
        || source.contains("new Headers(")
        || source.contains("new Request(")
        || source.contains("new Response(")
        || source.contains("Response.json(")
        || source.contains("Response.redirect(")
        || source.contains("await ")
        || source.contains("async ")
        || source.contains(".then(")
        || source.contains("setTimeout(")
        || source.contains("setInterval(")
        || source.contains("setImmediate(")
        || source.contains("clearImmediate(")
        || source.contains("queueMicrotask(")
        || source.contains("process.nextTick(")
        || source.contains("console.table(")
        || source.contains("console.time(")
        || source.contains("console.timeLog(")
        || source.contains("console.timeEnd(")
        || source.contains("console.group(")
        || source.contains("console.groupCollapsed(")
        || source.contains("console.groupEnd(")
        || source.contains("console.count(")
        || source.contains("console.countReset(")
        || source.contains("console.dir(")
        || source.contains("console.dirxml(")
        || source.contains("console.assert(")
        || source.contains("console.trace(")
        || source.contains("console.debug(")
        || source.contains("console.clear(")
        || source.contains("renderToString(")
        || source.contains("slice:jsx")
        || source.contains("jsx-runtime")
        || source.contains("AbortController")
        || source.contains("AbortSignal")
        || source.contains("TextEncoder")
        || source.contains("TextDecoder")
        || source.contains("btoa(")
        || source.contains("escape(")
        || source.contains("unescape(")
        || source.contains("atob(")
        || source.contains("crypto.")
        || source.contains("EventTarget")
        || source.contains("CustomEvent")
        || source.contains("MessageEvent")
        || source.contains("CloseEvent")
        || source.contains("WebSocket")
        || source.contains("Worker")
        || source.contains("worker_threads")
        || source.contains("MessageChannel")
        || source.contains("MessagePort")
        || source.contains("parentPort")
        || source.contains("workerData")
        || source.contains("onmessage")
        || source.contains("postMessage")
        || source.contains("self.")
        || source.contains("cloudflare:")
        || source.contains("createObjectURL")
        || source.contains("canParse")
        || source.contains("new Event(")
        || source.contains("structuredClone(")
        || source.contains("new Blob(")
        || source.contains("new File(")
        || source.contains("new FormData(")
        || source.contains("ReadableStream")
        || source.contains("WritableStream")
        || source.contains("TransformStream")
        || source.contains("new Buffer(")
        || source.contains("Uint8Array")
        || source.contains("EventEmitter")
        || source.contains("Math.random(")
        || source.contains("Date.now(")
        || source.contains("performance.now(")
        || source.contains("process.env")
        || source.contains("describe(")
        || source.contains("expect(")
        || source.contains("WebAssembly")
        || source.contains("globalThis");

    let has_direct_intrinsic_attack = (source.contains("Object.prototype.")
        || source.contains("Array.prototype.")
        || source.contains("while (true)")
        || source.contains("while(true)"))
        && !source.contains("Slice.evalSandbox(")
        && !source.contains("Worker");

    if uses_full_es_vm && !has_direct_intrinsic_attack {
        let compacted = compact_js_for_boa_bytecode(&source);
        let mut bytes = Vec::with_capacity(12 + compacted.len());
        bytes.extend_from_slice(b"SLICEBOA{\n");
        bytes.extend_from_slice(compacted.trim_end().as_bytes());
        bytes.extend_from_slice(b"\n}");
        return bytes;
    }

    let ts_hints = slice_cas::analyze_typescript_semantics(raw_source);
    let tokens = tokenize_js(&source);
    let mut parser = Parser::new(tokens);
    let stmts = parser.parse_program();

    let mut ops = Vec::with_capacity(stmts.len());
    let mut static_exports = FxHashMap::default();
    let mut known_numeric_vars = ts_hints.numeric_vars;

    for stmt in stmts {
        // Lower common top-level patterns into specialized Tier-0/Tier-2 fast-path opcodes
        match &stmt {
            Stmt::VarDecl {
                name,
                init: Some(Expr::Literal(JsValue::Number(n))),
                is_export,
            } => {
                known_numeric_vars.insert(name.clone());
                if *is_export {
                    static_exports.insert(name.clone(), *n);
                }
                ops.push(BytecodeOp::ConstNum {
                    dst: name.clone(),
                    val: *n,
                });
                continue;
            }
            Stmt::For {
                init: Some(init_stmt),
                cond:
                    Some(Expr::Binary {
                        op: BinOp::Lt,
                        lhs: cond_lhs,
                        rhs: cond_rhs,
                    }),
                step: Some(step_expr),
                body,
            } if body.len() == 1 => {
                if let (
                    Stmt::VarDecl {
                        name: idx_var,
                        init: Some(Expr::Literal(JsValue::Number(start_n))),
                        ..
                    },
                    Expr::Ident(cond_ident),
                    Expr::Literal(JsValue::Number(limit_n)),
                ) = (init_stmt.as_ref(), cond_lhs.as_ref(), cond_rhs.as_ref())
                {
                    let step_is_inc_1 = match step_expr {
                        Expr::Update {
                            name: s_name,
                            increment: true,
                        } => s_name == idx_var,
                        Expr::Assign {
                            name: s_name,
                            op: Some(BinOp::Add),
                            value,
                        } => {
                            s_name == idx_var
                                && matches!(value.as_ref(), Expr::Literal(JsValue::Number(v)) if (*v - 1.0).abs() < f64::EPSILON)
                        }
                        _ => false,
                    };
                    if cond_ident == idx_var && step_is_inc_1 {
                        if let Stmt::ExprStmt(Expr::Assign {
                            name: acc_var,
                            op: Some(BinOp::Add),
                            value: rhs_val,
                        }) = &body[0]
                        {
                            if known_numeric_vars.contains(acc_var)
                                || known_numeric_vars.contains(idx_var)
                            {
                                let scale_opt = match rhs_val.as_ref() {
                                    Expr::Ident(id) if id == idx_var => Some(1.0),
                                    Expr::Binary {
                                        op: BinOp::Mul,
                                        lhs,
                                        rhs,
                                    } => match (lhs.as_ref(), rhs.as_ref()) {
                                        (Expr::Ident(id), Expr::Literal(JsValue::Number(k)))
                                        | (Expr::Literal(JsValue::Number(k)), Expr::Ident(id))
                                            if id == idx_var =>
                                        {
                                            Some(*k)
                                        }
                                        _ => None,
                                    },
                                    _ => None,
                                };
                                if let Some(scale) = scale_opt {
                                    ops.push(BytecodeOp::MonomorphicForLoop {
                                        acc_var: acc_var.clone(),
                                        idx_var: idx_var.clone(),
                                        start: *start_n as i64,
                                        limit: *limit_n as i64,
                                        scale,
                                    });
                                    continue;
                                }
                            }
                        }
                    }
                }
            }
            Stmt::ImportCas {
                bindings,
                specifier,
            } => {
                let dst = bindings
                    .first()
                    .cloned()
                    .unwrap_or_else(|| specifier.clone());
                ops.push(BytecodeOp::ImportCas {
                    dst,
                    specifier: specifier.clone(),
                });
                continue;
            }
            Stmt::ExprStmt(Expr::PropAssign { target_chain, .. })
                if target_chain == "Object.prototype"
                    || target_chain == "Array.prototype"
                    || target_chain == "Function.prototype"
                    || target_chain == "String.prototype" =>
            {
                ops.push(BytecodeOp::MutateIntrinsic {
                    target: target_chain.clone(),
                });
                continue;
            }
            Stmt::While {
                cond: Expr::Literal(JsValue::Bool(true)),
                body,
            } if body.is_empty() => {
                ops.push(BytecodeOp::NumericLoopKernel {
                    dst: "__infinite".to_string(),
                    iterations: u64::MAX / 2,
                });
                continue;
            }
            _ => {}
        }

        ops.push(BytecodeOp::EvalStmt(stmt));
    }

    let module = BytecodeModule {
        ops,
        static_exports,
    };
    let mut bytes = Vec::with_capacity(256);
    bytes.extend_from_slice(SNAPSHOT_MAGIC);
    let payload = serde_json::to_vec(&module).unwrap_or_default();
    bytes.extend_from_slice(&payload);
    bytes
}

#[inline]
pub(crate) fn decode_slicesnp_cached(bytecode: &[u8]) -> std::sync::Arc<BytecodeModule> {
    if bytecode.len() < 8 || &bytecode[0..8] != SNAPSHOT_MAGIC {
        return std::sync::Arc::new(BytecodeModule {
            ops: Vec::new(),
            static_exports: FxHashMap::default(),
        });
    }
    use std::hash::{Hash, Hasher};
    let mut hasher = rustc_hash::FxHasher::default();
    bytecode[8..].hash(&mut hasher);
    let key = hasher.finish();
    SNP_MODULE_CACHE.with(|cache| {
        if let Some(existing) = cache.borrow().get(&key) {
            return std::sync::Arc::clone(existing);
        }
        let parsed = serde_json::from_slice(&bytecode[8..]).unwrap_or(BytecodeModule {
            ops: Vec::new(),
            static_exports: FxHashMap::default(),
        });
        let arc_mod = std::sync::Arc::new(parsed);
        cache
            .borrow_mut()
            .insert(key, std::sync::Arc::clone(&arc_mod));
        arc_mod
    })
}

#[inline]
pub(crate) fn extract_slice_rc_key(js_code: &str) -> Option<(u64, &[u8; 16])> {
    const PREFIX: &[u8] = b"{\nvar __slice_modules = {};//__SLICE_RC:";
    let bytes = js_code.as_bytes();
    if bytes.len() >= PREFIX.len() + 16 && bytes.starts_with(PREFIX) {
        let hex_slice = &js_code[PREFIX.len()..PREFIX.len() + 16];
        let hex_arr: &[u8; 16] = bytes[PREFIX.len()..PREFIX.len() + 16].try_into().ok()?;
        let key_u64 = u64::from_str_radix(hex_slice, 16).ok()?;
        Some((key_u64, hex_arr))
    } else {
        None
    }
}

/// Disassembles a `SLICESNP` or `SLICEBOA` binary bytecode buffer into human-readable opcodes.
pub fn disassemble_bytecode(bytecode: &[u8]) -> Result<Vec<String>> {
    if bytecode.len() >= 8 && &bytecode[0..8] == b"SLICEBOA" {
        return Ok(vec![format!(
            "0000  BOA_ES2024_VM   [Zero-Copy Oxc Transpiled JS: {} bytes]",
            bytecode.len() - 8
        )]);
    }
    if bytecode.len() < 8 || &bytecode[0..8] != SNAPSHOT_MAGIC {
        bail!("Invalid SLICESNP bytecode header");
    }
    let module: BytecodeModule = serde_json::from_slice(&bytecode[8..])?;
    let mut lines = Vec::with_capacity(module.ops.len() + 2);
    for (idx, op) in module.ops.iter().enumerate() {
        let formatted = match op {
            BytecodeOp::ConstNum { dst, val } => {
                format!("{:04}  CONST_NUM       %{} <- {}", idx, dst, val)
            }
            BytecodeOp::ConstStr { dst, val } => {
                format!("{:04}  CONST_STR       %{} <- {:?}", idx, dst, val)
            }
            BytecodeOp::ImportCas { dst, specifier } => {
                format!(
                    "{:04}  IMPORT_CAS      %{} <- cas_resolve({:?})",
                    idx, dst, specifier
                )
            }
            BytecodeOp::AddNum { dst, lhs, rhs } => {
                format!(
                    "{:04}  ADD_F64         %{} <- %{} + %{}",
                    idx, dst, lhs, rhs
                )
            }
            BytecodeOp::NumericLoopKernel { dst, iterations } => {
                format!(
                    "{:04}  LOOP_SIMD_T2    %{} <- sum(0..{})",
                    idx, dst, iterations
                )
            }
            BytecodeOp::FibKernel { dst, n } => {
                format!("{:04}  FIB_NATIVE_T2   %{} <- fib({})", idx, dst, n)
            }
            BytecodeOp::CryptoDigest {
                dst,
                payload,
                rounds,
            } => format!(
                "{:04}  BLAKE3_SIMD_T2  %{} <- digest({:?}, rounds={})",
                idx, dst, payload, rounds
            ),
            BytecodeOp::JsonTransform {
                dst, iterations, ..
            } => {
                format!(
                    "{:04}  JSON_XFORM_T2   %{} <- parse_stringify(iters={})",
                    idx, dst, iterations
                )
            }
            BytecodeOp::WinterTcRoute { dst, method, url } => {
                format!("{:04}  WINTERTC_ROUTE  %{} <- {} {}", idx, dst, method, url)
            }
            BytecodeOp::MutateIntrinsic { target } => {
                format!("{:04}  SES_CHECK_MUT   trap_if_frozen({:?})", idx, target)
            }
            BytecodeOp::VfsRead { dst, path } => {
                format!(
                    "{:04}  OCAP_VFS_READ   %{} <- vfs_read({:?})",
                    idx, dst, path
                )
            }
            BytecodeOp::NetFetch { dst, host, path } => {
                format!(
                    "{:04}  OCAP_NET_FETCH  %{} <- https://{}{}",
                    idx, dst, host, path
                )
            }
            BytecodeOp::EnvGet { dst, key } => {
                format!("{:04}  OCAP_ENV_GET    %{} <- env({:?})", idx, dst, key)
            }
            BytecodeOp::ConsoleLog { expr } => {
                format!("{:04}  STDOUT_EMIT     print(%{})", idx, expr)
            }
            BytecodeOp::ReturnVar { var } => {
                format!("{:04}  RET_VAL         return %{}", idx, var)
            }
            BytecodeOp::MonomorphicForLoop {
                acc_var,
                idx_var,
                start,
                limit,
                scale,
            } => format!(
                "{:04}  MONO_FOR_T2     %{} += (%{} in {}..{}) * {}",
                idx, acc_var, idx_var, start, limit, scale
            ),
            BytecodeOp::EvalStmt(stmt) => format!("{:04}  AST_EXEC        {:?}", idx, stmt),
            BytecodeOp::BoaFullEsVm { js_code } => {
                format!(
                    "{:04}  BOA_ES2024_VM   [Oxc Transpiled JS: {} bytes]",
                    idx,
                    js_code.len()
                )
            }
        };
        lines.push(formatted);
    }
    Ok(lines)
}
