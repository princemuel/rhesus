use alloc::borrow::Cow;
use alloc::collections::BTreeMap;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::{fmt, ptr};

use crate::ast::{Block, Expr};
use crate::environment::Environment;
use crate::errors::EvalError;
use crate::host::Host;

pub type Map<K, V> = BTreeMap<K, V>;

pub type BuiltinResult<'a> = Result<Object<'a>, EvalError<'a>>;

/// A native function's type in Monkey
pub type BuiltinFn<'a> = fn(&[Object<'a>], &mut dyn Host) -> BuiltinResult<'a>;

/// Only these object values may be hash keys.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum HashKey<'a> {
    Integer(i64),
    Boolean(bool),
    String(Cow<'a, str>),
}
impl fmt::Display for HashKey<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Integer(n) => write!(f, "{n}"),
            Self::Boolean(b) => write!(f, "{b}"),
            Self::String(s) => f.write_str(s),
        }
    }
}
impl<'a> TryFrom<&Object<'a>> for HashKey<'a> {
    type Error = EvalError<'a>;

    fn try_from(obj: &Object<'a>) -> Result<Self, EvalError<'a>> {
        match obj {
            Object::Integer(n) => Ok(Self::Integer(*n)),
            Object::Boolean(b) => Ok(Self::Boolean(*b)),
            Object::String(s) => Ok(Self::String(s.clone())),
            other => Err(EvalError::UnusableAsHashKey(other.type_name())),
        }
    }
}

pub struct Function<'a> {
    pub params: Rc<[&'a str]>,
    pub body: Rc<Block<'a>>,
    pub env: Environment<'a>,
}
impl fmt::Debug for Function<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Function")
            .field("params", &self.params)
            .field("body", &self.body)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug, Default)]
pub enum Object<'a> {
    #[default]
    Null,
    Integer(i64),
    Boolean(bool),
    String(Cow<'a, str>),
    Array(Rc<[Self]>),
    Hash(Rc<Map<HashKey<'a>, Self>>),
    Function(Rc<Function<'a>>),
    Builtin(BuiltinFn<'a>),
    /// A piece of unevaluated code: the result of `quote(...)`
    Quote(Rc<Expr<'a>>),
    /// this is similar to a function but runs at expansion time instead
    Macro(Rc<Function<'a>>),
}

impl Object<'_> {
    #[must_use]
    pub const fn type_name(&self) -> &'static str {
        match self {
            Self::Null => "NULL",
            Self::Integer(_) => "INTEGER",
            Self::Boolean(_) => "BOOLEAN",
            Self::String(_) => "STRING",
            Self::Array(_) => "ARRAY",
            Self::Hash(_) => "HASH",
            Self::Function(_) => "FUNCTION",
            Self::Builtin(_) => "BUILTIN",
            Self::Quote(_) => "QUOTE",
            Self::Macro(_) => "MACRO",
        }
    }

    /// all object values are truthy except `false` and `null`
    #[must_use]
    pub const fn is_truthy(&self) -> bool { !matches!(self, Self::Null | Self::Boolean(false)) }
}

/// What `==` means in Monkey
impl PartialEq for Object<'_> {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Null, Self::Null) => true,
            (Self::Integer(a), Self::Integer(b)) => a == b,
            (Self::Boolean(a), Self::Boolean(b)) => a == b,
            (Self::String(a), Self::String(b)) => a == b,
            (Self::Array(a), Self::Array(b)) => a == b,
            (Self::Hash(a), Self::Hash(b)) => a == b,
            (Self::Function(a), Self::Function(b)) | (Self::Macro(a), Self::Macro(b)) => {
                Rc::ptr_eq(a, b)
            }
            (Self::Builtin(a), Self::Builtin(b)) => ptr::fn_addr_eq(*a, *b),
            (Self::Quote(a), Self::Quote(b)) => a == b,
            _ => false,
        }
    }
}

impl fmt::Display for Object<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Null => f.write_str("null"),
            Self::Integer(n) => write!(f, "{n}"),
            Self::Boolean(b) => write!(f, "{b}"),
            Self::String(s) => f.write_str(s),
            Self::Array(items) => {
                f.write_str("[")?;

                let mut iter = items.iter();
                if let Some(first) = iter.next() {
                    write!(f, "{first}")?;
                    iter.try_for_each(|item| write!(f, ", {item}"))?;
                }

                f.write_str("]")
            }
            Self::Hash(map) => {
                f.write_str("{")?;

                let mut iter = map.iter();
                if let Some((k, v)) = iter.next() {
                    write!(f, "{k}: {v}")?;
                    iter.try_for_each(|(k, v)| write!(f, ", {k}: {v}"))?;
                }

                f.write_str("}")
            }
            Self::Function(func) => {
                write!(f, "fn({}) {{\n{}\n}}", func.params.join(", "), func.body)
            }
            Self::Builtin(_) => f.write_str("builtin function"),
            Self::Quote(node) => write!(f, "QUOTE({node})"),
            Self::Macro(mac) => {
                write!(f, "macro({}) {{\n{}\n}}", mac.params.join(", "), mac.body)
            }
        }
    }
}

impl From<bool> for Object<'_> {
    fn from(value: bool) -> Self { Self::Boolean(value) }
}

impl From<i64> for Object<'_> {
    fn from(value: i64) -> Self { Self::Integer(value) }
}

impl<'a> From<&'a str> for Object<'a> {
    fn from(value: &'a str) -> Self { Self::String(Cow::Borrowed(value)) }
}

impl From<String> for Object<'_> {
    fn from(value: String) -> Self { Self::String(Cow::Owned(value)) }
}

impl<'a> From<Cow<'a, str>> for Object<'a> {
    fn from(value: Cow<'a, str>) -> Self { Self::String(value) }
}

impl From<()> for Object<'_> {
    fn from((): ()) -> Self { Self::Null }
}

impl<T: Into<Self>> From<Option<T>> for Object<'_> {
    fn from(value: Option<T>) -> Self { value.map_or(Self::Null, Into::into) }
}

impl From<Vec<Self>> for Object<'_> {
    fn from(value: Vec<Self>) -> Self { Self::Array(value.into()) }
}

impl<'a> From<HashKey<'a>> for Object<'a> {
    fn from(key: HashKey<'a>) -> Self {
        match key {
            HashKey::Integer(n) => Self::Integer(n),
            HashKey::Boolean(b) => Self::Boolean(b),
            HashKey::String(s) => Self::String(s),
        }
    }
}

impl<'a> From<Map<HashKey<'a>, Self>> for Object<'a> {
    fn from(value: Map<HashKey<'a>, Self>) -> Self { Self::Hash(Rc::new(value)) }
}

impl<'a> From<Function<'a>> for Object<'a> {
    fn from(value: Function<'a>) -> Self { Self::Function(Rc::new(value)) }
}

impl<'a> From<Rc<Function<'a>>> for Object<'a> {
    fn from(value: Rc<Function<'a>>) -> Self { Self::Function(value) }
}

impl<'a> From<BuiltinFn<'a>> for Object<'a> {
    fn from(value: BuiltinFn<'a>) -> Self { Self::Builtin(value) }
}

/// Lets you `.collect::<Object>()` straight into an array.
impl FromIterator<Self> for Object<'_> {
    fn from_iter<I: IntoIterator<Item = Self>>(iter: I) -> Self {
        Self::Array(iter.into_iter().collect())
    }
}

#[cfg(test)]
mod tests {
    use alloc::string::ToString as _;
    use alloc::vec;

    use super::*;

    impl HashKey<'static> {
        fn from_str_key(s: &'static str) -> Self { Self::String(Cow::Borrowed(s)) }
    }

    fn key(obj: &Object<'_>) -> HashKey<'static> {
        match HashKey::try_from(obj).expect("hashable") {
            HashKey::Integer(n) => HashKey::Integer(n),
            HashKey::Boolean(b) => HashKey::Boolean(b),
            HashKey::String(s) => HashKey::String(Cow::Owned(s.into_owned())),
        }
    }

    // The book's TestStringHashKey.
    #[test]
    fn string_hash_key() {
        let hello1 = Object::from("Hello World");
        let hello2 = Object::from("Hello World");
        let diff1 = Object::from("My name is johnny");
        let diff2 = Object::from("My name is johnny");

        assert_eq!(key(&hello1), key(&hello2), "same content, different keys");
        assert_eq!(key(&diff1), key(&diff2), "same content, different keys");
        assert_ne!(key(&hello1), key(&diff1), "different content, same keys");
    }

    #[test]
    fn borrowed_and_owned_strings_make_the_same_key() {
        let borrowed = Object::from("abc");
        let owned = Object::from(String::from("abc"));
        assert_eq!(key(&borrowed), key(&owned));
    }

    #[test]
    fn integer_and_boolean_keys() {
        assert_eq!(key(&Object::Integer(1)), key(&Object::Integer(1)));
        assert_ne!(key(&Object::Integer(1)), key(&Object::Integer(2)));
        assert_eq!(key(&Object::Boolean(true)), key(&Object::Boolean(true)));
        assert_ne!(key(&Object::Boolean(true)), key(&Object::Boolean(false)));
    }

    #[test]
    fn keys_of_different_types_never_collide() {
        assert_ne!(key(&Object::Integer(1)), key(&Object::Boolean(true)));
        assert_ne!(key(&Object::Integer(1)), key(&Object::from("1")));
        assert_ne!(key(&Object::Boolean(true)), key(&Object::from("true")));
    }

    #[test]
    fn only_integers_booleans_and_strings_are_hashable() {
        for (obj, ty) in [
            (Object::Null, "NULL"),
            (Object::from(vec![]), "ARRAY"),
            (Object::from(Map::new()), "HASH"),
        ] {
            assert_eq!(HashKey::try_from(&obj), Err(EvalError::UnusableAsHashKey(ty)));
        }
    }

    #[test]
    fn hash_keys_round_trip_into_objects() {
        for obj in [Object::Integer(7), Object::Boolean(false), Object::from("k")] {
            assert_eq!(Object::from(key(&obj)), obj);
        }
    }

    #[test]
    fn cloning_arrays_and_hashes_shares_the_elements() {
        let array = Object::from(vec![Object::Integer(1), Object::Integer(2)]);
        let (Object::Array(a), Object::Array(b)) = (&array, &array.clone()) else {
            panic!("expected arrays");
        };
        assert!(Rc::ptr_eq(a, b));

        let hash = Object::from(Map::from([(HashKey::Integer(1), Object::Null)]));
        let (Object::Hash(a), Object::Hash(b)) = (&hash, &hash.clone()) else {
            panic!("expected hashes");
        };
        assert!(Rc::ptr_eq(a, b));
    }

    #[test]
    fn only_null_and_false_are_falsy() {
        let falsy = [Object::Null, Object::Boolean(false)];
        let truthy = [
            Object::Boolean(true),
            Object::Integer(0),
            Object::from(""),
            Object::from(vec![]),
            Object::from(Map::new()),
        ];
        assert!(falsy.iter().all(|o| !o.is_truthy()));
        assert!(truthy.iter().all(Object::is_truthy));
    }

    #[test]
    fn inspect_output() {
        let map = Map::from([
            (HashKey::from_str_key("a"), Object::Integer(1)),
            (HashKey::Integer(2), Object::Null),
        ]);

        let tests = [
            (Object::Null, "null"),
            (Object::Integer(-3), "-3"),
            (Object::Boolean(true), "true"),
            (Object::from("hi"), "hi"),
            (Object::from(vec![]), "[]"),
            (Object::from(vec![Object::Integer(1), Object::from("x")]), "[1, x]"),
            (Object::from(Map::new()), "{}"),
            (Object::from(map), "{2: null, a: 1}"),
        ];
        for (obj, want) in tests {
            assert_eq!(obj.to_string(), want);
        }
    }

    #[test]
    fn type_names_match_their_respective_objects() {
        let tests = [
            (Object::Null, "NULL"),
            (Object::Integer(1), "INTEGER"),
            (Object::Boolean(true), "BOOLEAN"),
            (Object::from("s"), "STRING"),
            (Object::from(vec![]), "ARRAY"),
            (Object::from(Map::new()), "HASH"),
        ];
        for (obj, want) in tests {
            assert_eq!(obj.type_name(), want);
        }
    }
}
