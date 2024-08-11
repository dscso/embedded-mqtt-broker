# Embedded Rust MQTT broker

This repository contains a working MQTT broker. It supports
- publishing
- subscribing
- will

retained messages are not supportet for the moment due to memory constrains. It has been tested on an ESP32 and an STM32F767ZI

## Run on ESP32

1. Install the ESP32 build toolchain as described [here](https://docs.esp-rs.org/book/installation/riscv-and-xtensa.html)
2. Install espflash

```bash
cargo install espup
espup install
cargo install espflash
```
To flash the program, please set the environment variables `SSID` and `PASSWORD`.

```bash
PASSWORD="<password>" SSID="<ssid>" cargo run --release
```

Now the programm should get flashed to the ESP32

## Run on STM32F767ZI

Install the flashing software for STM32
```bash
cargo install probe-rs-tools --locked
```

to run the code go to `stm32` and execute the following command: 

```bash
cargo run --release
```
