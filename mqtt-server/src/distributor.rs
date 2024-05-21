use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::channel::{Channel, Receiver, Sender};

pub type Msg = u8;
const MAX_MSG_CHANNEL: usize = 10;
type DistributorChannel = Channel<NoopRawMutex, Msg, MAX_MSG_CHANNEL>;
type DistributorSender<'a> = Sender<'a, NoopRawMutex, Msg, MAX_MSG_CHANNEL>;

struct InnerDistributor {
    channel: DistributorChannel,
}
pub struct Distributor {
    channel: DistributorChannel,
    //inner: Mutex<NoopRawMutex, InnerDistributor>,
}

impl<'a> Distributor {
    pub async fn new() -> Self {
        let channel = Channel::new();
        Self {
            channel
        //    inner
        }
    }
    pub fn publish(&self, msg: Msg) {
        self.channel.try_send(msg).unwrap();
    }
}