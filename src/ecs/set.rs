//! Sets of component data.

use std::marker::PhantomData;
use std::mem;
use std::sync::{Mutex, MutexGuard};

use super::*;

trait IsSame {
    fn is_same() -> bool { false }
}

impl<A, B> IsSame for (A, B) {}
impl<A> IsSame for (A, A) {
    #[inline]
    fn is_same() -> bool { true }
}

#[inline]
fn same<A, B>() -> bool {
    <(A, B) as IsSame>::is_same()
}

/// The base case of a recursive struct.
pub struct Empty;

/// A set of component storage data structures.
/// 
/// This is implemented as a recursive-variadic
/// data structure, which will allow for instant access of
/// component storage. The major downside is that attempted access
/// of components not in the set will resolve to a panic at runtime
/// rather than a compile error.
pub trait Set: Sized + Sync {
    fn push<T: Component>(self) -> SetEntry<T, Self>
    where T::Storage: Default {
        self.push_custom(Default::default())
    }
    
    fn push_custom<T: Component>(self, storage: T::Storage) -> SetEntry<T, Self> {
        SetEntry {
            data: Mutex::new(storage),
            parent: self,
            _marker: PhantomData,
        }
    }
    
    /// Lock a subset of this set.
    fn acquire_locks<'a, G: LockGroup<'a>>(&'a self) -> G::Locks {
        G::lock(self)
    }
    
    /// Get exclusive access to the storage for the given component by
    /// locking a mutex.
    fn lock_storage<T: Component>(&self) -> MutexGuard<T::Storage>;
    
    /// Get exclusive access to the storage for the given component by
    /// accessing it through a mutable reference.
    fn get_storage_mut<T: Component>(&mut self) -> &mut T::Storage;
}

/// An entry in a set.
pub struct SetEntry<T: Component, P: Set> {
    data: Mutex<T::Storage>,
    parent: P,
    _marker: PhantomData<T>,
}

impl Set for Empty {
    fn lock_storage<T: Component>(&self) -> MutexGuard<T::Storage> {
        panic!("Attempted access of component not in set.");
    }
    
    fn get_storage_mut<T: Component>(&mut self) -> &mut T::Storage {
        panic!("Attempted access of component not in set.");
    }
}

impl<T: Component, P: Set> Set for SetEntry<T, P> {
    fn lock_storage<C: Component>(&self) -> MutexGuard<C::Storage> {
        if same::<T, C>() {
            unsafe { mem::transmute(self.data.lock().unwrap()) }
        } else {
            self.parent.lock_storage::<C>()
        }
    }
    
    fn get_storage_mut<C: Component>(&mut self) -> &mut C::Storage {
        if same::<T, C>() {
            unsafe { mem::transmute(self.data.get_mut().unwrap()) }
        } else {
            self.parent.get_storage_mut::<C>()
        }
    }
}

/// Convenience trait for extending tuples of locks.
pub trait PushLock<'a, T: Component> {
    type Output: 'a;
    
    fn push(self, MutexGuard<'a, T::Storage>) -> Self::Output;
}

macro_rules! push_impl {
    () => {
        impl<'a, T: Component> PushLock<'a, T> for () {
            type Output = (MutexGuard<'a, T::Storage>,);
            
            fn push(self, lock: MutexGuard<'a, T::Storage>) -> Self::Output {
                (lock,)
            }
        }
    };
    
    ($f_id:ident $($id: ident)*) => {
        impl<'a,
            $f_id: Component, $($id: Component,)*
            COMP: Component
        > PushLock<'a, COMP> for (MutexGuard<'a, $f_id::Storage>, $(MutexGuard<'a, $id::Storage>,)*) {
            type Output = (MutexGuard<'a, $f_id::Storage>, $(MutexGuard<'a, $id::Storage>,)* MutexGuard<'a, COMP::Storage>,);
            
            fn push(self, lock: MutexGuard<'a, COMP::Storage>) -> Self::Output {
                let ($f_id, $($id,)*) = self;
                ($f_id, $($id,)*, lock)
            }    
        }
        
        push_impl!($($id)*);  
    };
}

/// A group of components to lock.
pub trait LockGroup<'a> {
    type Locks: 'a;
    
    /// Given a set, acquire the locks.
    fn lock<S: Set>(set: &'a S) -> Self::Locks;
}

macro_rules! group_impl {
    ($f_id: ident $($id: ident)*) => {
        impl<'a, $f_id: Component, $($id: Component,)*>
        LockGroup<'a> for ($f_id, $($id,)*) {
            type Locks = (MutexGuard<'a, $f_id::Storage>, $(MutexGuard<'a, $id::Storage>,)*);
                
            fn lock<SET: Set>(set: &'a SET) -> Self::Locks {
                (set.lock_storage::<$f_id>(), $(set.lock_storage::<$id>(),)*)
            }
        }
        
        group_impl!($($id)*);
    };
    
    () => {
        impl<'a> LockGroup<'a> for () {
            type Locks = ();
            
            fn lock<S: Set>(_: &'a S) -> () { () }
        }
    };
}

group_impl!(A B C D E F G H I J K);