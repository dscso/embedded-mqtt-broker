use core::future::poll_fn;
use core::task::{Context, Poll, Waker};
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::channel::{Channel, Receiver, Sender};
use embassy_sync::mutex::Mutex;
use embassy_sync::signal::Signal;
use log::{info, warn};
use crate::distributor::SomeCommand::On;

pub type Msg = u8;
const MAX_MSG_CHANNEL: usize = 1;
type DistributorChannel = Channel<NoopRawMutex, Msg, MAX_MSG_CHANNEL>;
type DistributorReceiver<'a> = Receiver<'a, NoopRawMutex, Msg, MAX_MSG_CHANNEL>;
pub type InnerDistributorMutex<const N: usize> = Mutex<NoopRawMutex, InnerDistributor<N>>;
enum SomeCommand {
    On,
    Off,
}

type DistributorSignal = Signal<NoopRawMutex, SomeCommand>;


pub struct InnerDistributor<const N: usize> {
    indices: [usize; N],
    wakers: [Option<Waker>; N],
}

impl<const N: usize> Default for InnerDistributor<N> {
    fn default() -> Self {
        const NONE_WAKER: Option<Waker> = None;
        Self {
            indices: [0; N],
            wakers: [NONE_WAKER; N],
        }
    }
}
impl<const N: usize> InnerDistributor<N> {
    fn publish(&mut self, id: usize, msg: Msg) {
        for i in 0..N {
            if i == id { continue; }
            self.indices[i] += 1;
            self.wakers[i].as_ref().map(|w| w.wake_by_ref());
        }
    }
}

pub struct Distributor<const N: usize> {
    //receiver: DistributorReceiver<'a>,
    inner: &'static InnerDistributorMutex<N>,
}

impl<'a, const N: usize> Distributor<N> {
    pub async fn new(inner: &'static InnerDistributorMutex<N>) -> Self {
        Self {
            inner
        }
    }
    pub async fn publish(&self, id: usize, msg: Msg) {
        self.inner.lock().await.publish(id, msg);
    }
    pub async fn subscribe(&self, id: usize) -> Msg {
        let ret  = poll_fn(move |cx| {
            self.poll_at(cx, id)
        }).await;
        warn!("subscribe!!! {}", ret);
        ret
    }

    fn poll_at(&self, _cx: &mut Context, id: usize) -> Poll<Msg> {
        info!("poll {id}");
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
            Poll::Ready(42)
        } else {
            if inner.wakers[id].is_none() {
                inner.wakers[id] = Some(_cx.waker().clone());
            }
            Poll::Pending
        }
    }
}