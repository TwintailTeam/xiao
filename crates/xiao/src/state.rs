use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::{LazyLock, RwLock};

type Managed = &'static (dyn Any + Send + Sync);

static STATES: LazyLock<RwLock<HashMap<TypeId, Managed>>> = LazyLock::new(|| RwLock::new(HashMap::new()));

/// A managed value, returned by [`AppHandle::state`](crate::AppHandle::state).
/// Dereferences to `T`.
pub struct State<'r, T: Send + Sync + 'static>(&'r T);

impl<'r, T: Send + Sync + 'static> State<'r, T> {
    /// The underlying reference, with its full lifetime.
    pub fn inner(&self) -> &'r T { self.0 }
}

impl<T: Send + Sync + 'static> std::ops::Deref for State<'_, T> {
    type Target = T;

    fn deref(&self) -> &T { self.0 }
}

impl<T: Send + Sync + 'static> Clone for State<'_, T> {
    fn clone(&self) -> Self { *self }
}

impl<T: Send + Sync + 'static> Copy for State<'_, T> {}

impl<T: Send + Sync + std::fmt::Debug + 'static> std::fmt::Debug for State<'_, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.debug_tuple("State").field(self.0).finish() }
}

impl<T: Send + Sync + std::fmt::Display + 'static> std::fmt::Display for State<'_, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { self.0.fmt(f) }
}

impl<T: Send + Sync + PartialEq + 'static> PartialEq for State<'_, T> {
    fn eq(&self, other: &Self) -> bool { self.0 == other.0 }
}

pub(crate) fn manage<T: Send + Sync + 'static>(value: T) -> bool {
    let mut states = STATES.write().unwrap();
    if states.contains_key(&TypeId::of::<T>()) {
        return false;
    }
    states.insert(TypeId::of::<T>(), Box::leak(Box::new(value)));
    true
}

pub(crate) fn try_state<T: Send + Sync + 'static>() -> Option<State<'static, T>> {
    let states = STATES.read().unwrap();
    let managed: Managed = *states.get(&TypeId::of::<T>())?;
    managed.downcast_ref::<T>().map(State)
}

pub(crate) fn state<T: Send + Sync + 'static>() -> State<'static, T> {
    try_state::<T>().unwrap_or_else(|| panic!("state() called before manage() for {}", std::any::type_name::<T>()))
}
