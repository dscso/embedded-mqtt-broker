use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use async_broadcast::{broadcast, Receiver, Sender};
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::mutex::Mutex;
use esp_println::println;

pub type Msg = Vec<u8>;

struct InnerDistributor {
    topics: BTreeMap<String, Sender<Msg>>,
}
pub struct Distributor {
    inner: Mutex<NoopRawMutex, InnerDistributor>,
}

impl Distributor {
    pub async fn add_publisher(&self, name: &str) -> Sender<Msg> {
        if let Some(sender) = self.inner.lock().await.topics.get(name) {
            if !sender.is_closed() {
                return sender.clone();
            }
        }
        println!("CREATING NEW CHANNEL!!! {}", name);
        let (sender, _) = broadcast(10);
        let ret_sender = sender.clone();
        self.inner
            .lock()
            .await
            .topics
            .insert(name.to_string(), sender);
        ret_sender
    }
    pub async fn add_subscriber(&self, name: &str) -> Receiver<Msg> {
        if let Some(sender) = self.inner.lock().await.topics.get(name) {
            if !sender.is_closed() {
                return sender.new_receiver();
            }
        }
        println!("CREATING NEW CHANNEL!!! {}", name);
        let (sender, receiver) = broadcast(10);
        self.inner
            .lock()
            .await
            .topics
            .insert(name.to_string(), sender);
        receiver
    }
}

impl Default for Distributor {
    fn default() -> Self {
        Self {
            inner: Mutex::new(InnerDistributor {
                topics: BTreeMap::new(),
            }),
        }
    }
}
