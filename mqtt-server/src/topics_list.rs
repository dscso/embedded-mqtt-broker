use heapless::{FnvIndexSet, String, Vec};

#[derive(Debug, Default)]
pub struct TopicsList<const N: usize, const MAX_SUBS: usize> {
    topics: FnvIndexSet<(String<64>, usize), N>,
}

impl<const N: usize, const MAX_SUBS: usize> TopicsList<N, MAX_SUBS> {
    pub(crate) fn insert(&mut self, topic: &str, id: usize) {
        self.topics
            .insert((String::try_from(topic).unwrap(), id))
            .unwrap();
    }
    pub(crate) fn remove(&mut self, topic: &str, id: usize) {
        self.topics.retain(|(t, i)| t.as_str() != topic || *i != id);
    }
    pub(crate) fn get_subscribed(&self, topic: &str) -> Vec<usize, MAX_SUBS> {
        let mut subscribers = Vec::new();
        for (t, i) in self.topics.iter() {
            if listens_to_topic(t.as_str(), topic) {
                subscribers.push(*i).unwrap();
            }
        }
        subscribers
    }
    pub(crate) fn remove_all_subscriptions(&mut self, id: usize) {
        self.topics.retain(|(_, i)| *i != id);
    }
}

fn listens_to_topic(subscription: &str, topic: &str) -> bool {
    let sub_iter = subscription.split('/');
    let topic_iter = topic.split('/');

    let mut sub_iter = sub_iter.filter(|s| !s.is_empty());
    let mut topic_iter = topic_iter.filter(|s| !s.is_empty());

    loop {
        return match (sub_iter.next(), topic_iter.next()) {
            (Some(sub), Some(top)) => {
                if sub == "#" {
                    return true;
                }
                if sub == "+" || sub == top {
                    continue;
                }
                false
            }
            (None, _) => true,
            _ => false,
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_listens_to_topic() {
        assert!(listens_to_topic("/a/b/c", "/a/b/c"));
        assert!(listens_to_topic("/", "/a/b/c"));
        assert!(listens_to_topic("/a/b//c", "/a/b/c"));
        assert!(listens_to_topic("/a/b/c", "//a/b/c"));
        assert!(listens_to_topic("/a/+/c", "/a/b/c/d/e/f"));

        assert!(!listens_to_topic("/a/b/c", "/a/b/d"));
        assert!(!listens_to_topic("/a/b/c", "/"));
        assert!(!listens_to_topic("/a/b/c", "/d"));
    }
}
