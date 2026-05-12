// ABOUTME: Port of nodebuild_events.cpp. A binary search tree of FEvent records keyed
// ABOUTME: by `distance` (f64). Despite the C++ file header saying "red-black", the
// ABOUTME: implementation is an unbalanced BST — no color, rotations, or rebalancing.

use super::EventInfo;

/// Sentinel for "no node" index, used in place of the C++ `&Nil` pointer.
pub const NIL: u32 = u32::MAX;

/// One BST node. Layout mirrors `FEvent` from nodebuild.h.
#[derive(Debug, Clone, Copy)]
struct EventNode {
    parent: u32,
    left: u32,
    right: u32,
    distance: f64,
    info: EventInfo,
}

impl EventNode {
    fn empty() -> Self {
        Self {
            parent: NIL,
            left: NIL,
            right: NIL,
            distance: 0.0,
            info: EventInfo::default(),
        }
    }
}

/// BST of `FEvent` records. Operations return arena indices (`u32`); use `info` /
/// `info_mut` / `distance` to read field data.
pub struct EventTree {
    nodes: Vec<EventNode>,
    root: u32,
}

impl Default for EventTree {
    fn default() -> Self {
        Self::new()
    }
}

impl EventTree {
    pub fn new() -> Self {
        Self {
            nodes: Vec::new(),
            root: NIL,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.root == NIL
    }

    /// Wipe every entry. Capacity is retained.
    pub fn delete_all(&mut self) {
        self.nodes.clear();
        self.root = NIL;
    }

    /// Insert a new record with `distance` as key. Duplicates are placed to the right,
    /// matching the C++ behavior (`Distance < y->Distance` chooses left; equality goes
    /// right).
    pub fn insert(&mut self, distance: f64, info: EventInfo) -> u32 {
        let z = self.nodes.len() as u32;
        let mut node = EventNode::empty();
        node.distance = distance;
        node.info = info;
        self.nodes.push(node);

        let mut y = NIL;
        let mut x = self.root;
        while x != NIL {
            y = x;
            x = if distance < self.nodes[x as usize].distance {
                self.nodes[x as usize].left
            } else {
                self.nodes[x as usize].right
            };
        }
        self.nodes[z as usize].parent = y;
        if y == NIL {
            self.root = z;
        } else if distance < self.nodes[y as usize].distance {
            self.nodes[y as usize].left = z;
        } else {
            self.nodes[y as usize].right = z;
        }
        z
    }

    /// Exact-distance lookup. Returns the first matching node found.
    pub fn find_event(&self, distance: f64) -> Option<u32> {
        let mut node = self.root;
        while node != NIL {
            let d = self.nodes[node as usize].distance;
            if d == distance {
                return Some(node);
            }
            node = if d > distance {
                self.nodes[node as usize].left
            } else {
                self.nodes[node as usize].right
            };
        }
        None
    }

    /// Leftmost node (smallest distance), or `None` if empty.
    pub fn get_minimum(&self) -> Option<u32> {
        if self.root == NIL {
            return None;
        }
        let mut node = self.root;
        while self.nodes[node as usize].left != NIL {
            node = self.nodes[node as usize].left;
        }
        Some(node)
    }

    /// In-order successor of `event`, or `None` if it is the maximum.
    pub fn successor(&self, event: u32) -> Option<u32> {
        let right = self.nodes[event as usize].right;
        if right != NIL {
            let mut node = right;
            while self.nodes[node as usize].left != NIL {
                node = self.nodes[node as usize].left;
            }
            return Some(node);
        }
        let mut node = event;
        let mut y = self.nodes[node as usize].parent;
        while y != NIL && node == self.nodes[y as usize].right {
            node = y;
            y = self.nodes[y as usize].parent;
        }
        if y == NIL {
            None
        } else {
            Some(y)
        }
    }

    /// In-order predecessor of `event`, or `None` if it is the minimum.
    pub fn predecessor(&self, event: u32) -> Option<u32> {
        let left = self.nodes[event as usize].left;
        if left != NIL {
            let mut node = left;
            while self.nodes[node as usize].right != NIL {
                node = self.nodes[node as usize].right;
            }
            return Some(node);
        }
        let mut node = event;
        let mut y = self.nodes[node as usize].parent;
        while y != NIL && node == self.nodes[y as usize].left {
            node = y;
            y = self.nodes[y as usize].parent;
        }
        if y == NIL {
            None
        } else {
            Some(y)
        }
    }

    pub fn distance(&self, event: u32) -> f64 {
        self.nodes[event as usize].distance
    }

    pub fn info(&self, event: u32) -> EventInfo {
        self.nodes[event as usize].info
    }

    pub fn info_mut(&mut self, event: u32) -> &mut EventInfo {
        &mut self.nodes[event as usize].info
    }

    /// Iterate nodes in ascending `distance` order.
    pub fn iter(&self) -> EventIter<'_> {
        EventIter {
            tree: self,
            next: self.get_minimum(),
        }
    }
}

pub struct EventIter<'a> {
    tree: &'a EventTree,
    next: Option<u32>,
}

impl<'a> Iterator for EventIter<'a> {
    type Item = u32;
    fn next(&mut self) -> Option<u32> {
        let cur = self.next?;
        self.next = self.tree.successor(cur);
        Some(cur)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(vertex: i32) -> EventInfo {
        EventInfo {
            vertex,
            front_seg: 0,
        }
    }

    #[test]
    fn empty_tree() {
        let t = EventTree::new();
        assert!(t.is_empty());
        assert_eq!(t.get_minimum(), None);
        assert_eq!(t.find_event(0.0), None);
    }

    #[test]
    fn insert_and_find() {
        let mut t = EventTree::new();
        t.insert(3.0, info(1));
        t.insert(1.0, info(2));
        t.insert(2.0, info(3));
        t.insert(5.0, info(4));
        t.insert(4.0, info(5));

        let min = t.get_minimum().unwrap();
        assert_eq!(t.distance(min), 1.0);
        assert_eq!(t.info(min).vertex, 2);

        assert!(t.find_event(2.0).is_some());
        assert!(t.find_event(99.0).is_none());
    }

    #[test]
    fn in_order_iteration_is_sorted() {
        let mut t = EventTree::new();
        for d in [3.0, 1.0, 4.0, 1.5, 5.0, 9.0, 2.0, 6.0, 5.5] {
            t.insert(d, info(0));
        }
        let collected: Vec<f64> = t.iter().map(|i| t.distance(i)).collect();
        let mut sorted = collected.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert_eq!(collected, sorted);
    }

    #[test]
    fn successor_and_predecessor() {
        let mut t = EventTree::new();
        let a = t.insert(1.0, info(1));
        let _ = t.insert(2.0, info(2));
        let c = t.insert(3.0, info(3));
        let min = t.get_minimum().unwrap();
        assert_eq!(min, a);
        assert_eq!(t.predecessor(min), None);
        let after_min = t.successor(min).unwrap();
        assert_eq!(t.distance(after_min), 2.0);
        let after_mid = t.successor(after_min).unwrap();
        assert_eq!(t.distance(after_mid), 3.0);
        assert_eq!(t.successor(after_mid), None);
        assert_eq!(t.predecessor(c).map(|x| t.distance(x)), Some(2.0));
    }

    #[test]
    fn duplicates_go_right() {
        // Mirrors C++ behavior: equality routes to the right child.
        let mut t = EventTree::new();
        t.insert(1.0, info(10));
        t.insert(1.0, info(20));
        // Both findable; the first-inserted is the leftmost.
        let min = t.get_minimum().unwrap();
        assert_eq!(t.info(min).vertex, 10);
        let next = t.successor(min).unwrap();
        assert_eq!(t.info(next).vertex, 20);
    }

    #[test]
    fn delete_all_clears() {
        let mut t = EventTree::new();
        t.insert(1.0, info(1));
        t.insert(2.0, info(2));
        assert!(!t.is_empty());
        t.delete_all();
        assert!(t.is_empty());
        assert_eq!(t.get_minimum(), None);
        // Reinsertion still works.
        t.insert(7.0, info(99));
        assert_eq!(t.distance(t.get_minimum().unwrap()), 7.0);
    }
}
