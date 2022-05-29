mod config;

use crate::config::Config;
use anyhow::Result;
use clap::Parser;
use futures::executor::block_on;
use paho_mqtt::{
    AsyncClient, ConnectOptionsBuilder, CreateOptionsBuilder, Message, PersistenceType,
};
use std::{path::PathBuf, time::Duration};

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

    block_on(async {
        let client = AsyncClient::new(
            CreateOptionsBuilder::new()
                .server_uri(&config.broker.address)
                .client_id(&config.broker.client_id)
                .persistence(PersistenceType::None)
                .finalize(),
        )?;

        let transmit_topic = config.topics.transmit.clone();
        let receive_control_topic = config.topics.receive_control.clone();
        client.set_connected_callback(move |c| {
            log::info!("Connected to MQTT broker");
            c.subscribe(&transmit_topic, 2);
            c.subscribe(&receive_control_topic, 2);
        });

        let lwt = Message::new(&config.topics.availability, "offline", 1);

        let response = client
            .connect(
                ConnectOptionsBuilder::new()
                    .clean_session(true)
                    .automatic_reconnect(Duration::from_secs(1), Duration::from_secs(5))
                    .user_name(&config.broker.username)
                    .password(&config.broker.password)
                    .will_message(lwt.clone())
                    .finalize(),
            )
            .await?;

        log::info!(
            "Using MQTT version {}",
            response.connect_response().unwrap().mqtt_version
        );

        let rx = client.start_consuming();

        let ctrlc_client = client.clone();
        ctrlc::set_handler(move || {
            ctrlc_client.stop_consuming();
        })?;

        client
            .publish(Message::new(&config.topics.availability, "online", 1))
            .await?;

        log::debug!("Opening serial port with config: {:?}", config.serial);
        let mut port = serialport::new(config.serial.device, config.serial.baud)
            .timeout(config.serial.timeout)
            .open()?;

        for msg in rx.iter().flatten() {
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
                                if let Err(e) = client
                                    .publish(Message::new(
                                        &config.topics.receive,
                                        &buffer[..rx_bytes],
                                        1,
                                    ))
                                    .await
                                {
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
        }

        if client.is_connected() {
            client.publish(lwt).await?;

            log::info!("Disconnecting from MQTT broker");
            client.disconnect(None).await?;
        }

        Ok(())
    })
}
