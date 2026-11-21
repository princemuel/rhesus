use alloc::borrow::Cow;
use alloc::collections::BTreeMap;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

use crate::ast::Block;
use crate::environment::Environment;
use crate::errors::EvalError;
use crate::host::Host;

pub type Map<K, V> = BTreeMap<K, V>;

pub type BuiltinResult<'a> = Result<Object<'a>, EvalError<'a>>;

/// A native function. It gets the arguments and the REPL's output sink (for
/// `puts`).
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
    pub params: Vec<&'a str>,
    pub body: Block<'a>,
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
    Array(Vec<Self>),
    Hash(Map<HashKey<'a>, Self>),
    Function(Rc<Function<'a>>),
    Builtin(BuiltinFn<'a>),
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
        }
    }

    /// only `false` and `null` are not truthy
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
            (Self::Function(a), Self::Function(b)) => Rc::ptr_eq(a, b),
            (Self::Builtin(a), Self::Builtin(b)) => core::ptr::fn_addr_eq(*a, *b),
            _ => false,
        }
    }
}

/// The book's `Inspect()`.
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

                    for item in iter {
                        write!(f, ", {item}")?;
                    }
                }
                f.write_str("]")
            }
            Self::Hash(map) => {
                f.write_str("{")?;

                let mut iter = map.iter();
                if let Some((k, v)) = iter.next() {
                    write!(f, "{k}: {v}")?;

                    for (k, v) in iter {
                        write!(f, ", {k}: {v}")?;
                    }
                }
                f.write_str("}")
            }
            Self::Function(func) => {
                write!(f, "fn({}) {{\n{}\n}}", func.params.join(", "), func.body)
            }
            Self::Builtin(_) => f.write_str("builtin function"),
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
    fn from(value: Vec<Self>) -> Self { Self::Array(value) }
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
    fn from(value: Map<HashKey<'a>, Self>) -> Self { Self::Hash(value) }
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
