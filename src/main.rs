#![no_std]
#![no_main]
#![feature(type_alias_impl_trait)]

mod distributor;
mod socket;
mod sta;

use crate::distributor::Distributor;
use embassy_executor::Spawner;
use embassy_net::{Config, Stack, StackResources};
use embassy_time::{Duration, Timer};
use embedded_alloc::Heap;
use esp_backtrace as _;
use esp_hal as hal;
use esp_println::{print, println};
use esp_wifi::wifi::{WifiDevice, WifiStaDevice};
use esp_wifi::{initialize, EspWifiInitFor};
use hal::clock::ClockControl;
use hal::rng::Rng;
use hal::{embassy, peripherals::Peripherals, prelude::*, timer::TimerGroup};
use log::LevelFilter;
use log::{info, Level, Metadata, Record};
use static_cell::make_static;

use crate::socket::listen_task;
use crate::sta::connection;

const SSID: &str = env!("SSID");
const PASSWORD: &str = env!("PASSWORD");
const MAX_CONNECTIONS: usize = 4;
const HEAP_SIZE: usize = 1024 * 4;

struct SimpleLogger;
#[global_allocator]
static HEAP: Heap = Heap::empty();

extern crate alloc;

impl log::Log for SimpleLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= Level::Info
    }

    fn log(&self, record: &Record) {
        if self.enabled(record.metadata()) {
            print!("{} ", record.level());
            if let Some(file) = record.file() {
                print!("{}", file);
            }
            if let Some(line) = record.line() {
                print!(":{} ", line);
            }
            println!("{}", record.args());
        }
    }

    fn flush(&self) {}
}

static LOGGER: SimpleLogger = SimpleLogger;

#[main]
async fn main(spawner: Spawner) -> ! {
    #[cfg(feature = "log")]
    esp_println::logger::init_logger(log::LevelFilter::Info);
    log::set_logger(&LOGGER)
        .map(|()| log::set_max_level(LevelFilter::Info))
        .unwrap();
    // initialize heap
    {
        use core::mem::MaybeUninit;
        static mut HEAP_MEM: [MaybeUninit<u8>; HEAP_SIZE] = [MaybeUninit::uninit(); HEAP_SIZE];
        unsafe { HEAP.init(HEAP_MEM.as_ptr() as usize, HEAP_SIZE) }
    }
    let peripherals = Peripherals::take();

    let system = peripherals.SYSTEM.split();
    let clocks = ClockControl::max(system.clock_control).freeze();
    let mut rng = Rng::new(peripherals.RNG);

    let (seed_hi, seed_lo) = (rng.random(), rng.random());
    let seed = (seed_hi as u64) << 32 | seed_lo as u64;
    info!("Seed: {:x}", seed);

    #[cfg(target_arch = "xtensa")]
    let timer = hal::timer::TimerGroup::new(peripherals.TIMG1, &clocks, None).timer0;
    #[cfg(target_arch = "riscv32")]
    let timer = hal::systimer::SystemTimer::new(peripherals.SYSTIMER).alarm0;
    let init = initialize(
        EspWifiInitFor::Wifi,
        timer,
        rng,
        system.radio_clock_control,
        &clocks,
    )
    .unwrap();

    let wifi = peripherals.WIFI;
    let (wifi_interface, controller) =
        esp_wifi::wifi::new_with_mode(&init, wifi, WifiStaDevice).unwrap();

    let timer_group0 = TimerGroup::new_async(peripherals.TIMG0, &clocks);

    embassy::init(&clocks, timer_group0);

    let config = Config::dhcpv4(Default::default());

    // Init network stack
    let stack = &*make_static!(Stack::new(
        wifi_interface,
        config,
        make_static!(StackResources::<{ 5 + MAX_CONNECTIONS }>::new()),
        seed
    ));

    spawner.spawn(watchdog()).ok();
    spawner.spawn(connection(controller)).ok();
    spawner.spawn(net_task(&stack)).ok();

    loop {
        if stack.is_link_up() {
            break;
        }
        Timer::after(Duration::from_millis(500)).await;
    }

    info!("Waiting to get IP address...");
    loop {
        if let Some(config) = stack.config_v4() {
            info!("Got IP: {}", config.address);
            break;
        }
        Timer::after(Duration::from_millis(500)).await;
    }

    let distributor = make_static!(Distributor::default());

    // spawn listeners for concurrent connections
    for i in 0..MAX_CONNECTIONS {
        spawner
            .spawn(listen_task(&stack, i, 8080, distributor))
            .ok();
    }

    loop {
        Timer::after(Duration::from_millis(1000)).await;
    }
}

#[embassy_executor::task]
async fn net_task(stack: &'static Stack<WifiDevice<'static, WifiStaDevice>>) {
    stack.run().await
}

#[embassy_executor::task]
async fn watchdog() {
    loop {
        println!("heap: {}", HEAP.used());

        Timer::after(Duration::from_secs(10)).await;
    }
}
