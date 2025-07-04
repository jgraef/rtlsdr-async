use std::{
    collections::HashSet,
    num::NonZeroUsize,
};

use adsbee_api_types::IcaoAddress;

#[derive(Clone, Debug)]
pub(super) struct Node {
    north_west: [f32; 2],
    south_east: [f32; 2],
    centroid: [f32; 2],
    pub(super) entries: HashSet<IcaoAddress>,
    pub(super) children: Option<Box<[Node; 4]>>,
}

impl Node {
    fn new(north_west: [f32; 2], south_east: [f32; 2]) -> Self {
        let centroid = [
            0.5 * (north_west[0] + south_east[0]),
            0.5 * (north_west[1] + south_east[1]),
        ];
        Self {
            north_west,
            south_east,
            centroid,
            entries: HashSet::new(),
            children: None,
        }
    }

    fn insert(&mut self, address: IcaoAddress, position: [f32; 2], depth: usize) {
        self.entries.insert(address);

        if depth > 1 {
            let children = self.children.get_or_insert_with(|| {
                assert!(self.entries.is_empty());

                Box::new([
                    Node::new(
                        [self.north_west[0], self.centroid[1]],
                        [self.centroid[1], self.south_east[1]],
                    ),
                    Node::new(self.centroid, self.south_east),
                    Node::new(self.north_west, self.centroid),
                    Node::new(
                        [self.centroid[0], self.north_west[1]],
                        [self.south_east[0], self.centroid[1]],
                    ),
                ])
            });

            let index = if position[0] < self.centroid[0] { 0 } else { 1 }
                | if position[1] < self.centroid[1] { 0 } else { 2 };

            children[index].insert(address, position, depth - 1);
        }
    }
}

#[derive(Clone, Debug)]
pub struct Tree {
    root: Node,
    depth: NonZeroUsize,
}

impl Tree {
    pub fn new(depth: NonZeroUsize) -> Self {
        Self {
            root: Node::new([-180.0, 90.0], [180.0, -90.0]),
            depth,
        }
    }

    pub fn insert(&mut self, address: IcaoAddress, position: [f32; 2]) {
        self.root.insert(address, position, self.depth.get());
    }

    pub fn get(&self, _p1: [f32; 2], _p2: [f32; 2]) -> TreeIter<'_> {
        todo!();
    }
}

#[derive(Debug)]
pub struct TreeIter<'a> {
    tree: &'a Tree,
}
