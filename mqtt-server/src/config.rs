use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::mutex::Mutex;
use heapless::String;
use crate::bitset::BitSet;
use crate::distributor::InnerDistributor;

pub type Topic = String<64>;
pub(crate) type SubscriberBitSet = BitSet<64>;
pub type InnerDistributorMutex<const N: usize> = Mutex<NoopRawMutex, InnerDistributor<N>>;
pub(crate) const QUEUE_LEN: usize = 1;
pub(crate) const TREE_SIZE: usize = 64;