//! Lexical-only single/double-quoted and interpolated backtick scanner.
//! Cranelift stage-0 has no String IR or native String ABI. This scanner
//! preserves correct source boundaries without granting execution support.

fn err(message: &str, offset: usize) -> String {
    format!("{message} at byte {offset}")
}

struct Scanner<'a> {
    bytes: &'a [u8],
    interpolations: usize,
}

impl Scanner<'_> {
    fn newline(&self, i: usize) -> usize {
        match self.bytes.get(i).copied() {
            Some(b'\r') => {
                if self.bytes.get(i + 1) == Some(&b'\n') { 2 } else { 1 }
            }
            Some(b'\n') => 1,
            Some(0xc2) if self.bytes.get(i + 1) == Some(&0x85) => 2,
            Some(0xe2) if self.bytes.get(i + 1) == Some(&0x80) &&
                matches!(self.bytes.get(i + 2), Some(0xa8 | 0xa9)) => 3,
            _ => 0,
        }
    }

    fn hex(value: u8) -> Option<u32> {
        match value {
            b'0'..=b'9' => Some(u32::from(value - b'0')),
            b'a'..=b'f' => Some(u32::from(value - b'a' + 10)),
            b'A'..=b'F' => Some(u32::from(value - b'A' + 10)),
            _ => None,
        }
    }

    fn unicode_unit(&self, index: &mut usize, escape: usize) -> Result<u32, String> {
        let mut scalar = 0_u32;
        for _ in 0..4 {
            let digit = self.bytes.get(*index).and_then(|&c| Self::hex(c))
                .ok_or_else(|| err("invalid Unicode escape", escape))?;
            scalar = scalar * 16 + digit;
            *index += 1;
        }
        Ok(scalar)
    }
    fn quoted(&mut self, start: usize, depth: usize) -> Result<usize, String> {
        if depth > 32 { return Err(err("string/template nesting limit", start)); }
        let quote = *self.bytes.get(start).ok_or_else(|| err("expected quote", start))?;
        if !matches!(quote, b'\'' | b'"' | b'`') {
            return Err(err("expected quote", start));
        }
        let mut i = start + 1;
        while i < self.bytes.len() {
            let ch = self.bytes[i];
            if ch == b'\\' {
                let at = i;
                i += 1;
                let next = *self.bytes.get(i).ok_or_else(|| err("unterminated escape", at))?;
                let line_bytes = self.newline(i);
                if line_bytes > 0 {
                    if quote == b'`' { return Err(err("invalid template escape", at)); }
                    i += line_bytes;
                    continue;
                }
                if next == b'u' {
                    i += 1;
                    let scalar = self.unicode_unit(&mut i, at)?;
                    if (0xd800..=0xdbff).contains(&scalar) {
                        if self.bytes.get(i) != Some(&b'\\') ||
                            self.bytes.get(i + 1) != Some(&b'u') {
                            return Err(err("invalid Unicode escape", at));
                        }
                        i += 2;
                        let low = self.unicode_unit(&mut i, at)?;
                        if !(0xdc00..=0xdfff).contains(&low) {
                            return Err(err("invalid Unicode escape", at));
                        }
                    } else if (0xdc00..=0xdfff).contains(&scalar) {
                        return Err(err("invalid Unicode escape", at));
                    }
                    continue;
                }
                // Java retains any escaped character in ordinary quotes,
                // but template literals use a strict escape whitelist.
                if quote == b'`' && !matches!(next,
                    b'\\' | b'\'' | b'"' | b'`' | b'$' | b'n' | b'r' | b't') {
                    return Err(err("invalid template escape", at));
                }
                i += 1;
                continue;
            }
            if ch == quote { return Ok(i + 1); }
            // Java/Graal ordinary strings preserve raw multiline content.
            if quote == b'`' && ch == b'$' && self.bytes.get(i + 1) == Some(&b'{') {
                self.interpolations += 1;
                if self.interpolations > 1024 {
                    return Err(err("template substitution limit", i));
                }
                let body = i + 2;
                let end = self.interpolation(body, depth + 1)?;
                if self.bytes[body..end - 1].iter().all(u8::is_ascii_whitespace) {
                    return Err(err("empty template interpolation", i));
                }
                i = end;
                continue;
            }
            i += 1;
        }
        Err(err("unterminated string/template", start))
    }

    fn interpolation(&mut self, at: usize, depth: usize) -> Result<usize, String> {
        if depth > 32 { return Err(err("template nesting limit", at)); }
        let mut i = at;
        let mut braces = 1_usize;
        let mut has_code = false;
        while i < self.bytes.len() {
            if self.bytes.get(i..i + 2) == Some(b"//") {
                i += 2;
                while i < self.bytes.len() && self.newline(i) == 0 { i += 1; }
                continue;
            }
            if self.bytes.get(i..i + 2) == Some(b"/*") {
                let comment_at = i;
                i += 2;
                let mut nested = 1_usize;
                while i < self.bytes.len() && nested > 0 {
                    if self.bytes.get(i..i + 2) == Some(b"/*") {
                        nested += 1;
                        if nested > 64 { return Err(err("comment nesting limit", i)); }
                        i += 2;
                    } else if self.bytes.get(i..i + 2) == Some(b"*/") {
                        nested -= 1;
                        i += 2;
                    } else { i += 1; }
                }
                if nested > 0 { return Err(err("unterminated interpolation comment", comment_at)); }
                continue;
            }
            if matches!(self.bytes[i], b'\'' | b'"' | b'`') {
                has_code = true;
                i = self.quoted(i, depth + 1)?;
                continue;
            }
            if self.bytes[i] == b'{' {
                has_code = true;
                braces += 1;
                if braces > 256 { return Err(err("interpolation brace limit", i)); }
            } else if self.bytes[i] == b'}' {
                braces -= 1;
                if braces == 0 {
                    if !has_code { return Err(err("empty template interpolation", at.saturating_sub(2))); }
                    return Ok(i + 1);
                }
                has_code = true;
            } else if self.newline(i) > 0 {
                i += self.newline(i);
                continue;
            } else if !self.bytes[i].is_ascii_whitespace() {
                has_code = true;
            }
            i += 1;
        }
        Err(err("unterminated template interpolation", at.saturating_sub(2)))
    }
}

/// Returns the first byte after the complete quoted literal.
pub(crate) fn scan_quoted(source: &str, start: usize) -> Result<usize, String> {
    let mut scanner = Scanner { bytes: source.as_bytes(), interpolations: 0 };
    scanner.quoted(start, 0)
}
