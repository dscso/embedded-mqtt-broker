use crate::bitset::BitSet;
use crate::distributor::InnerDistributor;
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::mutex::Mutex;
use heapless::String;

/// How many messages can be queued simultaneously
/// If queue is full, all other sockets are halted until all messages are sent
/// one should be ok for most cases
pub const QUEUE_LEN: usize = 1;
/// How many subscriptions can be saved simultaneously
pub const TREE_SIZE: usize = 64;
/// How many bytes can a single message be
pub const MAX_MESSAGE_SIZE: usize = 1024;
/// How many bytes a will can be
pub const MAX_WILL_LENGTH: usize = 128;
/// Maximum length of a topic
pub const MAX_TOPIC_LENGTH: usize = 64;

pub type Topic = String<MAX_TOPIC_LENGTH>;
/// This defines how many socket connections are supported by the underlying datastructures
pub(crate) type SubscriberBitSet = BitSet;
pub type InnerDistributorMutex<const N: usize> = Mutex<NoopRawMutex, InnerDistributor<N>>;
