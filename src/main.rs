
extern crate futures;
extern crate lapin_futures as lapin;
extern crate reqwest;
extern crate serde;
#[macro_use]
extern crate serde_derive;
#[macro_use]
extern crate serde_json;
extern crate tokio_core;

mod config;
mod emitter;
mod message;

use config::*;
use futures::future::Future;
use futures::Stream;
use lapin::types::FieldTable;
use std::{thread, time};
use std::net::ToSocketAddrs;
use tokio_core::reactor::Core;
use tokio_core::net::TcpStream;
use lapin::client::ConnectionOptions;
use lapin::channel::{BasicConsumeOptions, QueueDeclareOptions};

fn main() {
  loop {
    let amqp_hostname = get_amqp_hostname();
    let amqp_port = get_amqp_port();
    let amqp_username = get_amqp_username();
    let amqp_password = get_amqp_password();
    let amqp_vhost = get_amqp_vhost();
    let amqp_queue = get_amqp_queue();
    let amqp_completed_queue = get_amqp_completed_queue();
    let amqp_error_queue = get_amqp_error_queue();

    println!("Start connection with configuration:");
    println!("AMQP HOSTNAME: {}", amqp_hostname);
    println!("AMQP PORT: {}", amqp_port);
    println!("AMQP USERNAME: {}", amqp_username);
    println!("AMQP VHOST: {}", amqp_vhost);
    println!("AMQP QUEUE: {}", amqp_queue);

    // create the reactor
    let mut core = Core::new().unwrap();
    let handle = core.handle();
    let address = amqp_hostname.clone() + ":" + amqp_port.as_str();
    let addr = address.to_socket_addrs().unwrap().next().unwrap();
    let channel_name = amqp_queue;

    let state = core.run(

      TcpStream::connect(&addr, &handle).and_then(|stream| {
        lapin::client::Client::connect(stream, &ConnectionOptions{
          username: amqp_username,
          password: amqp_password,
          vhost: amqp_vhost,
          ..Default::default()
        })
      }).and_then(|(client, heartbeat_future_fn)| {
        let heartbeat_client = client.clone();
        handle.spawn(heartbeat_future_fn(&heartbeat_client).map_err(|_| ()));

        client.create_channel()
      }).and_then(|channel| {
        let id = channel.id;
        println!("created channel with id: {}", id);
        
        let ch = channel.clone();
        channel.queue_declare(&channel_name, &QueueDeclareOptions::default(), &FieldTable::new()).and_then(move |_| {
          println!("channel {} declared queue {}", id, channel_name);

          channel.basic_consume(&channel_name, "my_consumer", &BasicConsumeOptions::default(), &FieldTable::new())
        }).and_then(|stream| {
          stream.for_each(move |message| {
            let data = std::str::from_utf8(&message.data).unwrap();
            println!("got message: {}", data);

            match message::process(data) {
              Ok(job_id) => {
                let msg = json!({
                  "job_id": job_id,
                  "status": "completed"
                });
                emitter::publish(&amqp_completed_queue, msg.to_string());
                ch.basic_ack(message.delivery_tag);
              }
              Err(msg) => {
                let content = json!({
                  "status": "error",
                  "message": msg
                });
                emitter::publish(&amqp_error_queue, content.to_string());
                let requeue = false;
                ch.basic_reject(message.delivery_tag, requeue);
              }
            }
            Ok(())
          })
        })
      })
    );

    println!("{:?}", state);
    let sleep_duration = time::Duration::new(1, 0);
    thread::sleep(sleep_duration);
  }
}