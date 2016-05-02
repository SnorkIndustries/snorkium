//! A multithreaded Entity Component System (ECS)

use std::collections::VecDeque;
use std::marker::PhantomData;
use std::ops::Deref;

use self::set::*;
use self::query::*;

const ID_BITS: usize = 24;
const MIN_UNUSED: usize = 1024;

pub mod query;
pub mod set;

/// A component is a piece of raw data which is associated with an entity.
///
/// "Systems" will typically iterate over all entities with a specific set of components,
/// performing some action for each.
pub trait Component: 'static + Copy + Send + Sync {
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

impl<T: 'static + Copy + Send + Sync> Component for T {
    default type Storage = DefaultStorage<Self>;
}

/// Component data storage.
///
/// In general, this will be used through `DefaultStorage`, but some components
/// can be more conveniently accessed through special data structures.
pub trait Storage<T: Component>: Sync + Send {
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
        Query::new(&self.data, &self.entities, F::create())
    }
}

/// Systems are where the bulk of the work of the ECS is done.
pub trait System: Send + Sync {
    fn process<'a, S: 'a + Set>(&mut self, wh: WorldHandle<'a, S>);
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