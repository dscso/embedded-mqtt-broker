use crate::topics_list::TopicsList;
use core::future::{poll_fn, Future};
use core::task::{Context, Poll, Waker};
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::mutex::Mutex;
use heapless::{Deque, String, Vec};

pub type Msg = Vec<u8, 64>;
pub type Topic = String<64>;
pub type InnerDistributorMutex<const N: usize> = Mutex<NoopRawMutex, InnerDistributor<N>>;
const QUEUE_LEN: usize = 2;
const TREE_SIZE: usize = 64;
#[derive(Debug)]
pub struct MessageInQueue {
    message: Message,
    subscribers: usize,
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
    fn publish(&mut self, topic: &str, msg: &[u8]) {
        let vec = Vec::from_slice(msg).unwrap(); // todo correct error handling
        let subscribers = self.tree.get_subscribed(topic);
        if subscribers.is_empty() {
            return;
        }
        let msg = Message {
            topic: String::try_from(topic).unwrap(), // todo correct error handling
            msg: vec,
        };
        let msg = MessageInQueue {
            message: msg,
            subscribers: subscribers.len(),
        };
        self.queue.push_back(msg).unwrap(); // todo correct error handling
        for i in subscribers.iter() {
            self.indices[*i] += 1;
            if let Some(w) = self.wakers[*i].as_ref() {
                w.wake_by_ref()
            }
        }
    }
    fn subscribe(&mut self, subscription: &str, id: usize) {
        self.tree.insert(subscription, id);
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
    pub async fn publish(&self, topic: &str, msg: &[u8]) {
        self.inner.lock().await.publish(topic, msg);
    }
    pub async fn subscribe(&self, subscription: &str) {
        self.inner.lock().await.subscribe(subscription, self.id);
    }
    pub async fn unsubscibe_all_topics(&self) {
        self.inner.lock().await.unsubscribe_all_topics(self.id);
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
        if inner.indices[id] > 0 {
            inner.indices[id] -= 1;
            // might do an optimization here to avoid cloning waker every poll
            inner.wakers[id] = None;
            let item = inner.queue.iter_mut().last().unwrap();
            item.subscribers -= 1;
            if item.subscribers == 0 {
                Poll::Ready(inner.queue.pop_back().unwrap().message)
            } else {
                Poll::Ready(item.message.clone())
            }
        } else {
            if inner.wakers[id].is_none() {
                inner.wakers[id] = Some(_cx.waker().clone());
            }
            Poll::Pending
        }
    }
}
