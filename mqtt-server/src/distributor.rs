use core::future::{Future, poll_fn};
use core::task::{Context, Poll, Waker};
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::mutex::Mutex;
use heapless::{Deque, Vec};
use mqtt_format::v5::packets::subscribe::Subscriptions;
use crate::topics::Tree;

pub type Msg = Vec<u8, 64>;
pub type InnerDistributorMutex<const N: usize> = Mutex<NoopRawMutex, InnerDistributor<N>>;
const QUEUE_LEN: usize = 4;


pub struct InnerDistributor<const N: usize> {
    queue: Deque<(Msg, usize), QUEUE_LEN>,
    tree: Tree<N>,
    indices: [usize; N],
    wakers: [Option<Waker>; N],
}

impl<const N: usize> Default for InnerDistributor<N> {
    fn default() -> Self {
        const NONE_WAKER: Option<Waker> = None;
        Self {
            queue: Default::default(),
            tree: Tree::new(),
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
        self.queue.push_back((vec, subscribers.len())).unwrap(); // todo correct error handling
        for i in subscribers.iter() {
            self.indices[*i] += 1;
            self.wakers[*i].as_ref().map(|w| w.wake_by_ref());
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
        Self {
            id,
            inner
        }
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
    pub fn next(&self) -> impl Future<Output = Msg> + '_ {
        poll_fn(move |cx| {
            self.poll_at(cx, self.id)
        })
    }

    fn poll_at(&self, _cx: &mut Context, id: usize) -> Poll<Msg> {
        // todo maybe needs to be changed? Does the task wake up again?
        let mut inner = match self.inner.try_lock() {
            Ok(inner) => inner,
            Err(_e) => {
                return Poll::Pending;
            }
        };
        if inner.indices[id] > 0 {
            inner.indices[id] -= 1;
            // might do an optimization here to avoid cloning waker every poll
            inner.wakers[id] = None;
            let item = inner.queue.iter_mut().last().unwrap();
            item.1 -= 1;
            if item.1 == 0 {
                Poll::Ready(inner.queue.pop_back().unwrap().0)
            } else {
                Poll::Ready(item.0.clone())
            }
        } else {
            if inner.wakers[id].is_none() {
                inner.wakers[id] = Some(_cx.waker().clone());
            }
            Poll::Pending
        }
    }
}