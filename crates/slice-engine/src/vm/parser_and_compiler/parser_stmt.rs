//! Recursive-Descent Statement & Declaration Parser (`Parser`, `Stmt`) for Tier-0 `SLICESNP`.

use super::lexer::Token;
use crate::value_and_ast::Stmt;

#[inline]
pub(super) fn is_tier0_guarded_proto_target(chain: &str) -> bool {
    slice_sandbox::is_frozen_intrinsic(chain)
}

pub(crate) struct Parser {
    pub(super) tokens: Vec<Token>,
    pub(super) pos: usize,
}

impl Parser {
    pub(crate) fn new(tokens: Vec<Token>) -> Self {
        Self { tokens, pos: 0 }
    }

    pub(super) fn peek(&self) -> &Token {
        self.tokens.get(self.pos).unwrap_or(&Token::Eof)
    }

    pub(super) fn next(&mut self) -> Token {
        let tok = self.peek().clone();
        if self.pos < self.tokens.len() {
            self.pos += 1;
        }
        tok
    }

    pub(super) fn eat_semi(&mut self) {
        while matches!(self.peek(), Token::Semi) {
            self.next();
        }
    }

    /// Skip TypeScript `: Type` annotation if present after an identifier or `)`
    pub(super) fn skip_optional_ts_type(&mut self) {
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

    pub(super) fn parse_stmt(&mut self) -> Option<Stmt> {
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
        } else if let Token::Ident(name) = self.peek().clone()
            && name != "from"
        {
            self.next();
            bindings.push(name);
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

    pub(super) fn parse_block(&mut self) -> Vec<Stmt> {
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
}
