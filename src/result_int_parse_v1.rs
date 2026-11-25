//! HOST_ONLY typed Result<int,Error> parser and tenant-scoped identity contract.
//! NOT source-compiled Cranelift guest support or a transferable Error ABI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    pub tenant_id: u64,
    pub identity: u64,
    pub message: &'static str,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaggedResult {
    Ok(i64),
    Err(Error),
}
pub struct TenantParser {
    tenant_id: u64,
    next_error: u64,
}
impl TenantParser {
    pub const MAX_PARSE_BYTES: usize = 1024;
    pub fn new(tenant_id: u64) -> Result<Self, &'static str> {
        if tenant_id == 0 {
            return Err("E_TENANT_ID");
        }
        Ok(Self {
            tenant_id,
            next_error: 0,
        })
    }
    pub fn can_observe(&self, err: &Error) -> bool {
        err.identity != 0 && err.tenant_id == self.tenant_id
    }
    fn invalid(&mut self) -> TaggedResult {
        self.next_error = self
            .next_error
            .checked_add(1)
            .expect("E_ERROR_ID_EXHAUSTED: fatal trusted runtime quota");
        TaggedResult::Err(Error {
            tenant_id: self.tenant_id,
            identity: self.next_error,
            message: "invalid integer",
        })
    }
    pub fn parse(&mut self, text: &str) -> TaggedResult {
        let bytes = text.as_bytes();
        if bytes.is_empty() || bytes.len() > Self::MAX_PARSE_BYTES {
            return self.invalid();
        }
        let mut at = 0;
        let mut negative = false;
        if bytes[0] == b'-' || bytes[0] == b'+' {
            negative = bytes[0] == b'-';
            at += 1;
        }
        if at == bytes.len() {
            return self.invalid();
        }
        let limit = if negative { i64::MIN } else { -i64::MAX };
        let cutoff = limit / 10;
        let mut value = 0i64;
        for &ch in &bytes[at..] {
            if !ch.is_ascii_digit() {
                return self.invalid();
            }
            let digit = i64::from(ch - b'0');
            if value < cutoff {
                return self.invalid();
            }
            value *= 10;
            if value < limit + digit {
                return self.invalid();
            }
            value -= digit;
        }
        TaggedResult::Ok(if negative { value } else { -value })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn checked_decimal_40k_boundary_and_owned_identity() {
        let mut a = TenantParser::new(101).unwrap();
        let mut b = TenantParser::new(102).unwrap();
        for n in -20_000i64..=20_000 {
            assert_eq!(a.parse(&n.to_string()), TaggedResult::Ok(n));
        }
        for (text, value) in [
            ("0", 0),
            ("+42", 42),
            ("-0", 0),
            ("00042", 42),
            ("-9223372036854775808", i64::MIN),
            ("9223372036854775807", i64::MAX),
        ] {
            assert_eq!(a.parse(text), TaggedResult::Ok(value));
        }
        for invalid in [
            "",
            "+",
            "-",
            " 42",
            "42 ",
            "++1",
            "-+1",
            "4_2",
            "42x",
            "9223372036854775808",
            "-9223372036854775809",
            "٤٢",
            "0\0",
        ] {
            let TaggedResult::Err(error) = a.parse(invalid) else {
                panic!("invalid input was admitted");
            };
            assert_eq!(error.message, "invalid integer");
            assert!(a.can_observe(&error));
            assert!(!b.can_observe(&error));
        }
        assert_eq!(a.parse(&"0".repeat(1024)), TaggedResult::Ok(0));
        assert!(matches!(a.parse(&"0".repeat(1025)), TaggedResult::Err(_)));
        let one = a.parse("bad");
        let propagated = one.clone(); // original error identity retained
        assert_eq!(one, propagated);
        assert_ne!(a.parse("bad"), propagated); // independent Error identity
        assert!(matches!(b.parse("bad"), TaggedResult::Err(_)));
        assert!(TenantParser::new(0).is_err());
    }
}
