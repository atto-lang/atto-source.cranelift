//! HOST_ONLY typed Result<int,Error> parser and tenant-scoped identity contract.
//! NOT source-compiled Cranelift guest support or a transferable Error ABI.
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT_PARSER_GENERATION: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    tenant_id: u64,
    generation: u64,
    identity: u64,
    message: &'static str,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnwrapKind {
    OptionNone,
    ResultErr,
}
/// Host-only error envelope. The original Error is retained without formatting.
/// Fields are private to prevent minting an authorized envelope from host data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckedUnwrapError {
    error: Error,
    kind: UnwrapKind,
    original: Option<Error>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnwrapFailure {
    Ordinary(CheckedUnwrapError),
    ForeignError,
}
impl CheckedUnwrapError {
    pub fn kind(&self) -> &UnwrapKind {
        &self.kind
    }
    pub fn original(&self) -> Option<&Error> {
        self.original.as_ref()
    }
    pub fn error(&self) -> &Error {
        &self.error
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaggedResult {
    Ok(i64),
    Err(Error),
}
pub struct TenantParser {
    tenant_id: u64,
    generation: u64,
    next_error: u64,
}
impl TenantParser {
    pub const MAX_PARSE_BYTES: usize = 1024;
    pub fn new(tenant_id: u64) -> Result<Self, &'static str> {
        if tenant_id == 0 {
            return Err("E_TENANT_ID");
        }
        let generation = NEXT_PARSER_GENERATION
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |v| v.checked_add(1))
            .map_err(|_| "E_PARSER_GENERATION_EXHAUSTED")?;
        Ok(Self {
            tenant_id,
            generation,
            next_error: 0,
        })
    }
    pub fn can_observe(&self, err: &Error) -> bool {
        err.identity != 0 && err.tenant_id == self.tenant_id && err.generation == self.generation
    }
    fn mint(&mut self, message: &'static str) -> Error {
        self.next_error = self
            .next_error
            .checked_add(1)
            .expect("E_ERROR_ID_EXHAUSTED: fatal trusted runtime quota");
        Error {
            tenant_id: self.tenant_id,
            generation: self.generation,
            identity: self.next_error,
            message,
        }
    }
    fn invalid(&mut self) -> TaggedResult {
        TaggedResult::Err(self.mint("invalid integer"))
    }
    pub fn unwrap_option(&mut self, value: Option<i64>) -> Result<i64, UnwrapFailure> {
        match value {
            Some(n) => Ok(n),
            None => Err(UnwrapFailure::Ordinary(CheckedUnwrapError {
                error: self.mint("called Option::unwrap() on a None value"),
                kind: UnwrapKind::OptionNone,
                original: None,
            })),
        }
    }
    pub fn unwrap_result(&mut self, value: TaggedResult) -> Result<i64, UnwrapFailure> {
        match value {
            TaggedResult::Ok(n) => Ok(n),
            TaggedResult::Err(original) => {
                if !self.can_observe(&original) {
                    return Err(UnwrapFailure::ForeignError);
                }
                Err(UnwrapFailure::Ordinary(CheckedUnwrapError {
                    error: self.mint("called Result::unwrap() on an Err value"),
                    kind: UnwrapKind::ResultErr,
                    original: Some(original),
                }))
            }
        }
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
    fn checked_unwrap_retains_original_and_fences_parser_generations() {
        let mut owner = TenantParser::new(101).unwrap();
        let mut sibling = TenantParser::new(102).unwrap();
        let mut reincarnated = TenantParser::new(101).unwrap();
        for value in [i64::MIN, -1, 0, 42, i64::MAX] {
            assert_eq!(owner.unwrap_option(Some(value)), Ok(value));
            assert_eq!(owner.unwrap_result(TaggedResult::Ok(value)), Ok(value));
        }
        let Err(UnwrapFailure::Ordinary(none)) = owner.unwrap_option(None) else {
            panic!("None must become an ordinary failure");
        };
        assert_eq!(none.kind(), &UnwrapKind::OptionNone);
        assert_eq!(none.original(), None);
        assert!(owner.can_observe(none.error()));
        let TaggedResult::Err(original) = owner.parse("bad") else {
            panic!("expected parse failure");
        };
        assert!(!sibling.can_observe(&original));
        assert!(!reincarnated.can_observe(&original));
        for foreign in [&mut sibling, &mut reincarnated] {
            assert_eq!(
                foreign.unwrap_result(TaggedResult::Err(original.clone())),
                Err(UnwrapFailure::ForeignError)
            );
        }
        let Err(UnwrapFailure::Ordinary(wrapped)) =
            owner.unwrap_result(TaggedResult::Err(original.clone()))
        else {
            panic!("Err must become ordinary failure");
        };
        assert_eq!(wrapped.kind(), &UnwrapKind::ResultErr);
        assert_eq!(wrapped.original(), Some(&original));
        assert_ne!(wrapped.error(), &original);
        assert!(owner.can_observe(wrapped.error()));
        assert!(!reincarnated.can_observe(wrapped.error()));
        assert_eq!(wrapped, wrapped.clone());
        assert_ne!(
            owner.unwrap_result(TaggedResult::Err(original)),
            Err(UnwrapFailure::Ordinary(wrapped))
        );
    }
    #[test]
    fn concurrent_parser_incarnations_never_share_error_authority() {
        let workers: Vec<_> = (0..16).map(|_| std::thread::spawn(|| {
            let mut parser = TenantParser::new(7).unwrap();
            let TaggedResult::Err(error) = parser.parse("bad") else { panic!("parse accepted") };
            (parser, error)
        })).collect();
        let mut incarnations: Vec<_> = workers.into_iter().map(|w| w.join().unwrap()).collect();
        let errors: Vec<_> = incarnations.iter().map(|(_, err)| err.clone()).collect();
        for (i, (parser, own)) in incarnations.iter_mut().enumerate() {
            assert_eq!(own.identity, 1);
            for (j, error) in errors.iter().enumerate() {
                assert_eq!(parser.can_observe(error), i == j);
                if i != j {
                    assert_eq!(parser.unwrap_result(TaggedResult::Err(error.clone())),
                        Err(UnwrapFailure::ForeignError));
                }
            }
            let Err(UnwrapFailure::Ordinary(next)) = parser.unwrap_option(None) else {
                panic!("expected ordinary None");
            };
            assert_eq!(next.error().identity, 2, "foreign denial must not mint Error");
        }
    }
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
