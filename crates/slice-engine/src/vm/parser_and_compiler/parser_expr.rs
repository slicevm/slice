//! Precedence-Climbing Expression Parser (`Expr`) for Tier-0 `SLICESNP`.

use super::lexer::Token;
use super::parser_stmt::Parser;
use crate::value_and_ast::{BinOp, Expr, JsFunction, JsValue, Stmt, UnaryOp};

impl Parser {
    pub(super) fn parse_expr(&mut self) -> Expr {
        self.parse_assign()
    }

    fn parse_assign(&mut self) -> Expr {
        let lhs = self.parse_ternary();
        match self.peek().clone() {
            Token::Eq
            | Token::PlusEq
            | Token::MinusEq
            | Token::StarEq
            | Token::SlashEq
            | Token::PercentEq => {
                let op_tok = self.next();
                let assign_op = match op_tok {
                    Token::PlusEq => Some(BinOp::Add),
                    Token::MinusEq => Some(BinOp::Sub),
                    Token::StarEq => Some(BinOp::Mul),
                    Token::SlashEq => Some(BinOp::Div),
                    Token::PercentEq => Some(BinOp::Rem),
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
                    Expr::Index { object, index } => match (*object, *index) {
                        (Expr::Ident(target), idx_expr) => Expr::IndexAssign {
                            target,
                            index: Box::new(idx_expr),
                            value: Box::new(rhs),
                        },
                        (obj_expr, Expr::Literal(JsValue::Str(prop))) => {
                            let chain = expr_to_dotted_chain(&obj_expr);
                            if !chain.is_empty() {
                                Expr::PropAssign {
                                    target_chain: chain,
                                    prop: prop.to_string(),
                                    value: Box::new(rhs),
                                }
                            } else {
                                rhs
                            }
                        }
                        _ => rhs,
                    },
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
        let mut left = self.parse_bit_or();
        while matches!(self.peek(), Token::AmpAmp) {
            self.next();
            let right = self.parse_bit_or();
            left = Expr::Binary {
                op: BinOp::And,
                lhs: Box::new(left),
                rhs: Box::new(right),
            };
        }
        left
    }

    fn parse_bit_or(&mut self) -> Expr {
        let mut left = self.parse_bit_xor();
        while matches!(self.peek(), Token::Pipe) {
            self.next();
            let right = self.parse_bit_xor();
            left = Expr::Binary {
                op: BinOp::BitOr,
                lhs: Box::new(left),
                rhs: Box::new(right),
            };
        }
        left
    }

    fn parse_bit_xor(&mut self) -> Expr {
        let mut left = self.parse_bit_and();
        while matches!(self.peek(), Token::Caret) {
            self.next();
            let right = self.parse_bit_and();
            left = Expr::Binary {
                op: BinOp::BitXor,
                lhs: Box::new(left),
                rhs: Box::new(right),
            };
        }
        left
    }

    fn parse_bit_and(&mut self) -> Expr {
        let mut left = self.parse_equality();
        while matches!(self.peek(), Token::Amp) {
            self.next();
            let right = self.parse_equality();
            left = Expr::Binary {
                op: BinOp::BitAnd,
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
                Token::UShr => BinOp::UShr,
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
            Token::Tilde => {
                self.next();
                Expr::Unary {
                    op: UnaryOp::BitNot,
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
                        prefix: true,
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
                        prefix: true,
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
                            prefix: false,
                        };
                    }
                }
                Token::MinusMinus => {
                    self.next();
                    if let Expr::Ident(name) = expr {
                        expr = Expr::Update {
                            name,
                            increment: false,
                            prefix: false,
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
            let prefix = expr_to_dotted_chain(object);
            if prefix.is_empty() {
                String::new()
            } else {
                format!("{}.{}", prefix, property)
            }
        }
        Expr::Index { object, index } => {
            if let Expr::Literal(JsValue::Str(prop)) = index.as_ref() {
                let prefix = expr_to_dotted_chain(object);
                if !prefix.is_empty() {
                    return format!("{}.{}", prefix, prop);
                }
            }
            String::new()
        }
        _ => String::new(),
    }
}
