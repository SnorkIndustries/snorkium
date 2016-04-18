//! A multithreaded Entity Component Systems (ECS)

use std::collections::VecDeque;

const ID_BITS: usize = 24;
const MIN_UNUSED: usize = 1024;

pub trait Component: Copy + 'static {}
impl<T: Copy + 'static> Component for T {}

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
/// be useful.
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