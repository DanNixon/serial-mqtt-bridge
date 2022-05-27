mod config;

use crate::config::Config;
use anyhow::{anyhow, Result};
use clap::Parser;
use paho_mqtt::{Client, ConnectOptionsBuilder, CreateOptionsBuilder, Message};
use std::{env, path::PathBuf, thread, time::Duration};

/// A bridge between a serial port and MQTT
#[derive(Clone, Debug, Parser)]
#[clap(author, version, about, long_about = None)]
struct Cli {
    /// Configuration file
    config: PathBuf,
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let args = Cli::parse();
    let config = Config::load(&args.config)?;
    log::debug!("Config: {:#?}", config);

    let client = Client::new(
        CreateOptionsBuilder::new()
            .server_uri(&config.broker.address)
            .client_id(&config.broker.client_id)
            .persistence(env::temp_dir())
            .finalize(),
    )?;

    let lwt = Message::new(&config.topics.availability, "offline", 1);

    client.connect(
        ConnectOptionsBuilder::new()
            .user_name(&config.broker.username)
            .password(&config.broker.password)
            .will_message(lwt.clone())
            .finalize(),
    )?;

    let rx = client.start_consuming();

    client.subscribe(&config.topics.transmit, 2)?;
    client.subscribe(&config.topics.receive_control, 2)?;

    let ctrlc_client = client.clone();
    ctrlc::set_handler(move || {
        ctrlc_client.stop_consuming();
    })?;

    client.publish(Message::new(&config.topics.availability, "online", 1))?;
    log::info!("Connected to MQTT broker");

    log::debug!("Opening serial port with config: {:?}", config.serial);
    let mut port = serialport::new(config.serial.device, config.serial.baud)
        .timeout(config.serial.timeout)
        .open()?;

    for msg in rx.iter() {
        if let Some(msg) = msg {
            if msg.topic() == config.topics.transmit {
                log::info!("Tx message: {:?}", msg);
                if let Err(e) = port.write(msg.payload()) {
                    log::error!("Failed to write to serial port: {}", e);
                }
            } else if msg.topic() == config.topics.receive_control {
                log::info!("Rx control message: {:?}", msg);
                match msg.payload_str().parse::<usize>() {
                    Ok(req_bytes) => {
                        log::info!("Requested read of {} bytes", req_bytes);
                        let mut buffer: Vec<u8> = vec![0; req_bytes];
                        match port.read(buffer.as_mut_slice()) {
                            Ok(rx_bytes) => {
                                log::info!("Received {} bytes from serial port", rx_bytes);
                                if let Err(e) = client.publish(Message::new(
                                    &config.topics.receive,
                                    &buffer[..rx_bytes],
                                    1,
                                )) {
                                    log::error!("Failed publish received bytes via MQTT: {}", e);
                                }
                            }
                            Err(e) => {
                                log::error!("Failed to read from serial port: {}", e);
                            }
                        }
                    }
                    Err(e) => {
                        log::error!("Failed to parse requested receive byte count: {}", e);
                    }
                }
            }
        } else if !client.is_connected() {
            if let Err(e) = try_reconnect(&client) {
                log::error!("{}", e);
                break;
            }
        }
    }

    if client.is_connected() {
        client.publish(lwt)?;

        log::info!("Disconnecting from MQTT broker");
        client.disconnect(None)?;
    }

    Ok(())
}

fn try_reconnect(c: &Client) -> Result<()> {
    for i in 0..300 {
        log::info!("Attempting reconnection {}...", i);
        match c.reconnect() {
            Ok(_) => {
                log::info!("Reconnection successful");
                return Ok(());
            }
            Err(e) => {
                log::error!("Reconnection failed: {}", e);
            }
        }
        thread::sleep(Duration::from_secs(1));
    }
    Err(anyhow!("Failed to reconnect to broker"))
}
