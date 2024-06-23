use core::usize;
use heapless::{FnvIndexSet, String, Vec};
use crate::bitset::BitSet;

const MAX_DEPTH: usize = 16;

// cant use const generics because https://blog.rust-lang.org/inside-rust/2021/09/06/Splitting-const-generics.html#featuregeneric_const_exprs
#[derive(Debug, Default)]
struct Node<const N: usize> {
    name: String<64>,
    subscribers: BitSet<64>, // max amount of subscribers
    parent: Option<usize>,
    children: BitSet<64>, // max amount of nodes
}

impl<const N: usize> Node<N> {
    fn new(name: &str, parent: Option<usize>) -> Self {
        Self {
            name: String::try_from(name).unwrap(),
            subscribers: BitSet::default(),
            parent,
            children: BitSet::default(),
        }
    }
}

#[derive(Debug)]
pub struct Tree<const N: usize> {
    nodes: Vec<Option<Node<N>>, N>,
}

impl<const N: usize> Tree<N> {
    pub fn new() -> Self {
        let mut nodes = Vec::new();
        let root = Node::new("", None);
        nodes.push(Some(root)).unwrap();
        for _ in 1..N {
            nodes
                .push(None)
                .unwrap_or_else(|_| panic!("Failed to create tree"));
        }
        Self { nodes }
    }
    fn insert_node(&mut self, val: &str, parent: Option<usize>) -> Option<usize> {
        let node = Node::new(val, parent);
        // find first free slot
        let mut id = None;
        for (i, n) in self.nodes.iter_mut().enumerate() {
            if n.is_none() {
                *n = Some(node);
                id = Some(i);
                break;
            }
        }
        if let Some(parent) = parent {
            self.get_mut_node(parent)
                .children
                .set(id.unwrap());
        }
        id
    }
    pub fn insert(&mut self, topic: &str, subscription: usize) {
        let mut parent = 0; // root
        for name in topic.split('/') {
            if name.is_empty() {
                continue;
            }
            if let Some(child_id) = self.get_child_id(parent, name) {
                parent = child_id;
            } else {
                let child_id = self.insert_node(name, Some(parent));
                parent = child_id.unwrap();
            }
        }
        self.get_mut_node(parent)
            .subscribers
            .set(subscription);

    }
    pub fn get_ancestors(&self, topic: &str) -> Vec<usize, MAX_DEPTH> {
        let mut node = 0; // root
        let mut ancestors = Vec::<usize, MAX_DEPTH>::new();
        for name in topic.split('/') {
            if name.is_empty() {
                continue;
            }
            if let Some(child_id) = self.get_child_id(node, name) {
                node = child_id;
                ancestors.push(node).unwrap();
            } else {
                ancestors.clear();
                return ancestors;
            }
        }
        ancestors
    }
    fn get_mut_node(&mut self, id: usize) -> &mut Node<N> {
        self.nodes.get_mut(id).unwrap().as_mut().unwrap()
    }
    fn remove_if_not_needed(&mut self, node_id: usize) {
        let node = self.get_mut_node(node_id);
        if node.subscribers.is_empty() && node.children.is_empty() {
            // if not root
            if let Some(parent) = node.parent {
                let parent_node = self.get_mut_node(parent);
                parent_node.children.unset(node_id);
                // recursion
                *self.nodes.get_mut(node_id).unwrap() = None;
                self.remove_if_not_needed(parent);
            }
        }
    }
    pub fn remove(&mut self, topic: &str, subscription: usize) {
        let ancestors = self.get_ancestors(topic);
        if ancestors.is_empty() {
            return;
        }

        if let Some(last) = ancestors.iter().last() {
            let subscribers = self.get_mut_node(*last);
            subscribers.subscribers.unset(subscription);
            self.remove_if_not_needed(*last);
        }
    }
    pub fn remove_all_subscriptions(&mut self, id: usize) {
        for i in 1..N {
            if let Some(node) = self.nodes.get_mut(i).as_mut() {
                if let Some(node) = node {
                    node.subscribers.unset(id);
                    self.remove_if_not_needed(i);
                }
            }
        }
    }
    fn get_child_id(&self, parent: usize, name: &str) -> Option<usize> {
        let parent = self.nodes[parent].as_ref()?;
        for child_id in parent.children.iter_ones() {
            let child = self.nodes[child_id].as_ref()?;
            if child.name == name {
                return Some(child_id);
            }
        }
        return None;
    }
    fn get_node(&self, id: usize) -> &Node<N> {
        self.nodes[id].as_ref().unwrap()
    }
    pub fn get_subscribed(&self, topic: &str) -> FnvIndexSet<usize, N> {
        let mut parent = 0; // root
        let mut subscribers = FnvIndexSet::new();
        for name in topic.split('/') {
            if name.is_empty() {
                continue;
            }
            if let Some(child_id) = self.get_child_id(parent, name) {
                parent = child_id;
                self.get_node(parent)
                    .subscribers
                    .iter_ones()
                    .for_each(|s| {
                        subscribers.insert(s).unwrap();
                    });
            } else {
                // return no subscribers if no child found
                return subscribers;
            }
        }

        subscribers
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use log;
    fn to_vec<const N: usize>(set: FnvIndexSet<usize, N>) -> Vec<usize, N> {
        let mut vec = Vec::new();
        for s in set.iter() {
            vec.push(*s).unwrap();
        }
        vec.sort_unstable();
        vec
    }
    #[test]
    fn test_tree() {
        let mut tree = Tree::<64>::new();
        tree.insert("/a/b", 1);
        tree.insert("/a/b", 3);
        tree.insert("/a/b", 4);
        tree.insert("/a/b", 6);
        tree.insert("/c/b", 7);
        assert_eq!(to_vec(tree.get_subscribed("/a/b")).as_slice(), [1, 3, 4, 6]);
        assert_eq!(
            to_vec(tree.get_subscribed("/a/b/c")).as_slice(),
            [1, 3, 4, 6]
        );
        assert_eq!(to_vec(tree.get_subscribed("/a/d")).as_slice(), []);
        assert_eq!(to_vec(tree.get_subscribed("/c/b")).as_slice(), [7]);
        tree.remove("/a/b", 6);
        assert_eq!(to_vec(tree.get_subscribed("/a/b")).as_slice(), [1, 3, 4]);
        tree.remove("/a/b", 1);
        tree.remove("/a/b", 3);
        tree.remove("/a/b", 4);
        assert_eq!(to_vec(tree.get_subscribed("/c/b")).as_slice(), [7]);
        tree.remove("/c/b", 7);
        assert_eq!(to_vec(tree.get_subscribed("/c/b")).as_slice(), []);
    }
    #[test]
    fn test_cleanup() {
        let mut tree = Tree::<64>::new();
        tree.insert("/a/b", 1);
        tree.insert("/a/b", 2);
        tree.insert("/a/b", 3);
        tree.insert("/c/b", 1);
        tree.insert("/b/b/c/d", 1);
        tree.remove_all_subscriptions(1);
        assert_eq!(to_vec(tree.get_subscribed("/a/b")).as_slice(), [2, 3]);
        tree.remove("/a/b", 2);
        tree.remove("/a/b", 3);
        // only root should be left
        assert_eq!(tree.nodes.iter().filter(|n| n.is_some()).count(), 1);
    }
}
