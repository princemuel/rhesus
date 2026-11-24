use alloc::rc::Rc;
use core::cell::RefCell;
use core::fmt;

use crate::object::{Map, Object};

#[derive(Default)]
struct Scope<'a> {
    store: Map<&'a str, Object<'a>>,
    outer: Option<Environment<'a>>,
}

#[derive(Clone, Default)]
pub struct Environment<'a>(Rc<RefCell<Scope<'a>>>);

impl<'a> Environment<'a> {
    #[must_use]
    pub fn new() -> Self { Self::default() }

    #[must_use]
    pub fn new_enclosed(outer: &Self) -> Self {
        Self(Rc::new(RefCell::new(Scope { store: Map::new(), outer: Some(outer.clone()) })))
    }

    /// Look inside this scope, then walk outward.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<Object<'a>> {
        let scope = self.0.borrow();
        scope.store.get(name).cloned().or_else(|| scope.outer.as_ref()?.get(name))
    }

    pub fn set(&self, name: &'a str, value: Object<'a>) {
        drop(self.0.borrow_mut().store.insert(name, value));
    }
}

// The debug trait was intentionally not derived;
// since a function's env can contain that same function, printing it would
// recurse forever.
impl fmt::Debug for Environment<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Environment").finish_non_exhaustive()
    }
}
