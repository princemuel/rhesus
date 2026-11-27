use alloc::rc::{Rc, Weak};
use alloc::vec::Vec;
use core::cell::RefCell;
use core::{fmt, mem};

use crate::object::{Map, Object};

#[derive(Default)]
struct Scope<'a> {
    store: Store<'a>,
    outer: Outer<'a>,
}

/// How a scope reaches the scope around it.
#[derive(Default)]
enum Outer<'a> {
    /// End of the chain: a global scope.
    #[default]
    Root,
    /// Keeps the enclosing scope alive (a call's scope, a closure's captured
    /// scope).
    Strong(Environment<'a>),
    /// Sees the enclosing scope without owning it. See
    /// [`Environment::viewing`].
    Weak(Weak<RefCell<Scope<'a>>>),
}

#[derive(Clone, Default)]
pub struct Environment<'a>(Rc<RefCell<Scope<'a>>>);

impl<'a> Environment<'a> {
    #[must_use]
    pub fn new() -> Self { Self::default() }

    #[must_use]
    pub fn new_enclosed(outer: &Self) -> Self {
        Self(Rc::new(RefCell::new(Scope {
            store: Store::Local(Vec::new()),
            outer: Outer::Strong(outer.clone()),
        })))
    }

    /// A fresh, empty scope that sees `root`, including definitions made in it
    /// later, without keeping it alive.
    #[must_use]
    pub fn viewing(root: &Self) -> Self {
        Self(Rc::new(RefCell::new(Scope {
            store: Store::Local(Vec::new()),
            outer: Outer::Weak(Rc::downgrade(&root.0)),
        })))
    }

    #[must_use]
    pub fn is_root(&self) -> bool { matches!(self.0.borrow().outer, Outer::Root) }

    #[must_use]
    pub fn ptr_eq(&self, other: &Self) -> bool { Rc::ptr_eq(&self.0, &other.0) }

    /// For *local* scopes only: moves existing bindings into a new outer parent
    /// while keeping `self`'s Rc identity, so a function stored in `self` can
    /// capture the parent without capturing itself.
    #[must_use]
    pub fn split_for_closure(&self) -> Self {
        let mut inner = self.0.borrow_mut();

        debug_assert!(!matches!(inner.outer, Outer::Root), "cannot split root scope");

        let store = mem::replace(&mut inner.store, Store::Local(Vec::new()));
        let outer = mem::take(&mut inner.outer);

        let parent = Self(Rc::new(RefCell::new(Scope { store, outer })));
        inner.outer = Outer::Strong(parent.clone());
        parent
    }

    /// Look inside this scope, then walk outward.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<Object<'a>> {
        let scope = self.0.borrow();
        scope.store.get(name).or_else(|| match &scope.outer {
            Outer::Root => None,
            Outer::Strong(env) => env.get(name),
            Outer::Weak(weak) => weak.upgrade().and_then(|rc| Self(rc).get(name)),
        })
    }

    pub fn set(&self, name: &'a str, value: Object<'a>) {
        self.0.borrow_mut().store.set(name, value);
    }
}

// The debug trait was intentionally not derived;
// since a function's env could contain that same function, printing it would
// probably recurse forever.
impl fmt::Debug for Environment<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Environment").finish_non_exhaustive()
    }
}

enum Store<'a> {
    Global(Map<&'a str, Object<'a>>),
    Local(Vec<(&'a str, Object<'a>)>),
}

impl Default for Store<'_> {
    fn default() -> Self { Self::Global(Map::new()) }
}

impl<'a> Store<'a> {
    fn get(&self, name: &str) -> Option<Object<'a>> {
        match self {
            Self::Global(map) => map.get(name).cloned(),
            Self::Local(vec) => vec.iter().find(|(k, _)| *k == name).map(|(_, v)| v.clone()),
        }
    }

    fn set(&mut self, name: &'a str, value: Object<'a>) {
        match self {
            Self::Global(map) => {
                drop(map.insert(name, value));
            }
            Self::Local(vec) => match vec.iter_mut().find(|(k, _)| *k == name) {
                Some(slot) => slot.1 = value,
                None => vec.push((name, value)),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::Block;
    use crate::object::Function;

    #[test]
    fn a_viewing_scope_sees_definitions_made_after_it() {
        let root = Environment::new();
        let view = Environment::viewing(&root);
        assert_eq!(view.get("x"), None);

        root.set("x", Object::Integer(1));
        assert_eq!(view.get("x"), Some(Object::Integer(1)));

        drop(root);
        assert_eq!(view.get("x"), None, "a view must not keep its root alive");
    }

    #[test]
    fn a_function_stored_in_the_root_does_not_keep_the_root_alive() {
        let root = Environment::new();
        let func = Function {
            name: None,
            params: Rc::from([]),
            body: Rc::new(Block::default()),
            env: Environment::viewing(&root),
        };
        root.set("f", Object::from(func));

        let weak = Rc::downgrade(&root.0);
        drop(root);
        assert!(weak.upgrade().is_none(), "the root leaked through the function's scope");
    }
}
