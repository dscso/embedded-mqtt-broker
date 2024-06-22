use core::{fmt, usize};
use heapless::{FnvIndexSet, String, Vec};

#[derive(Debug, Default)]
struct Node<const N: usize>  {
    name: String<64>,
    subscribers: FnvIndexSet<usize, N>,
    parent: Option<usize>,
    children: Vec<usize, N>,
}
pub struct Tree<const N: usize> {
    nodes: Vec<Option<Node<N>>, N>,
}

impl <const N: usize> Tree<N> {
    pub fn new() -> Self {
        let mut nodes = Vec::new();
        let root = Node {
            name: String::try_from("").unwrap(),
            subscribers: FnvIndexSet::new(),
            parent: None,
            children: Vec::new(),
        };
        nodes.push(Some(root)).unwrap();
        for _ in 1..N {
            nodes.push(None).unwrap_or_else(|_| panic!("Failed to create tree"));
        }
        Self {
            nodes,
        }
    }
    fn insert_node(&mut self, val: &str, parent: Option<usize>) -> Option<usize> {
        let node = Node {
            name: String::try_from(val).unwrap(),
            subscribers: FnvIndexSet::new(),
            parent,
            children: Vec::new(),
        };
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
            self.nodes.get_mut(parent).unwrap().as_mut().unwrap().children.push(id.unwrap()).unwrap();
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
        self.nodes.get_mut(parent).unwrap().as_mut().unwrap().subscribers.insert(subscription).unwrap();
    }
    fn get_child_id(&self, parent: usize, name: &str) -> Option<usize> {
        let parent = self.nodes[parent].as_ref()?;
        for child_id in parent.children.iter() {
            let child = self.nodes[*child_id].as_ref()?;
            if child.name == name {
                return Some(*child_id);
            }
        }
        return None;
    }
    fn get_node(&self, id: usize) -> Option<&Node<N>> {
        self.nodes[id].as_ref()
    }
    pub fn get_subscribed(&self, topic: &str) -> FnvIndexSet<usize, N>{
        let mut parent = 0; // root
        let mut subscribers = FnvIndexSet::new();
        for name in topic.split('/') {
            if name.is_empty() {
                continue;
            }
            if let Some(child_id) = self.get_child_id(parent, name) {
                parent = child_id;
                self.get_node(parent).unwrap().subscribers.iter().for_each(|s| {
                    subscribers.insert(*s).unwrap();
                });
            } else {
                // return no subscribers if no child found
                return subscribers;
            }
        }

        subscribers
    }
}

impl <const N: usize> fmt::Debug for Tree<N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Tree")
            .field("nodes", &self.nodes)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_tree() {
        let mut tree = Tree::<64>::new();
        tree.insert("/a/b", 1);
        tree.insert("/a/b", 3);
        tree.insert("/a/b", 4);
        tree.insert("/a/b", 6);
        tree.insert("/c/b", 7);
        assert_eq!(tree.get_subscribed("/a/b"), [1, 3, 4, 6]);
        //assert_eq!(tree.get_subscribed("/c/b"), [7]);
        //tree.insert("/a/b/d", 111);
        //tree.insert("a/b/d", 2);
        //tree.insert("a/b/e", 3);
        //debug!("{:?}", tree);
        //panic!("{:?}", tree);
    }
}