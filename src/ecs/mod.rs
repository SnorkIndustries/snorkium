//! A multithreaded Entity Component System (ECS)

use std::any::{Any, TypeId};
use std::collections::VecDeque;
use std::marker::PhantomData;
use std::mem;

const ID_BITS: usize = 24;
const MIN_UNUSED: usize = 1024;

pub mod impls;

/// A component is a piece of raw data which is associated with an entity.
///
/// "Systems" will typically iterate over all entities with a specific set of components,
/// performing some action for each.
pub trait Component: Any + Copy + Sync {
    /// The data structure which stores component of this type.
    ///
    /// By default, this will be the `DefaultStorage` structure,
    /// which is good for almost all use-cases.
    /// However, for some components, it is more performant to store them
    /// in a special data structure with custom filters. A good example of this
    /// is positional data, which can be queried much more easily when stored
    /// in a quadtree or octree.
    type Storage: Storage<Self>;
}

impl<T: Any + Copy + Sync> Component for T {
    default type Storage = DefaultStorage<Self>;
}

/// Component data storage.
///
/// In general, this will be used through `DefaultStorage`, but some components
/// can be more conveniently accessed through special data structures.
pub trait Storage<T: Component>: Sync {
    /// Set the component data for an entity.
    fn set(&mut self, e: VerifiedEntity, data: T);
    
    /// Whether this entity has this component.
    fn has(&self, e: VerifiedEntity) -> bool;
    
    /// Get a reference to the component data for an entity.
    fn get(&self, e: VerifiedEntity) -> Option<&T>;
    
    /// Get a mutable reference to the component data for an entity.
    fn get_mut(&mut self, e: VerifiedEntity) -> Option<&mut T>;
    
    /// Remove an entity's data, returning it by value if it existed.
    fn remove(&mut self, e: VerifiedEntity) -> Option<T>;
    
    /// Destroy an entity's data without returning the data.
    ///
    /// This entity may not be alive.
    /// This will usually be called with entities that have been
    /// destroyed in a previous frame to have storage mappers clean
    /// up.
    fn destroy(&mut self, e: Entity);  
    
    /// Return an iterator over all entities this stores data for.
    fn entities<'a>(&'a self) -> Box<Iterator<Item=Entity> + 'a>;
}

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

#[derive(Clone, Copy)]
struct Index {
    off: usize,
    entity: Entity,
}

impl Index {
    fn new(off: usize, entity: Entity) -> Self {
        Index {
            off: off,
            entity: entity,
        }
    }
}

// Component data storage.
pub struct DefaultStorage<T: Component> {
    // data vector -- this is tightly packed.
    data: Vec<T>,
    // loosely packed lookup table mapping entity ids to data indices.
    indices: Vec<Option<Index>>,
    // unused indices in the data table.
    unused: VecDeque<usize>,
}

impl<T: Component> DefaultStorage<T> {
    fn new() -> Self {
        DefaultStorage {
            data: Vec::new(),
            indices: Vec::new(),
            unused: VecDeque::new(),
        }
    }
}

impl<T: Component> Storage<T> for DefaultStorage<T> {    
    /// Sets the component for the given entity.
    fn set(&mut self, e: VerifiedEntity, data: T) {
        let id = e.entity().id() as usize;
        while self.indices.len() < id as usize {
            self.indices.push(None);
        }
        
        if let Some(idx) = self.indices[id] {
            self.data[idx.off] = data;
            self.indices[id].unwrap().entity = e.entity();
        } else if let Some(off) = self.unused.pop_front() {
            self.data[off] = data;
            self.indices[id] = Some(Index::new(off, e.entity()));
        } else {
            self.data.push(data);
            self.indices[id] = Some(Index::new(self.data.len() - 1, e.entity()));
        }
    }
    
    fn has(&self, e: VerifiedEntity) -> bool {
        if let Some(&Some(idx)) = self.indices.get(e.entity().id() as usize) {
            idx.entity == e.entity()
        } else {
            false
        }
    }
    
    /// Get a reference to an entity's data.
    fn get(&self, e: VerifiedEntity) -> Option<&T> {
        match self.indices.get(e.entity().id() as usize) {
            Some(&Some(idx)) if idx.entity == e.entity() => {
                Some(&self.data[idx.off])
            }
            _ => None,
        }
    }
    
    /// Get a mutable reference to an entity's data.
    fn get_mut(&mut self, e: VerifiedEntity) -> Option<&mut T> {
        match self.indices.get(e.entity().id() as usize) {
            Some(&Some(idx)) if idx.entity == e.entity() => {
                Some(&mut self.data[idx.off])
            }
            _ => None,
        }
    }
    
    /// Remove an entity's data, returning it by value if it existed.
    fn remove(&mut self, e: VerifiedEntity) -> Option<T> {
        match self.indices.get(e.entity().id() as usize) {
            Some(&Some(idx)) => {
                self.indices[e.entity().id() as usize] = None;
                self.unused.push_back(idx.off);
                
                if idx.entity == e.entity() {
                    Some(self.data[idx.off])
                } else {
                    None
                }
            }       
            _ => None,
        }
    }
    
    fn destroy(&mut self, e: Entity) {
        if let Some(&Some(idx)) = self.indices.get(e.id() as usize) {
            self.indices[e.id() as usize] = None;
            self.unused.push_back(idx.off);
        }
    }
    
    fn entities<'a>(&'a self) -> Box<Iterator<Item=Entity> + 'a> {
        let iter = self.indices.iter().filter_map(|idx| {
            idx.as_ref().map(|inner| inner.entity)
        });
        
        Box::new(iter)
    }
}

impl<T: Component> Default for DefaultStorage<T> {
    fn default() -> Self {
        DefaultStorage::new()
    }
}

/// Manages creation and deletion of entities.
pub struct EntityManager {
    gens: Vec<u8>,
    unused: VecDeque<u32>,
}

impl EntityManager {
    /// Creates a new EntityManager
    pub fn new() -> Self {
        EntityManager {
            gens: Vec::new(),
            unused: VecDeque::new(),
        }    
    }
    
    /// Creates a new entity.
    pub fn next(&mut self) -> Entity {
        if self.unused.len() >= MIN_UNUSED {
            let id = self.unused.pop_front().unwrap();
            Entity::new(self.gens[id as usize], id)
        } else {
            self.gens.push(0);
            Entity::new(0, self.gens.len() as u32 - 1)
        }
    }
    
    /// Whether an entity is alive.
    pub fn is_alive(&self, entity: Entity) -> bool {
        self.gens[entity.id() as usize] == entity.gen()
    }
    
    /// Attempts to verify the entity given.
    ///
    /// What this does is check if the entity is alive, and if so,
    /// returns a "verified" handle to the entity. This verified handle
    /// borrows the `EntityManager` immutably, so the entity is guaranteed
    /// to stay alive as long as the `VerifiedEntity` sticks around.
    pub fn verify(&self, entity: Entity) -> Option<VerifiedEntity> {
        if self.is_alive(entity) {
            Some(VerifiedEntity {
                inner: entity,
                _marker: PhantomData,
            })
        } else {
            None
        }
    }  
     
    /// Destroys an entity. No-op if already dead.
    pub fn destroy(&mut self, entity: Entity) {
        if !self.is_alive(entity) { return; }
        
        self.gens[entity.id() as usize] += 1;
        self.unused.push_back(entity.id());
    }
}

/// An entity is a handle associated with a unique set of components.
///
/// An entity may be "dead", in which case it is no longer valid and will not
/// be useful. The only way to check this is with `EntityManager::is_alive`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Entity {
    id: u32
}

impl Entity {
    fn new(gen: u8, id: u32) -> Self {
        Entity {
            id: ((gen as u32) << ID_BITS) + id 
        }
    }
    
    fn gen(&self) -> u8 {
        (self.id >> ID_BITS) as u8
    }
    
    fn id(&self) -> u32 {
        self.id & ((1 << ID_BITS) - 1)
    }
}

/// A verified entity is an entity guaranteed to be alive.
/// 
/// See `EntityManager::verify()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VerifiedEntity<'a> {
    inner: Entity,
    _marker: PhantomData<&'a EntityManager>,
}

impl<'a> VerifiedEntity<'a> {
    /// Gets the entity handle.
    pub fn entity(&self) -> Entity {
        self.inner
    }
}

pub struct Empty;

pub struct Entry<T: Component, S: Storage<T>, P: Set> {
    data: S,
    parent: P,
    _marker: PhantomData<T>,
}

pub trait Set: Sized {
    fn push<T: Component>(self) -> Entry<T, T::Storage, Self>
    where T::Storage: Default {
        Entry {
            data: T::Storage::default(),
            parent: self,
            _marker: PhantomData,
        }
    }
    
    fn push_storage<T: Component, S: Storage<T>>(self, storage: S) -> Entry<T, S, Self> {
        Entry {
            data: storage,
            parent: self,
            _marker: PhantomData,
        }
    }
    
    
    fn storage<T: Component>(&self) -> &T::Storage;
    fn storage_mut<T: Component>(&mut self) -> &mut T::Storage;
}

impl Set for Empty {
    fn storage<T: Component>(&self) -> &T::Storage {
        panic!("Attempted access of component not in set.");
    }
    
    fn storage_mut<T: Component>(&mut self) -> &mut T::Storage {
        panic!("Attempted access of component not in set.");
    }
}

impl<T: Component, S: Storage<T>, P: Set> Set for Entry<T, S, P> {
    fn storage<C: Component>(&self) -> &C::Storage {
        if TypeId::of::<T>() == TypeId::of::<C>() {
            unsafe { mem::transmute(&self.data) }
        } else {
            self.parent.storage::<C>()
        }
    }
    
    fn storage_mut<C: Component>(&mut self) -> &mut C::Storage {
        if TypeId::of::<T>() == TypeId::of::<C>() {
            unsafe { mem::transmute(&mut self.data) }
        } else {
            self.parent.storage_mut::<C>()
        }
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
    /// relavant component data.
    fn for_each<F, S: Set>(self, &'a S, &'a EntityManager, F)
    where F: 'a + Sync + Fn(VerifiedEntity, Self::Item);
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
    pub fn for_each<F>(self, f: F)
    where F: 'a + Sync + Fn(VerifiedEntity, <P as Pipeline>::Item) {
        self.pipeline.for_each(self.set, self.entities, f);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn smoke() {
        let mut manager = EntityManager::new();
        
        let e1 = manager.next();
        let e2 = manager.next();
        let e3 = manager.next();
        
        assert!(manager.is_alive(e1));
        assert!(manager.is_alive(e2));
        assert!(manager.is_alive(e3));
        
        manager.destroy(e2);
        
        assert!(manager.is_alive(e1));
        assert!(!manager.is_alive(e2));
        assert!(manager.is_alive(e3));
        
        manager.destroy(e2);
        manager.destroy(e3);
        
        assert!(manager.is_alive(e1));
        assert!(!manager.is_alive(e2));
        assert!(!manager.is_alive(e3));
    }
}