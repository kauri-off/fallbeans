use crate::math::{Affine, V3, compose};

pub type NodeId = u32;

/// One transform in the map's scene graph.
#[derive(Clone, Debug)]
pub struct Node {
    pub parent: Option<NodeId>,
    pub pos: V3,
    /// Euler angles, XYZ order.
    pub rot: V3,
    pub scale: V3,
    pub visible: bool,
    pub world: Affine,
}

/// Nodes in creation order: a parent always comes before its children.
#[derive(Clone, Debug)]
pub struct Nodes(pub Vec<Node>);

pub const ROOT: NodeId = 0;

impl Default for Nodes {
    fn default() -> Self {
        Self(vec![Node {
            parent: None,
            pos: V3::ZERO,
            rot: V3::ZERO,
            scale: V3::ONE,
            visible: true,
            world: Affine::IDENTITY,
        }])
    }
}

impl Nodes {
    pub fn add(&mut self, parent: NodeId, pos: V3) -> NodeId {
        let id = self.0.len() as NodeId;
        self.0.push(Node {
            parent: Some(parent),
            pos,
            rot: V3::ZERO,
            scale: V3::ONE,
            visible: true,
            world: Affine::IDENTITY,
        });
        id
    }

    #[inline]
    pub fn get(&self, id: NodeId) -> &Node {
        &self.0[id as usize]
    }

    #[inline]
    pub fn get_mut(&mut self, id: NodeId) -> &mut Node {
        &mut self.0[id as usize]
    }

    /// Recomputes one node's world matrix from its parent's current one.
    pub fn update_one(&mut self, id: NodeId) {
        let n = &self.0[id as usize];
        let local = compose(n.pos, n.rot, n.scale);
        let world = match n.parent {
            Some(p) => self.0[p as usize].world * local,
            None => local,
        };
        self.0[id as usize].world = world;
    }

    /// Updates the world matrices of the chain from the root down to `id`.
    pub fn update_chain(&mut self, id: NodeId) {
        if let Some(p) = self.0[id as usize].parent {
            self.update_chain(p);
        }
        self.update_one(id);
    }

    pub fn update_all(&mut self) {
        for i in 0..self.0.len() {
            self.update_one(i as NodeId);
        }
    }

    /// Visible unless it or an ancestor is hidden.
    pub fn shown(&self, mut id: NodeId) -> bool {
        loop {
            let n = &self.0[id as usize];
            if !n.visible {
                return false;
            }
            match n.parent {
                Some(p) => id = p,
                None => return true,
            }
        }
    }
}
