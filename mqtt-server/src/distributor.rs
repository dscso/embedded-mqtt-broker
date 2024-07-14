use crate::errors::DistributorError;
use crate::topics_list::TopicsList;
use core::future::{poll_fn, Future};
use core::task::{Context, Poll, Waker};
use heapless::{Deque, String, Vec};
use log::info;
use crate::config::{SubscriberBitSet, QUEUE_LEN, TREE_SIZE, InnerDistributorMutex};


#[derive(Debug)]
pub struct MessageInQueue {
    message: Message,
    subscribers: SubscriberBitSet,
}
#[derive(Debug, Clone)]
pub struct Message {
    topic: String<64>,
    msg: Vec<u8, 1024>,
}

impl Message {
    pub fn topic(&self) -> &str {
        self.topic.as_str()
    }
    pub fn message(&self) -> &[u8] {
        self.msg.as_slice()
    }
}
pub struct InnerDistributor<const N: usize> {
    queue: Deque<MessageInQueue, QUEUE_LEN>,
    tree: TopicsList<TREE_SIZE, N>,
    wakers: [Option<Waker>; N],
    lock_wakers: [Option<Waker>; N],
    lock: SubscriberBitSet
}

impl<const N: usize> Default for InnerDistributor<N> {
    fn default() -> Self {
        const NONE_WAKER: Option<Waker> = None;
        Self {
            queue: Default::default(),
            tree: Default::default(),
            wakers: [NONE_WAKER; N],
            lock_wakers: [NONE_WAKER; N],
            lock: Default::default(),
        }
    }
}
impl<const N: usize> InnerDistributor<N> {
    fn lock_for_publishing(&mut self, id: usize) -> Result<(), DistributorError> {
        assert!(!self.lock.get(id), "Lock already set");
        self.lock.set(id);
        Ok(())
    }
    fn unlock_for_publishing(&mut self, id: usize) {
        self.lock.unset(id);
        self.lock_wakers.iter().for_each(|w| {
            if let Some(w) = w.as_ref() {
                w.wake_by_ref()
            }
        });
    }
    fn publish(&mut self, topic: &str, msg: &[u8]) -> Result<(), DistributorError> {
        let message = Vec::from_slice(msg).map_err(|_| DistributorError::MessageTooLong)?;
        let topic_string = String::try_from(topic).map_err(|_| DistributorError::TopicTooLong)?;
        let subscribers = self.tree.get_subscribed(topic);
        if subscribers.is_empty() {
            return Ok(());
        }
        let mut sub_bitset = SubscriberBitSet::default();
        for subscriber in subscribers.iter() {
            sub_bitset.set(*subscriber);
        }
        let msg = Message {
            topic: topic_string,
            msg: message,
        };
        let msg = MessageInQueue {
            subscribers: sub_bitset,
            message: msg,
        };
        self.queue.push_back(msg).map_err(|e| {
            panic!("Queue full {:?}\nDied on topic {}", e, topic);
        })/*.map_err(|_| DistributorError::QueueFull)*/.unwrap();
        for i in subscribers.iter() {
            if let Some(w) = self.wakers[*i].as_ref() {
                w.wake_by_ref()
            }
        }
        Ok(())
    }
    fn subscribe(&mut self, subscription: &str, id: usize) -> Result<(), DistributorError> {
        self.tree.insert(subscription, id).map_err(|e| e.into())
    }
    fn unsubscribe(&mut self, subscription: &str, id: usize) {
        self.tree.remove(subscription, id);
    }
    fn unsubscribe_all_topics(&mut self, id: usize) {
        self.tree.remove_all_subscriptions(id);
        let cleanup_necessary = self.queue.iter_mut().any(|msg| {
            let cleanup_necessary = msg.subscribers.get(id);
            msg.subscribers.unset(id);
            cleanup_necessary
        });
        if cleanup_necessary {
            for _ in 0..self.queue.len() {
                let e = self.queue.pop_back().unwrap();
                if !e.subscribers.is_empty() {
                    self.queue.push_front(e).unwrap()
                }
            }
        }
    }

    #[allow(unused)]
    fn get_last(&self, id: usize) -> Option<&MessageInQueue> {
        self
            .queue
            .back()
            .filter(|last| last.subscribers.get(id))
    }
    #[allow(unused)]
    fn get_last_mut(&mut self, id: usize) -> Option<&mut MessageInQueue> {
        self
            .queue
            .back_mut()
            .filter(|last| last.subscribers.get(id))
    }
}

pub struct Distributor<const N: usize> {
    id: usize,
    inner: &'static InnerDistributorMutex<N>,
}


impl<const N: usize> Distributor<N> {
    /// This function so the server only processes n MQTT messages at a time
    /// were: n = QUEUE_LEN
    /// it should be used as follows
    /// ```example
    /// loop {
    ///     let msg = match select(distributor.next(), distributor.lock(parser.next())).await {
    ///         ...
    ///     }
    ///     distributor.unlock();
    /// }
    /// ```
    pub(crate) async fn lock<T>(&self, feature: impl Future<Output=T>) -> T  {
        let res = feature.await;
        info!("recv socket");

        // delay till there is enough space
        poll_fn(move |cx| {
            let mut inner = self.inner.try_lock().unwrap();

            let available = QUEUE_LEN - inner.queue.len();

            return if available > inner.lock.count_ones() {
                inner.lock_wakers[self.id] = None;
                inner.lock_for_publishing(self.id).unwrap();
                Poll::Ready(())
            } else {
                inner.lock_wakers[self.id] = Some(cx.waker().clone());
                Poll::Pending
            }
        }).await;

        res
    }
    pub fn unlock(&self) {
        self.inner.try_lock().unwrap().unlock_for_publishing(self.id);
    }
}

impl<'a, const N: usize> Distributor<N> {
    pub fn new(inner: &'static InnerDistributorMutex<N>, id: usize) -> Self {
        Self { id, inner }
    }
    pub fn get_id(&self) -> usize {
        self.id
    }
    
    pub fn publish(&self, topic: &str, msg: &[u8]) -> Result<(), DistributorError> {
        self.inner.try_lock().unwrap().publish(topic, msg)
    }
    pub fn subscribe(&self, subscription: &str) -> Result<(), DistributorError> {
        self.inner.try_lock().unwrap().subscribe(subscription, self.id)
    }
    pub fn unsubscibe_all_topics(&self) {
        self.inner.try_lock().unwrap().unsubscribe_all_topics(self.id);
    }
    pub fn unsubscribe(&self, subscription: &str) {
        self.inner.try_lock().unwrap().unsubscribe(subscription, self.id);
    }
    pub fn next(&self) -> impl Future<Output = Message> + '_ {
        poll_fn(move |cx| self.poll_next(cx, self.id))
    }

    fn poll_next(&self, _cx: &mut Context, id: usize) -> Poll<Message> {
        // todo maybe needs to be changed? Does the task wake up again?
        let mut inner = self.inner.try_lock().unwrap();

        let message = match inner.get_last_mut(id) {
            // last message is not ment for this subscriber
            None => {
                inner.wakers[id] = Some(_cx.waker().clone());
                return Poll::Pending;
            }
            Some(last) => {
                last.subscribers.unset(id);
                if last.subscribers.is_empty() {
                    // return the value itself if this was the last subscriber
                    inner.queue.pop_back().unwrap().message
                } else {
                    // return a clone since we still need the original for the other subscribers
                    last.message.clone()
                }
            }
        };
        if inner.queue.is_empty() {
            inner.lock_wakers.iter().for_each(|w| {
                if let Some(w) = w.as_ref() {
                    w.wake_by_ref()
                }
            });
        }
        inner.wakers[id] = None;
        Poll::Ready(message)
    }
}
