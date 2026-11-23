//! Fast ASCII JavaScript/TypeScript Tokenizer (`Token`, `tokenize_js`) for Tier-0 `SLICESNP`.

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
    PercentEq,
    Eq,
    EqEq,
    EqEqEq,
    Bang,
    BangEq,
    BangEqEq,
    Tilde,
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
    UShr,
    Eof,
}

pub(crate) fn tokenize_js(input: &str) -> Option<Vec<Token>> {
    if !input.is_ascii() {
        return None;
    }
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
                if quote == b'`' && bytes[i] == b'$' && i + 1 < bytes.len() && bytes[i + 1] == b'{'
                {
                    return None;
                }
                if bytes[i] == b'\\' && i + 1 < bytes.len() {
                    i += 1;
                    match bytes[i] {
                        b'n' => s.push('\n'),
                        b'r' => s.push('\r'),
                        b't' => s.push('\t'),
                        b'x' | b'u' | b'0'..=b'7' => return None,
                        other => s.push(other as char),
                    }
                } else {
                    s.push(bytes[i] as char);
                }
                i += 1;
            }
            if i >= bytes.len() {
                return None;
            }
            i += 1;
            tokens.push(Token::Str(s));
            continue;
        }
        // Number literal (including leading-dot decimals like `.5` emitted by Oxc codegen)
        if b.is_ascii_digit() || (b == b'.' && i + 1 < bytes.len() && bytes[i + 1].is_ascii_digit())
        {
            let start = i;
            if b == b'0' && i + 1 < bytes.len() {
                match bytes[i + 1] {
                    b'b' | b'B' | b'o' | b'O' => return None,
                    b'x' | b'X' => {
                        i += 2;
                        let hex_start = i;
                        while i < bytes.len() && (bytes[i].is_ascii_hexdigit() || bytes[i] == b'_')
                        {
                            if bytes[i] == b'_' {
                                return None;
                            }
                            i += 1;
                        }
                        if i == hex_start {
                            return None;
                        }
                        let hex_str = &input[hex_start..i];
                        let val = u64::from_str_radix(hex_str, 16).ok()?;
                        if val > (1u64 << 53) {
                            return None;
                        }
                        tokens.push(Token::Num(val as f64));
                        continue;
                    }
                    _ => {}
                }
            }
            let mut seen_dot = b == b'.';
            let mut has_underscore = false;
            if seen_dot {
                i += 1;
            }
            while i < bytes.len() {
                if bytes[i].is_ascii_digit() {
                    i += 1;
                } else if bytes[i] == b'_' {
                    has_underscore = true;
                    i += 1;
                } else if bytes[i] == b'.' && !seen_dot {
                    seen_dot = true;
                    i += 1;
                } else {
                    break;
                }
            }
            if i < bytes.len() && (bytes[i] == b'e' || bytes[i] == b'E') {
                i += 1;
                if i < bytes.len() && (bytes[i] == b'+' || bytes[i] == b'-') {
                    i += 1;
                }
                while i < bytes.len() {
                    if bytes[i].is_ascii_digit() {
                        i += 1;
                    } else if bytes[i] == b'_' {
                        has_underscore = true;
                        i += 1;
                    } else {
                        break;
                    }
                }
            }
            let val = if !has_underscore {
                input[start..i].parse::<f64>().unwrap_or(0.0)
            } else {
                let num_str: String = input[start..i].chars().filter(|&c| c != '_').collect();
                num_str.parse::<f64>().unwrap_or(0.0)
            };
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
        } else if rem.starts_with(b">>>") {
            tokens.push(Token::UShr);
            i += 3;
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
        } else if rem.starts_with(b"%=") {
            tokens.push(Token::PercentEq);
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
                b'~' => tokens.push(Token::Tilde),
                b'<' => tokens.push(Token::Lt),
                b'>' => tokens.push(Token::Gt),
                b'&' => tokens.push(Token::Amp),
                b'|' => tokens.push(Token::Pipe),
                b'^' => tokens.push(Token::Caret),
                _ => return None,
            }
            i += 1;
        }
    }

    tokens.push(Token::Eof);
    Some(tokens)
}
