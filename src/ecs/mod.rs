//! A multithreaded Entity Component Systems (ECS)

use std::collections::VecDeque;
use std::marker::PhantomData;

const ID_BITS: usize = 24;
const MIN_UNUSED: usize = 1024;

pub trait Component: Copy + 'static {}
impl<T: Copy + 'static> Component for T {}

// Component data storage.
struct DefaultStorage<T: Component> {
    // data vector -- this is tightly packed.
    data: Vec<T>,
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
    
    /// Sets the component for the given entity.
    fn set(&mut self, e: VerifiedEntity, data: T) {
        let id = e.entity().id() as usize;
        while self.indices.len() < id as usize {
            self.indices.push(None);
        }
        
        if let Some(idx) = self.indices[id] {
            self.data[idx] = data;
        } else if let Some(idx) = self.unused.pop_front() {
            self.data[idx] = data;
            self.indices[id] = Some(idx);
        } else {
            self.data.push(data);
            self.indices[id] = Some(self.data.len() - 1);
        }
    }
    
    /// Get a reference to an entity's data.
    fn get(&self, e: VerifiedEntity) -> Option<&T> {
        match self.indices.get(e.entity().id() as usize) {
            Some(&Some(idx)) => {
                Some(&self.data[idx])
            }
            _ => None,
        }
    }
    
    /// Get a mutable reference to an entity's data.
    fn get_mut(&mut self, e: VerifiedEntity) -> Option<&mut T> {
        match self.indices.get(e.entity().id() as usize) {
            Some(&Some(idx))=> {
                Some(&mut self.data[idx])
            }
            _ => None,
        }
    }
    
    /// Remove an entity's data, returning it by value if it existed.
    fn remove(&mut self, e: VerifiedEntity) -> Option<T> {
        match self.indices.get(e.entity().id() as usize) {
            Some(&Some(idx)) => {
                self.indices[e.entity().id() as usize] = None;
                self.unused.push_back(idx);
                
                Some(self.data[idx])
            }       
            _ => None,
        }
    }
    
    /// Destroy an entity's data without returning the data.
    ///
    /// This entity may not be alive.
    /// This will usually be called with entities that have been
    /// destroyed in a previous frame to have storage mappers clean
    /// up.
    fn destroy(&mut self, e: Entity) {
        if let Some(&Some(idx)) = self.indices.get(e.id() as usize) {
            self.indices[e.id() as usize] = None;
            self.unused.push_back(idx);
        }
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