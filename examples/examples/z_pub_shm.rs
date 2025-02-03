// Copyright (c) 2023 ZettaScale Technology

//

// This program and the accompanying materials are made available under the

// terms of the Eclipse Public License 2.0 which is available at

// http://www.eclipse.org/legal/epl-2.0, or the Apache License, Version 2.0

// which is available at https://www.apache.org/licenses/LICENSE-2.0.

//

// SPDX-License-Identifier: EPL-2.0 OR Apache-2.0

//

// Contributors:

//   ZettaScale Zenoh Team, <zenoh@zettascale.tech>

//

use clap::Parser;

use zenoh::{

    key_expr::KeyExpr,

    shm::{

        BlockOn, GarbageCollect, PosixShmProviderBackend, ShmProviderBuilder, POSIX_PROTOCOL_ID,

    },

    Config, Wait,

};

use zenoh_examples::CommonArgs;

use std::time::{SystemTime, UNIX_EPOCH};



const N: usize = 10;



#[tokio::main]

async fn main() -> zenoh::Result<()> {

    // Initiate logging

    zenoh::init_log_from_env_or("error");



    let (config, path, payload) = parse_args();



    println!("Opening session...");

    let session = zenoh::open(config).await.unwrap();



    println!("Creating POSIX SHM provider...");

    // Create an SHM backend

    let backend = PosixShmProviderBackend::builder()

        .with_size(N * 1024)

        .unwrap()

        .wait()

        .unwrap();

    // Create an SHM provider

    let provider = ShmProviderBuilder::builder()

        .protocol_id::<POSIX_PROTOCOL_ID>()

        .backend(backend)

        .wait();



    let publisher = session.declare_publisher(&path).await.unwrap();



    // Create allocation layout for series of similar allocations

    println!("Allocating Shared Memory Buffer...");

    let layout = provider.alloc(1024).into_layout().unwrap();



    println!("Press CTRL-C to quit...");



    // Initialize message ID counter

    let mut message_id: u32 = 0;



    for idx in 0..u32::MAX {

        tokio::time::sleep(std::time::Duration::from_micros(1)).await;



        // Record the timestamp just before writing to the shared memory buffer

        let timestamp = SystemTime::now()

            .duration_since(UNIX_EPOCH)

            .expect("Time went backwards")

            .as_nanos();



        // Increment the message ID

        message_id += 1;



        // Allocate a particular SHM buffer using the pre-created layout

        let mut sbuf = layout

            .alloc()

            .with_policy::<BlockOn<GarbageCollect>>()

            .await

            .unwrap();



        // Include the message ID, timestamp, and iteration index in the payload

        let prefix = format!("[{idx:4}] [ID: {message_id}] [Timestamp: {timestamp}] ");

        let prefix_len = prefix.len();

        let slice_len = prefix_len + payload.len();



        sbuf[0..prefix_len].copy_from_slice(prefix.as_bytes());

        sbuf[prefix_len..slice_len].copy_from_slice(payload.as_bytes());



        // Write the data

        println!(

            "Put SHM Data ('{}': '{}')",

            path,

            String::from_utf8_lossy(&sbuf[0..slice_len])

        );

        publisher.put(sbuf).await?;

    }



    Ok(())

}



#[derive(clap::Parser, Clone, PartialEq, Eq, Hash, Debug)]

struct Args {

    #[arg(short, long, default_value = "demo/example/zenoh-rs-pub")]

    /// The key expression to publish onto.

    key: KeyExpr<'static>,

    #[arg(short, long, default_value = "Pub from Rust SHM!")]

    /// The payload to publish.

    payload: String,

    #[command(flatten)]

    common: CommonArgs,

}



fn parse_args() -> (Config, KeyExpr<'static>, String) {

    let args = Args::parse();

    (args.common.into(), args.key, args.payload)

}
