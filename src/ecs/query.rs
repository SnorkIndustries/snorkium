//! Queries and filters.

use std::marker::PhantomData;
use std::sync::RwLock;

use super::*;
use super::set::LockGroup;

/// Filters are used to test properties of entities' data.
///
/// The most common kind of filter is to test whether an entity has a specific
/// component. This is implemented with the `Has` filter. All other filters must
/// be sub-filters of `Has`.
/// Queries are composed of multiple filters, which each entity will be tested
/// against in turn.
pub trait Filter {
    type Component: Component;
    
    /// The predicate for entities to fulfill.
    /// 
    /// This may only return true if the entity has the given component.
    fn pred(&self, &<Self::Component as Component>::Storage, VerifiedEntity) -> bool;
}

/// A filter which tests whether an entity has a specific component.
/// 
/// These are automatically created from the implementation of `FilterFactory`
/// for tuples of components. See `WorldHandle::query()` for more details.
pub struct Has<T: Component> {
    _marker: PhantomData<T>,
}

impl<T: Component> Filter for Has<T> {
    type Component = T;
    
    fn pred(&self, storage: &T::Storage, e: VerifiedEntity) -> bool {
        storage.has(e)
    }
}

/// Convenience trait for extending tuples of filters.
pub trait PushFilter<T> {
    type Output;
    
    fn push(self, T) -> Self::Output;
}

/// For creating filter lists.
///
/// This isn't used directly, but instead in the implementation
/// of `Query`.
/// This is how we transform tuples of component
/// types into tuples of "Has" filters. This can be implemented
/// by the user, but it won't integrate with queries.
///
/// In practice this is a tuple of components, like
/// `(A, B, C)` or `(A, B, C, D, E)`. This will map
/// to a tuple of "Has" filters, one for each component type given.
///
/// This tuple can then have other filters "pushed" on top of it to
/// make a larger tuple.
pub trait FilterFactory {
    type Filters: for<'a> FilterGroup<'a>;
    
    fn create() -> Self::Filters;
}

/// A group of filters.
///
/// This is implemented for all tuples of filters.
pub trait FilterGroup<'a> {
    type Locks: 'a;
    
    /// Acquire the necessary locks, find all entities which fulfill these filters,
    /// and return both.
    fn filter_acquire<S: 'a + Set>(self, &'a S, &EntityManager) -> (Vec<Entity>, Self::Locks);
}

/// This stores some handles to the world data as well as a group of filters.
///
/// You can consume a query to return a vector of entities matching the filters
/// as well as the locks for each queried component's data.
pub struct Query<'a, S: 'a + Set, F> {
    entities: &'a RwLock<EntityManager>,
    data: &'a S,
    filters: F,
}

impl<'a, S: 'a + Set, F> Query<'a, S, F> {
    /// Create a new query.
    ///
    /// Use of `WorldHandle::make_query()` is advised over this, although they are
    /// functionally equivalent.
    pub fn new(wh: &WorldHandle<'a, S>, filters: F) -> Query<'a, S, F> {
        Query {
            entities: wh.entities,
            data: wh.data,
            filters: filters,
        }    
    }
    
    /// Add a component to this query. When executed, this query will only return entities which
    /// have this component.
    pub fn with<T: Component>(self) -> Query<'a, S, <F as PushFilter<Has<T>>>::Output>
    where F: PushFilter<Has<T>> {
        self.with_filtered(Has { _marker: PhantomData })
    }
    
    /// Add a custom filter to this query. When executed, this query will only return entities which
    /// pass the filter.
    pub fn with_filtered<T: Filter>(self, f: T) -> Query<'a, S, <F as PushFilter<T>>::Output>
    where F: PushFilter<T> {
        Query {
            entities: self.entities,
            data: self.data,
            filters: self.filters.push(f),
        }
    }
    
    /// Execute this query. This will return 
    pub fn execute(self) -> (Vec<Entity>, F::Locks) where F: FilterGroup<'a> {
        let entities = self.entities.read().unwrap();
        self.filters.filter_acquire(self.data, &entities)
    }
}

// implementations for tuples.

macro_rules! as_expr {
    ($e: expr) => { $e }
}

// field access macro.
macro_rules! access {
    ($e: expr; $id: tt) => { as_expr!($e.$id) }
}

macro_rules! push_impl {
    ($($id: ident $num: tt)*) => {
        impl<$($id: Filter,)* Last: Filter> PushFilter<Last> for ($($id,)*) {
            type Output = ($($id,)* Last,);
            
            fn push(self, last: Last) -> Self::Output {
                ($(access!(self; $num),)* last,)
            }
        }
    }
}

push_impl!(A 0 B 1 C 2 D 3 E 4 F 5);
push_impl!(A 0 B 1 C 2 D 3 E 4);
push_impl!(A 0 B 1 C 2 D 3);
push_impl!(A 0 B 1 C 2);
push_impl!(A 0 B 1);
push_impl!(A 0);
push_impl!();

macro_rules! factory {
    ($($id: ident)*) => {
        impl<$($id: Component,)*> FilterFactory for ($($id,)*) {
            type Filters = ($(Has<$id>,)*);
            
            fn create() -> Self::Filters {
                ($(Has::<$id> { _marker: PhantomData }, )*)
            }
        }
    };
}

factory!(A B C D E F);
factory!(A B C D E);
factory!(A B C D);
factory!(A B C);
factory!(A B);
factory!(A);
factory!();

// filter extension trait used in filter group implementation.
trait FilterExt: Filter {
    // return a vector of all living entities which fulfill the predicate
    // in the form Some(e);
    fn all<'a>(&'a self, &<Self::Component as Component>::Storage, &'a EntityManager)
    -> Vec<Option<VerifiedEntity>>;
    
    // given the storage and a vector of Option<VerifiedEntity>, set those which do not fulfill
    // the predicate to None.
    fn filter(&self, &<Self::Component as Component>::Storage, &mut Vec<Option<VerifiedEntity>>);
}

impl<F: Filter> FilterExt for F {
    fn all<'a>(&'a self, storage: &<Self::Component as Component>::Storage, em: &'a EntityManager)
    -> Vec<Option<VerifiedEntity>> {
        storage.entities()
            .filter_map(|e| em.verify(e))
            .filter(|e| self.pred(storage, *e))
            .map(Some)
            .collect()
    }
    
    fn filter(&self, storage: &<Self::Component as Component>::Storage, 
              entities: &mut Vec<Option<VerifiedEntity>>) {
        for i in entities {
            if i.is_none() { continue; }
            
            let e = *i.as_ref().unwrap();
            if !self.pred(storage, e) { *i = None }
        }
    }
}

impl<'a> FilterGroup<'a> for () {
    type Locks = ();
    
    fn filter_acquire<S: 'a + Set>(self, _: &'a S, _: &EntityManager) -> (Vec<Entity>, ()) {
        (Vec::new(), ())
    }
}

macro_rules! group_impl {
    ($f_id: ident $f_num: tt $($id: ident $num: tt)*) => {
        
        impl<'a, $f_id: Filter, $($id: Filter,)*> FilterGroup<'a> for ($f_id, $($id,)*) {
            type Locks = <($f_id::Component, $($id::Component,)*) as LockGroup<'a>>::Locks;
            
            #[allow(unused_mut)]
            fn filter_acquire<S: 'a + Set>(self, set: &'a S, entities: &EntityManager)
            -> (Vec<Entity>, Self::Locks) {
                let locks = set.acquire_locks::<($f_id::Component, $($id::Component,)*)>();
                
                let mut es = access!(self; $f_num).all(&access!(locks; $f_num), entities);
                
                $(
                    access!(self; $num).filter(&access!(locks; $num), &mut es);  
                )*
                
                let es = es.into_iter().filter_map(|x| x.map(|v| v.entity())).collect();
                
                (es, locks)
            }
        }
    };
}

group_impl!(A 0 B 1 C 2 D 3 E 4 F 5);
group_impl!(A 0 B 1 C 2 D 3 E 4);
group_impl!(A 0 B 1 C 2 D 3);
group_impl!(A 0 B 1 C 2);
group_impl!(A 0 B 1);
group_impl!(A 0);