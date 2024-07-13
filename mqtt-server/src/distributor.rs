use crate::bitset::BitSet;
use crate::errors::DistributorError;
use crate::topics_list::TopicsList;
use core::future::{poll_fn, Future};
use core::task::{Context, Poll, Waker};
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::mutex::Mutex;
use heapless::{Deque, String, Vec};

pub type Msg = Vec<u8, 64>;
pub type Topic = String<64>;
pub type InnerDistributorMutex<const N: usize> = Mutex<NoopRawMutex, InnerDistributor<N>>;
const QUEUE_LEN: usize = 3;
const TREE_SIZE: usize = 64;
// so u never forget this is used for the distributor. more it can not address
const MAX_SOCKET_SUBSCRIBERS: usize = 64;
#[derive(Debug)]
pub struct MessageInQueue {
    message: Message,
    subscribers: BitSet<MAX_SOCKET_SUBSCRIBERS>,
}
#[derive(Debug, Clone)]
pub struct Message {
    topic: String<64>,
    msg: Vec<u8, 64>,
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
    indices: [usize; N],
    wakers: [Option<Waker>; N],
}

impl<const N: usize> Default for InnerDistributor<N> {
    fn default() -> Self {
        const NONE_WAKER: Option<Waker> = None;
        Self {
            queue: Default::default(),
            tree: Default::default(),
            indices: [0; N],
            wakers: [NONE_WAKER; N],
        }
    }
}
impl<const N: usize> InnerDistributor<N> {
    fn publish(&mut self, topic: &str, msg: &[u8]) -> Result<(), DistributorError> {
        let message = Vec::from_slice(msg).map_err(|_| DistributorError::MessageTooLong)?;
        let topic_string = String::try_from(topic).map_err(|_| DistributorError::TopicTooLong)?;
        let subscribers = self.tree.get_subscribed(topic);
        if subscribers.is_empty() {
            return Ok(());
        }
        let mut sub_bitset = BitSet::default();
        for subsciber in subscribers.iter() {
            sub_bitset.set(*subsciber);
        }
        let msg = Message {
            topic: topic_string,
            msg: message,
        };
        let msg = MessageInQueue {
            subscribers: sub_bitset,
            message: msg,
        };
        self.queue.push_back(msg).unwrap(); //.map_err(|_| DistributorError::QueueFull)?; // todo correct error handling
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
    }
}

pub struct Distributor<const N: usize> {
    id: usize,
    inner: &'static InnerDistributorMutex<N>,
}

impl<'a, const N: usize> Distributor<N> {
    pub async fn new(inner: &'static InnerDistributorMutex<N>, id: usize) -> Self {
        Self { id, inner }
    }
    pub async fn publish(&self, topic: &str, msg: &[u8]) -> Result<(), DistributorError> {
        self.inner.lock().await.publish(topic, msg)
    }
    pub async fn subscribe(&self, subscription: &str) -> Result<(), DistributorError> {
        self.inner.lock().await.subscribe(subscription, self.id)
    }
    pub async fn unsubscibe_all_topics(&self) {
        self.inner.lock().await.unsubscribe_all_topics(self.id);
        // todo
        // clean up possibly leaked message that was distributed between connection
        // was closed and all topics were unsubscribed
    }
    pub async fn unsubscribe(&self, subscription: &str) {
        self.inner.lock().await.unsubscribe(subscription, self.id);
    }
    pub fn next(&self) -> impl Future<Output = Message> + '_ {
        poll_fn(move |cx| self.poll_at(cx, self.id))
    }

    fn poll_at(&self, _cx: &mut Context, id: usize) -> Poll<Message> {
        // todo maybe needs to be changed? Does the task wake up again?
        let mut inner = match self.inner.try_lock() {
            Ok(inner) => inner,
            Err(_e) => {
                panic!("In single core system mutex can not be already locked");
                //return Poll::Pending;
            }
        };
        // check if last message is form current subscriber (defined by id)
        let last = inner
            .queue
            .back_mut()
            .filter(|last| last.subscribers.get(id));
        let message = match last {
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
        inner.wakers[id] = None;
        Poll::Ready(message)
    }
}
