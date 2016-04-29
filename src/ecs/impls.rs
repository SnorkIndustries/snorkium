//! Tedious trait implementations for the ECS.

use super::*;

use std::marker::PhantomData;

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

factory!(A B C D E F);
factory!(A B C D E);
factory!(A B C D);
factory!(A B C);
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
            
            fn for_each<F, S: Set>(self, _: &'a S, _: &'a EntityManager, _: F)
            where F: 'a + Sync + Fn(VerifiedEntity, Self::Item) {}
        }
    };
    
    ($f_id: ident $f_num: tt $($id: ident $num: tt)*) => {
        impl<'a, $f_id: Filter, $($id: Filter,)*> Pipeline<'a> for
        ($f_id, $($id,)*) {
            type Item = (&'a <$f_id as Filter>::Component, $(&'a <$id as Filter>::Component,)*);
            
            #[allow(unused_mut)]
            fn for_each<OP, SET: Set>(self, set: &'a SET, entities: &'a EntityManager, f: OP)
            where OP: 'a + Sync + Fn(VerifiedEntity, Self::Item) {
                // get a tuple of all the storage containers, one for each type.
                // in a multithreaded implementation, these are going to be MutexGuards.
                // it's important that we don't lock the mutexes more than once.
                let storages = (set.storage::<$f_id::Component>(), $(set.storage::<$id::Component>(),)*);
                
                // the first filter is special-cased -- we use the "all" method of FilterExt here
                // to get a vector which will get whittled down.
                let mut entities = access!(self; $f_num).all(access!(storages; $f_num), entities);
                
                // apply the "filter" method of FilterExt to the vector in turn.
                $(
                    access!(self; $num).filter(access!(storages; $num), &mut entities);
                )*
                
                // for each entry that is still Some (that is, the entity within passes all filters)
                for e in entities.into_iter().filter_map(|e| e) {
                    // get the data by looking into the storage containers,
                    let data = (
                        access!(storages; $f_num).get(e).unwrap(),
                        $(access!(storages; $num).get(e).unwrap(),)*
                    );
                    
                    // and call the function provided.
                    f(e, data);
                }
            }
        } 
    };
}

pipeline_impl!(A 0 B 1 C 2 D 3 E 4 F 5);
pipeline_impl!(A 0 B 1 C 2 D 3 E 4);
pipeline_impl!(A 0 B 1 C 2 D 3);
pipeline_impl!(A 0 B 1 C 2);
pipeline_impl!(A 0 B 1);
pipeline_impl!(A 0);
pipeline_impl!();