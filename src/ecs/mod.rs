//! A multithreaded Entity Component System (ECS)

use std::any::{Any, TypeId};
use std::collections::VecDeque;
use std::marker::PhantomData;
use std::mem;
use std::sync::{Mutex, MutexGuard};
use std::ops::Deref;

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

/// The default component data storage.
///
/// Data is stored contiguously and can be iterated
/// over very quickly.
pub struct DefaultStorage<T: Component> {
    // data vector -- this is tightly packed.
    data: Vec<(Entity, T)>,
    // loosely packed lookup table mapping entity ids to data indices.
    indices: Vec<Option<usize>>,
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
        
        let data = (e.entity(), data);
        
        if let Some(idx) = self.indices[id] {
            self.data[idx] = data;
        } else if let Some(idx) = self.unused.pop_front() {
            self.data[idx] = data;
            self.indices[id] = Some(idx);
        } else {
            self.data.push(data);
            self.indices[id] = Some(self.data.len());
        }
    }
    
    fn has(&self, e: VerifiedEntity) -> bool {
        match self.indices.get(e.entity().id() as usize) {
            Some(&Some(idx)) => {
                self.data[idx].0 == e.entity()
            }
            _ => false,
        }
    }
    
    /// Get a reference to an entity's data.
    fn get(&self, e: VerifiedEntity) -> Option<&T> {
        if let Some(&Some(idx)) = self.indices.get(e.entity().id() as usize) {
            if self.data[idx].0 == e.entity() {
                return Some(&self.data[idx].1)
            }
        }
        
        None
    }
    
    /// Get a mutable reference to an entity's data.
    fn get_mut(&mut self, e: VerifiedEntity) -> Option<&mut T> {
        if let Some(&Some(idx)) = self.indices.get(e.entity().id() as usize) {
            if self.data[idx].0 == e.entity() {
                return Some(&mut self.data[idx].1)
            }
        }
        
        None
    }
    
    /// Remove an entity's data, returning it by value if it existed.
    fn remove(&mut self, e: VerifiedEntity) -> Option<T> {
        let id = e.entity().id() as usize;
        if let Some(&Some(idx)) = self.indices.get(id) {
            self.unused.push_back(idx);
            self.indices[id] = None;
            
            if self.data[idx].0 == e.entity() {
                return Some(self.data[idx].1)
            }
        }
        
        None
    }
    
    fn destroy(&mut self, e: Entity) {
        if let Some(&Some(idx)) = self.indices.get(e.id() as usize) {
            self.indices[e.id() as usize] = None;
            self.unused.push_back(idx);
        }
    }
    
    fn entities<'a>(&'a self) -> Box<Iterator<Item=Entity> + 'a> {
        let iter = self.data.iter().map(|&(e, _)| e);
        
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

impl<'a> Deref for VerifiedEntity<'a> {
    type Target = Entity;
    
    fn deref(&self) -> &Entity {
        &self.inner
    }
}

/// The world stores component and entity data.
pub struct World<S: Set> {
    data: S,
    entities: EntityManager,
}

pub struct WorldHandle<'a, S: 'a + Set> {
    data: &'a S,
    entities: &'a EntityManager,
}

impl<'a, S: 'a + Set> WorldHandle<'a, S> {
    /// Create a query against the world data.
    ///
    /// # Examples
    /// ```
    /// use snorkium::ecs::*;
    /// #[derive(Clone, Copy)]
    /// struct Position(f32, f32);
    /// #[derive(Clone, Copy)]
    /// struct Dot;
    ///     
    /// // imagine this draws a dot at the position.
    /// fn draw_dot(_: &Position) { }
    /// 
    /// struct DotSystem;
    /// impl System for DotSystem {
    ///     // draw a dot for each entity with a position and the zero-sized dot component.
    ///     fn process<'a, S: 'a + Set>(&mut self, wh: WorldHandle<'a, S>) {
    ///         wh.query::<(Position, Dot)>().for_each(|e, (p, d)| {
    ///             draw_dot(p); 
    ///         });
    ///     }
    /// }
    /// ```
    pub fn query<F>(&self) -> Query<'a, S, F::Pipeline>
    where F: PipelineFactory {
        Query {
            set: self.data,
            entities: self.entities,
            pipeline: F::create(),
        }
    }
}

pub trait System: Send + Sync {
    fn process<'a, S: 'a + Set>(&mut self, wh: WorldHandle<'a, S>);
}

/// The empty set.
pub struct Empty;

/// A non-empty set.
pub struct Entry<T: Component, P: Set> {
    data: T::Storage,
    parent: P,
    _marker: PhantomData<T>,
}

/// A set of component storage data structures.
/// 
/// This is implemented as a recursive-variadic
/// data structure, which will allow for instant access of
/// component storage. The major downside is that attempted access
/// of components not in the set will resolve to a panic at runtime
/// rather than a compile error.
pub trait Set: Sized + Sync {
    fn push<T: Component>(self) -> Entry<T, Self>
    where T::Storage: Default {
        self.push_custom(Default::default())
    }
    
    fn push_custom<T: Component>(self, storage: T::Storage) -> Entry<T, Self> {
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

impl<T: Component, P: Set> Set for Entry<T, P> {
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
    /// relevant component data. This will output a vector of the returned
    /// outputs from the function.
    fn for_each<F, U: Send, S: Set>(self, &'a S, &'a EntityManager, F) -> Vec<U>
    where F: 'a + Sync + Fn(VerifiedEntity, Self::Item) -> U;
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
    pub fn for_each<F, U: Send>(self, f: F) -> Vec<U>
    where F: 'a + Sync + Fn(VerifiedEntity, <P as Pipeline<'a>>::Item) -> U {
        self.pipeline.for_each(self.set, self.entities, f)
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