use std::marker::PhantomData;

use super::*;
use super::set::{Set, LockedSubset};

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
pub struct Has<T: Component> {
    _marker: PhantomData<T>,
}

impl<T: Component> Filter for Has<T> {
    type Component = T;
    
    fn pred(&self, storage: &T::Storage, e: VerifiedEntity) -> bool {
        storage.has(e)
    }
}

/// A collection of filters.
///
/// This technically can be user-implemented, but this whole section
/// of the API is so intertwined and complex that it's probably best not to.
/// Re-implementation of Pipeline will require implementations of `PipelineFactory`
/// and `Push`.
pub trait Pipeline<'a>: Sized {
    type Item: 'a;
    
    /// Consume self along with handles to ECS state to pass all entities
    /// fulfilling the pipeline's predicates to the functions along with
    /// relevant component data. This will output a vector of the returned
    /// outputs from the function.
    fn for_each<F, U: Send, S: LockedSubset>(self, &S, &EntityManager, F) -> Vec<U>
    where F: Sync + for <'b> Fn(VerifiedEntity, <Self as Pipeline<'b>>::Item) -> U;
}

/// Convenience trait for extending tuples of filters.
pub trait Push<T> {
    type Output;
    
    fn push(self, T) -> Self::Output;
}

/// For creating pipelines.
///
/// This is how we transform tuples of component
/// types into tuples of "Has" filters. This can be implemented
/// by the user, but it won't integrate with the Query.
pub trait PipelineFactory {
    type Pipeline: for<'a> Pipeline<'a>;
    
    fn create() -> Self::Pipeline;
}

/// A query is a collection of filters coupled with handles
/// to the state of the ECS.
pub struct Query<'a, S: Set + 'a, P: 'a> {
    set: &'a S,
    entities: &'a EntityManager,
    pipeline: P,
}

impl<'a, S: 'a + Set, P: 'a + Pipeline<'a>> Query<'a, S, P> {
    /// Create a new query. Use of `WorldHandle::query()` is advised
    /// over this.
    pub fn new(s: &'a S, entities: &'a EntityManager, pipeline: P) -> Self {
        Query {
            set: s,
            entities: entities,
            pipeline: pipeline,
        }
    }
    
    /// Add another component to the query. When "for_each" is called,
    /// this will filter out all entities without this component.
    ///
    /// Adding a component more than once may lead to deadlock or panic.
    #[inline]
    pub fn with<T: Component>(self) -> Query<'a, S, <P as Push<Has<T>>>::Output>
    where P: Push<Has<T>>, <P as Push<Has<T>>>::Output: Pipeline<'a> {
        self.with_filtered(Has { _marker: PhantomData })
    }
    
    /// Add a component to the query to be specially filtered. This is useful for those
    /// cases where components are stored in a special data structure.
    ///
    /// Adding a component more than once may lead to deadlock or panic,
    #[inline]
    pub fn with_filtered<T: Filter>(self, filter: T) -> Query<'a, S, <P as Push<T>>::Output>
    where P: Push<T>, <P as Push<T>>::Output: Pipeline<'a> {
        Query {
            set: self.set,
            entities: self.entities,
            pipeline: self.pipeline.push(filter)
        }
    }
    
    /// Perform an action for each entity which fits the properties of 
    /// the filter.
    pub fn for_each<F, U: Send>(self, f: F) -> Vec<U>
    where F: Sync + for<'b> Fn(VerifiedEntity, <P as Pipeline<'b>>::Item) -> U {
        // TODO: amend Pipeline so that we can get a LockGroup to pass to lock_subset.
        // TODO: have for_each return the locked subset along with the items.
        let empty = ::ecs::set::Empty;
        self.pipeline.for_each(&empty, self.entities, f)
    }
}

// implementations for tuples.

macro_rules! as_expr {
    ($e: expr) => { $e }
}

macro_rules! access {
    ($e: expr; $id: tt) => { as_expr!($e.$id) }
}

macro_rules! push_impl {
    ($($id: ident $num: tt)*) => {
        impl<$($id: Filter,)* Last: Filter> Push<Last> for ($($id,)*) {
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
        impl<$($id: Component,)*> PipelineFactory for ($($id,)*) {
            type Pipeline = ($(Has<$id>,)*);
            
            fn create() -> Self::Pipeline {
                ($(Has::<$id> { _marker: PhantomData }, )*)
            }
        }
    };
}

// factory!(A B C D E F);
// factory!(A B C D E);
// factory!(A B C D);
// factory!(A B C);
factory!(A B);
factory!(A);
factory!();

// filter extension trait used in pipeline implementation.
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

macro_rules! pipeline_impl {
    () => {
        impl<'a> Pipeline<'a> for () {
            type Item = ();
            
            fn for_each<F, U: Send, S: LockedSubset>(self, _: &S, _: &EntityManager, _: F) -> Vec<U>
            where F: 'a + Sync + for<'b> Fn(VerifiedEntity, <Self as Pipeline<'b>>::Item) -> U {
                Vec::new()
            }
        }
    };
    
    ($f_id: ident $f_num: tt $($id: ident $num: tt)*) => {
        impl<'a, $f_id: Filter, $($id: Filter,)*> Pipeline<'a> for
        ($f_id, $($id,)*) {
            type Item = (&'a <$f_id as Filter>::Component, $(&'a <$id as Filter>::Component,)*);
            
            #[allow(unused_mut)]
            fn for_each<OP, U: Send, SET: LockedSubset>(self, set: &SET, entities: &EntityManager, f: OP) -> Vec<U>
            where OP: 'a + Sync + for<'b> Fn(VerifiedEntity, <Self as Pipeline<'b>>::Item) -> U {  
                // it's ok to unwrap the calls to get_storage() since this function is called with a subset
                // that has been locked with this pipeline in mind.
                              
                // the first filter is special-cased -- we use the "all" method of FilterExt here
                // to get a vector which will get whittled down.
                let mut entities = access!(self; $f_num).all(set.get_storage::<$f_id::Component>().unwrap(), entities);
                
                // apply the "filter" method of FilterExt to the vector in turn.
                $(
                    access!(self; $num).filter(set.get_storage::<$id::Component>().unwrap(), &mut entities);
                )*
                
                // for each entry that is still Some (that is, the entity within passes all filters)
                entities.into_iter().filter_map(|e| e).map(|e| {
                    // get the data by looking into the storage containers,
                    let data = (
                        set.get_storage::<$f_id::Component>().unwrap().get(e).unwrap(),
                        $(set.get_storage::<$id::Component>().unwrap().get(e).unwrap(),)*
                    );
                    
                    // and call the function provided, collecting the outputs
                    // for a "write" phase.
                    f(e, data)
                }).collect()
            }
        } 
    };
}

// pipeline_impl!(A 0 B 1 C 2 D 3 E 4 F 5);
// pipeline_impl!(A 0 B 1 C 2 D 3 E 4);
// pipeline_impl!(A 0 B 1 C 2 D 3);
// pipeline_impl!(A 0 B 1 C 2);
pipeline_impl!(A 0 B 1);
pipeline_impl!(A 0);
pipeline_impl!();