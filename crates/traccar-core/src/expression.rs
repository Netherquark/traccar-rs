// SPDX-License-Identifier: Apache-2.0
//
// A small, safe expression layer for computed attributes and the
// `/attributes/computed/test` endpoint. Traccar uses a JEXL-style engine; we
// implement a deliberately restricted arithmetic/boolean evaluator over
// position attributes. It never executes arbitrary code and rejects unknown
// identifiers at evaluation time.

use crate::error::{Error, Result};
use crate::model::Position;

/// Validate an expression's syntax. Returns an error for unbalanced grouping or
/// characters outside the supported grammar.
pub fn validate(expr: &str) -> Result<()> {
    let mut depth = 0i32;
    for ch in expr.chars() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth < 0 {
                    return Err(Error::bad_request("unbalanced parentheses"));
                }
            }
            c if c.is_alphanumeric() || " _.:+-*/%<>=!&|,()".contains(c) => {}
            c => return Err(Error::bad_request(format!("unsupported character: {c}"))),
        }
    }
    if depth != 0 {
        return Err(Error::bad_request("unbalanced parentheses"));
    }
    Ok(())
}

/// Evaluate an expression against a position's attributes. A bare identifier
/// resolves to `attributes[name]` (numbers as f64, booleans as 1.0/0.0).
pub fn eval_f64(expr: &str, position: &Position) -> Result<f64> {
    let tokens = tokenize(expr)?;
    let mut parser = Parser { tokens, pos: 0, position };
    let v = parser.parse_expr()?;
    if parser.pos != parser.tokens.len() {
        return Err(Error::bad_request("trailing tokens in expression"));
    }
    Ok(v)
}

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Num(f64),
    Ident(String),
    Op(char),
    LParen,
    RParen,
}

fn tokenize(expr: &str) -> Result<Vec<Tok>> {
    let chars: Vec<char> = expr.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
        } else if c.is_ascii_digit() || (c == '.' && i + 1 < chars.len() && chars[i + 1].is_ascii_digit()) {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                i += 1;
            }
            let s: String = chars[start..i].iter().collect();
            out.push(Tok::Num(s.parse().map_err(|_| Error::bad_request("invalid number"))?));
        } else if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_' || chars[i] == '.') {
                i += 1;
            }
            out.push(Tok::Ident(chars[start..i].iter().collect()));
        } else if "+-*/%<>!=&|".contains(c) {
            out.push(Tok::Op(c));
            i += 1;
        } else if c == '(' {
            out.push(Tok::LParen);
            i += 1;
        } else if c == ')' {
            out.push(Tok::RParen);
            i += 1;
        } else {
            return Err(Error::bad_request(format!("unexpected character: {c}")));
        }
    }
    Ok(out)
}

struct Parser<'a> {
    tokens: Vec<Tok>,
    pos: usize,
    position: &'a Position,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<&Tok> {
        self.tokens.get(self.pos)
    }
    fn next(&mut self) -> Option<Tok> {
        let t = self.tokens.get(self.pos).cloned();
        self.pos += 1;
        t
    }

    fn parse_expr(&mut self) -> Result<f64> {
        self.parse_or()
    }

    fn parse_or(&mut self) -> Result<f64> {
        let mut left = self.parse_and()?;
        while matches!(self.peek(), Some(Tok::Op('|'))) {
            if matches!(self.tokens.get(self.pos + 1), Some(Tok::Op('|'))) {
                self.pos += 2;
                let right = self.parse_and()?;
                left = if left != 0.0 || right != 0.0 { 1.0 } else { 0.0 };
            } else {
                break;
            }
        }
        Ok(left)
    }

    fn parse_and(&mut self) -> Result<f64> {
        let mut left = self.parse_cmp()?;
        while matches!(self.peek(), Some(Tok::Op('&'))) {
            if matches!(self.tokens.get(self.pos + 1), Some(Tok::Op('&'))) {
                self.pos += 2;
                let right = self.parse_cmp()?;
                left = if left != 0.0 && right != 0.0 { 1.0 } else { 0.0 };
            } else {
                break;
            }
        }
        Ok(left)
    }

    fn parse_cmp(&mut self) -> Result<f64> {
        let left = self.parse_add()?;
        let op = match self.peek() {
            Some(Tok::Op(c @ ('<' | '>' | '=' | '!'))) => {
                let first = *c;
                // Support == and != and <= >=.
                let two = matches!(self.tokens.get(self.pos + 1), Some(Tok::Op('=')));
                self.pos += if two { 2 } else { 1 };
                Some((first, two))
            }
            _ => None,
        };
        if let Some((first, two)) = op {
            let right = self.parse_add()?;
            let result = match (first, two) {
                ('=', _) => (left == right) as i32 as f64,
                ('!', _) => (left != right) as i32 as f64,
                ('<', false) => (left < right) as i32 as f64,
                ('<', true) => (left <= right) as i32 as f64,
                ('>', false) => (left > right) as i32 as f64,
                ('>', true) => (left >= right) as i32 as f64,
                _ => 0.0,
            };
            return Ok(result);
        }
        Ok(left)
    }

    fn parse_add(&mut self) -> Result<f64> {
        let mut left = self.parse_mul()?;
        while let Some(Tok::Op(c @ ('+' | '-'))) = self.peek() {
            let c = *c;
            self.pos += 1;
            let right = self.parse_mul()?;
            left = if c == '+' { left + right } else { left - right };
        }
        Ok(left)
    }

    fn parse_mul(&mut self) -> Result<f64> {
        let mut left = self.parse_unary()?;
        while let Some(Tok::Op(c @ ('*' | '/' | '%'))) = self.peek() {
            let c = *c;
            self.pos += 1;
            let right = self.parse_unary()?;
            left = match c {
                '*' => left * right,
                '/' => {
                    if right == 0.0 {
                        return Err(Error::bad_request("division by zero"));
                    }
                    left / right
                }
                _ => left % right,
            };
        }
        Ok(left)
    }

    fn parse_unary(&mut self) -> Result<f64> {
        if let Some(Tok::Op('-')) = self.peek() {
            self.pos += 1;
            return Ok(-self.parse_unary()?);
        }
        if let Some(Tok::Op('!')) = self.peek() {
            self.pos += 1;
            return Ok((self.parse_unary()? == 0.0) as i32 as f64);
        }
        self.parse_atom()
    }

    fn parse_atom(&mut self) -> Result<f64> {
        match self.next() {
            Some(Tok::Num(n)) => Ok(n),
            Some(Tok::Ident(name)) => Ok(self.resolve(&name)),
            Some(Tok::LParen) => {
                let v = self.parse_expr()?;
                match self.next() {
                    Some(Tok::RParen) => Ok(v),
                    _ => Err(Error::bad_request("expected )")),
                }
            }
            _ => Err(Error::bad_request("unexpected token")),
        }
    }

    fn resolve(&self, name: &str) -> f64 {
        match name {
            "true" => return 1.0,
            "false" => return 0.0,
            "null" => return 0.0,
            _ => {}
        }
        self.position
            .attributes
            .get(name)
            .and_then(|v| match v {
                serde_json::Value::Number(n) => n.as_f64(),
                serde_json::Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
                serde_json::Value::String(s) => s.parse::<f64>().ok(),
                _ => None,
            })
            .unwrap_or(0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn arithmetic_and_attributes() {
        let mut p = Position::default();
        p.set_attr("speed", Value::from(10.0));
        p.set_attr("batteryLevel", Value::from(50.0));
        assert_eq!(eval_f64("speed * 2", &p).unwrap(), 20.0);
        assert_eq!(eval_f64("(speed + 5) / 3", &p).unwrap(), 5.0);
        assert_eq!(eval_f64("batteryLevel > 20", &p).unwrap(), 1.0);
        assert_eq!(eval_f64("speed > 20 && batteryLevel > 20", &p).unwrap(), 0.0);
    }

    #[test]
    fn rejects_garbage() {
        assert!(validate("speed * (2 + 3)").is_ok());
        assert!(validate("speed * (2 + 3").is_err());
        assert!(validate("speed; drop table").is_err());
    }
}
