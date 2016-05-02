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
    fn lock_subset<'a, G: LockGroup<'a>>(&'a self) -> G::Subset {
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

/// A locked subset of a set.
///
/// This is really similar to a `Set`, but the mutexes for each of the
/// components within have already been obtained. Also, the accessor methods
/// here return `None` rather than panicking on access, since they are more likely
/// to be queried with a non-contained component than a `Set` which will encompass
/// all components.
pub trait LockedSubset: Sized {
    fn push<T: Component>(self, guard: MutexGuard<T::Storage>) -> SubsetEntry<T, Self> {
        SubsetEntry {
            data: guard,
            parent: self,
            _marker: PhantomData,
        }
    }
    
    /// Get a reference to the storage container for the supplied component.
    /// Fails if this subset hasn't locked that component.
    fn get_storage<T: Component>(&self) -> Option<&T::Storage>;
    
    /// Get a mutable reference to the storage container for the supplied component.
    /// Fails if this subset hasn't locked that component.
    fn get_storage_mut<T: Component>(&mut self) -> Option<&mut T::Storage>;
}

/// An entry in a subset.
pub struct SubsetEntry<'a, T: 'a + Component, P: 'a + LockedSubset> {
    data: MutexGuard<'a, T::Storage>,
    parent: P,
    _marker: PhantomData<T>
}

impl LockedSubset for Empty {
    fn get_storage<T: Component>(&self) -> Option<&T::Storage> { None }
    fn get_storage_mut<T: Component>(&mut self) -> Option<&mut T::Storage> { None }
}

impl<'a, T: 'a + Component, P: 'a + LockedSubset> LockedSubset for SubsetEntry<'a, T, P> {
    fn get_storage<C: Component>(&self) -> Option<&C::Storage> {
        if same::<T, C>() {
            unsafe {
                Some(mem::transmute::<&T::Storage, &C::Storage>(&*self.data))
            }
        } else {
            self.parent.get_storage::<C>()
        }
    }
    
    fn get_storage_mut<C: Component>(&mut self) -> Option<&mut C::Storage> {
        if same::<T, C>() {
            unsafe {
                Some(mem::transmute::<&mut T::Storage, &mut C::Storage>(&mut *self.data))
            }
        } else {
            self.parent.get_storage_mut::<C>()
        }
    }
}

/// A group of components to lock.
pub trait LockGroup<'a> {
    type Subset: 'a + LockedSubset;
    
    /// Given a set, lock the subset.
    fn lock<S: Set>(set: &'a S) -> Self::Subset;
}

macro_rules! group_impl {
    ($f_id: ident $($id: ident)*) => {
        impl<'a, $f_id: Component, $($id: Component,)*>
        LockGroup<'a> for ($f_id, $($id,)*) {
            type Subset = SubsetEntry<'a,
                $f_id,
                <($($id,)*) as LockGroup<'a>>::Subset>;
                
            fn lock<SET: Set>(set: &'a SET) -> Self::Subset {
                let parent = <($($id,)*) as LockGroup<'a>>::lock(set);
                LockedSubset::push(parent, set.lock_storage::<$f_id>())
            }
        }
        
        group_impl!($($id)*);
    };
    
    () => {
        impl<'a> LockGroup<'a> for () {
            type Subset = Empty;
            
            fn lock<S: Set>(_: &'a S) -> Empty { Empty }
        }
    };
}

group_impl!(A B C D E F G H I J K);